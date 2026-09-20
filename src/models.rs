//! Optional model backends. Configuration is resolved by store::load_config,
//! which keeps endpoints, credential selectors and model paths global-only.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, path::PathBuf};

pub fn request_json(config: &Value, route: &str, body: &Value) -> Result<Value> {
    let base = config["api_base"]
        .as_str()
        .unwrap_or("https://api.openai.com/v1")
        .trim_end_matches('/');
    let env = config["api_key_env"].as_str().unwrap_or("OPENAI_API_KEY");
    let key = std::env::var(env)
        .with_context(|| format!("Credential environment variable {env} is not set"))?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(
            if route == "chat/completions" { 30 } else { 10 },
        )))
        .max_redirects(0)
        .build()
        .into();
    // Do not propagate HTTP errors containing URLs, headers or provider bodies.
    agent
        .post(format!("{base}/{route}"))
        .header("Authorization", format!("Bearer {key}"))
        .send_json(body)
        .map_err(|_| anyhow::anyhow!("Model HTTP request failed"))?
        .body_mut()
        .read_json()
        .map_err(|_| {
            if route == "embeddings" {
                InvalidEmbedding.into()
            } else {
                anyhow::anyhow!("Invalid model JSON response")
            }
        })
}
fn model_path(c: &Value) -> PathBuf {
    let configured = c["onnx_path"].as_str().unwrap_or("");
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    if configured.is_empty() {
        home.join(".cache/mnemosyne/models")
            .join(c["model"].as_str().unwrap_or("").replace('/', "--"))
            .join("model.onnx")
    } else if let Some(rest) = configured.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(configured)
    }
}
pub fn fingerprint(c: &Value) -> Result<String> {
    let mut hash = Sha256::new();
    let mut safe = c.clone();
    if let Some(o) = safe.as_object_mut() {
        o.remove("api_key_env");
        o.remove("batch_size");
    }
    hash.update(b"mnemosyne-input-v1-wordpiece-cls-64");
    hash.update(serde_json::to_vec(&safe)?);
    if c["backend"].as_str().unwrap_or("onnx") == "onnx" {
        // Hash only the explicitly selected model and its vocabulary; no download.
        use std::io::Read;
        for path in [model_path(c), model_path(c).with_file_name("vocab.txt")] {
            let mut file =
                std::fs::File::open(&path).context("Local model or vocabulary missing")?;
            let mut buf = [0u8; 65536];
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hash.update(&buf[..n]);
            }
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}
/// One immutable inference/cache identity. Remote alias changes require an explicit revision.
#[derive(Clone, Debug)]
pub struct ModelProfile {
    settings: Value,
    pub fingerprint: String,
    pub dimensions: usize,
}

impl ModelProfile {
    pub fn new(settings: &Value) -> Result<Self> {
        Ok(Self {
            settings: settings.clone(),
            fingerprint: fingerprint(settings)?,
            dimensions: settings["dimensions"].as_u64().unwrap_or(512) as usize,
        })
    }

    pub fn settings(&self) -> &Value {
        &self.settings
    }

    pub fn matches(&self, settings: &Value) -> bool {
        settings["enabled"].as_bool().unwrap_or(false)
            && fingerprint(settings).is_ok_and(|value| value == self.fingerprint)
    }
}

#[derive(Debug)]
pub struct InvalidEmbedding;
impl std::fmt::Display for InvalidEmbedding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Invalid embedding response dimensions, values, or indices")
    }
}
impl std::error::Error for InvalidEmbedding {}

pub fn valid_vector(vector: &[f64], dimensions: usize) -> bool {
    vector.len() == dimensions
        && vector.iter().all(|value| value.is_finite())
        && vector.iter().any(|value| *value != 0.0)
}

fn response_vectors(response: &Value, count: usize) -> Result<Vec<Vec<f64>>> {
    let rows = response["data"]
        .as_array()
        .context("Missing embedding data")?;
    ensure!(rows.len() == count, "Embedding count mismatch");
    let mut ordered = vec![None; count];
    for (position, row) in rows.iter().enumerate() {
        let index = match row.get("index") {
            Some(value) => value.as_u64().context("Invalid embedding response index")? as usize,
            None => position,
        };
        ensure!(
            index < ordered.len() && ordered[index].is_none(),
            "Invalid embedding response index"
        );
        ordered[index] = Some(
            row["embedding"]
                .as_array()
                .context("Missing vector")?
                .iter()
                .map(|v| v.as_f64().context("Invalid embedding value"))
                .collect::<Result<Vec<_>>>()?,
        );
    }
    ordered
        .into_iter()
        .map(|v| v.context("Missing embedding"))
        .collect()
}

pub fn embed(c: &Value, texts: &[String]) -> Result<Vec<Vec<f64>>> {
    if texts.is_empty() {
        return Ok(vec![]);
    }
    let vectors = match c["backend"].as_str().unwrap_or("onnx") {
        "openai" | "openai-compatible" => {
            let response =
                request_json(c, "embeddings", &json!({"model":c["model"],"input":texts}))?;
            response_vectors(&response, texts.len()).map_err(|_| InvalidEmbedding)?
        }
        "onnx" => local_inference(c, texts, None)?,
        other => bail!("Unknown embedding backend: {other}"),
    };
    let dim = c["dimensions"].as_u64().unwrap_or(512) as usize;
    if vectors.len() != texts.len() || !vectors.iter().all(|v| valid_vector(v, dim)) {
        return Err(InvalidEmbedding.into());
    }
    Ok(vectors)
}
pub fn rerank(c: &Value, query: &str, docs: &[String]) -> Result<Vec<f64>> {
    if docs.is_empty() {
        return Ok(vec![]);
    }
    let rows = local_inference(c, docs, Some(query))?;
    rows.into_iter()
        .map(|v| {
            v.first()
                .copied()
                .filter(|v| v.is_finite())
                .context("Invalid reranker output")
        })
        .collect()
}

pub fn wordpiece_ids(text: &str, vocab: &HashMap<String, i64>) -> Vec<i64> {
    let re = regex::Regex::new(r"[\u{4e00}-\u{9fff}]|[a-z0-9_]+|[^\w\s]").unwrap();
    let lower = text.to_lowercase();
    let mut ids = Vec::new();
    for token in re.find_iter(&lower).map(|m| m.as_str()) {
        if let Some(id) = vocab.get(token) {
            ids.push(*id);
            continue;
        }
        let chars: Vec<_> = token.chars().collect();
        let mut start = 0;
        let mut pieces = Vec::new();
        while start < chars.len() {
            let mut found = None;
            for end in (start + 1..=chars.len()).rev() {
                let part = format!(
                    "{}{}",
                    if start == 0 { "" } else { "##" },
                    chars[start..end].iter().collect::<String>()
                );
                if let Some(id) = vocab.get(&part) {
                    found = Some((end, *id));
                    break;
                }
            }
            if let Some((end, id)) = found {
                pieces.push(id);
                start = end;
            } else {
                pieces = vec![*vocab.get("[UNK]").unwrap_or(&100)];
                break;
            }
        }
        ids.extend(pieces);
    }
    ids
}

#[cfg(not(feature = "onnx"))]
fn local_inference(_c: &Value, _texts: &[String], _query: Option<&str>) -> Result<Vec<Vec<f64>>> {
    bail!("Local ONNX requires a build with --features onnx and ONNX Runtime; no Python fallback")
}

#[cfg(feature = "onnx")]
fn local_inference(c: &Value, texts: &[String], query: Option<&str>) -> Result<Vec<Vec<f64>>> {
    use ort::{session::Session, value::Tensor};
    if std::env::var_os("ORT_DYLIB_PATH").is_none() {
        let runtime = std::env::current_exe()?.with_file_name(if cfg!(target_os = "macos") {
            "libonnxruntime.dylib"
        } else if cfg!(target_os = "windows") {
            "onnxruntime.dll"
        } else {
            "libonnxruntime.so"
        });
        if runtime.is_file() {
            ort::init_from(runtime)?.commit();
        }
    }
    let path = model_path(c);
    let vocab: HashMap<String, i64> = std::fs::read_to_string(path.with_file_name("vocab.txt"))?
        .lines()
        .enumerate()
        .map(|(i, s)| (s.to_owned(), i as i64))
        .collect();
    let mut session = Session::builder()?.commit_from_file(path)?;
    let length = if query.is_some() { 256 } else { 64 };
    let mut ids = Vec::new();
    let mut attention = Vec::new();
    let mut types = Vec::new();
    for text in texts {
        let cls = *vocab.get("[CLS]").unwrap_or(&101);
        let sep = *vocab.get("[SEP]").unwrap_or(&102);
        let pad = *vocab.get("[PAD]").unwrap_or(&0);
        let mut row = vec![cls];
        let mut segments = vec![0i64];
        if let Some(q) = query {
            row.extend(wordpiece_ids(q, &vocab).into_iter().take(64));
            row.push(sep);
            segments.resize(row.len(), 0);
        }
        row.extend(
            wordpiece_ids(text, &vocab)
                .into_iter()
                .take(length - row.len() - 1),
        );
        row.push(sep);
        segments.resize(row.len(), if query.is_some() { 1 } else { 0 });
        let mut mask = vec![1i64; row.len()];
        row.resize(length, pad);
        segments.resize(length, 0);
        mask.resize(length, 0);
        ids.extend(row);
        attention.extend(mask);
        types.extend(segments);
    }
    let shape = [texts.len(), length];
    let ids = Tensor::from_array((shape, ids))?;
    let mask = Tensor::from_array((shape, attention))?;
    let has_types = session
        .inputs()
        .iter()
        .any(|i| i.name() == "token_type_ids");
    let output = if has_types {
        session.run(ort::inputs!["input_ids"=>ids,"attention_mask"=>mask,"token_type_ids"=>Tensor::from_array((shape,types))?])?
    } else {
        session.run(ort::inputs!["input_ids"=>ids,"attention_mask"=>mask])?
    };
    let (shape, data) = output[0].try_extract_tensor::<f32>()?;
    ensure!(
        shape.len() == 2 || shape.len() == 3,
        "Unsupported model output shape"
    );
    ensure!(shape[0] as usize == texts.len(), "Model batch mismatch");
    let width = *shape.last().unwrap() as usize;
    let stride = if shape.len() == 3 {
        shape[1] as usize * width
    } else {
        width
    };
    let mut result = Vec::new();
    for i in 0..texts.len() {
        let mut row: Vec<f64> = data[i * stride..i * stride + width]
            .iter()
            .map(|x| *x as f64)
            .collect();
        if query.is_none() {
            let norm = row.iter().map(|x| x * x).sum::<f64>().sqrt();
            if norm > 0.0 {
                for x in &mut row {
                    *x /= norm;
                }
            }
        }
        result.push(row);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn http_vectors_reorder_and_reject_invalid_indices_and_values() {
        let rows = json!({"data":[{"index":1,"embedding":[0,1]}, {"index":0,"embedding":[1,0]}]});
        assert_eq!(
            response_vectors(&rows, 2).unwrap(),
            vec![vec![1.0, 0.0], vec![0.0, 1.0]]
        );
        for indices in [
            json!([0, 0]),
            json!([0, 2]),
            json!([-1, 1]),
            json!(["0", 1]),
        ] {
            let rows = json!({"data":[{"index":indices[0],"embedding":[1,0]}, {"index":indices[1],"embedding":[0,1]}]});
            assert!(response_vectors(&rows, 2).is_err());
        }
        assert!(response_vectors(&json!({"data":[{"embedding":[null,1]}]}), 1).is_err());
        for vector in [
            vec![0.0, 0.0],
            vec![1.0],
            vec![f64::NAN, 1.0],
            vec![f64::INFINITY, 1.0],
        ] {
            assert!(!valid_vector(&vector, 2));
        }
    }

    #[test]
    fn explicit_revision_and_same_size_local_asset_changes_invalidate_profile() {
        let settings = json!({"enabled":true,"backend":"openai","model":"alias","dimensions":2});
        let profile = ModelProfile::new(&settings).unwrap();
        let mut changed = settings.clone();
        changed["revision"] = json!("2026-09-19");
        assert!(!profile.matches(&changed));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.onnx");
        std::fs::write(&path, "aaaa").unwrap();
        std::fs::write(path.with_file_name("vocab.txt"), "bbbb").unwrap();
        let settings = json!({"enabled":true,"backend":"onnx","onnx_path":path,"dimensions":2});
        let profile = ModelProfile::new(&settings).unwrap();
        std::fs::write(&path, "cccc").unwrap();
        assert!(!profile.matches(&settings));
    }

    #[test]
    fn wordpiece_preserves_chinese_and_subwords() {
        let vocab = HashMap::from([
            ("认".into(), 4),
            ("证".into(), 5),
            ("api".into(), 6),
            ("##s".into(), 7),
            ("[UNK]".into(), 1),
        ]);
        assert_eq!(
            wordpiece_ids("认证 apis missing", &vocab),
            vec![4, 5, 6, 7, 1]
        );
    }
}
