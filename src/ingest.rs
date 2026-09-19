use crate::{
    api::{WriteRequest, classify_entry, write_entry},
    store::{Store, find_project_store, global_store, load_config},
};
use anyhow::{Result, bail};
use fs2::FileExt;
use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::Write,
};

#[derive(Clone, Debug)]
struct Finding {
    kind: String,
    importance: i64,
    title: String,
    tags: Vec<String>,
    content: String,
    evidence: String,
}

#[derive(Clone, Debug, PartialEq)]
struct Turn {
    role: String,
    text: String,
}

fn destination() -> Store {
    find_project_store().unwrap_or_else(global_store)
}

fn allowed(config: &Value) -> HashSet<String> {
    config["memory"]["types"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn item(value: &Value, types: &HashSet<String>) -> Option<Finding> {
    let kind = value.get("type")?.as_str()?.trim();
    if !types.contains(kind) {
        return None;
    }
    let title = value
        .get("title")?
        .as_str()?
        .trim()
        .chars()
        .take(80)
        .collect::<String>();
    let content = value.get("content")?.as_str()?.trim().to_owned();
    if title.is_empty() || content.is_empty() {
        return None;
    }
    let importance = value
        .get("importance")
        .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
        .unwrap_or(50)
        .clamp(0, 100);
    let tags = value
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .collect();
    let evidence = value
        .get("evidence")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .chars()
        .take(200)
        .collect();
    Some(Finding {
        kind: kind.into(),
        importance,
        title,
        tags,
        content,
        evidence,
    })
}

fn json_findings(text: &str, types: &HashSet<String>) -> Vec<Finding> {
    let Ok(data) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else {
        return vec![];
    };
    let items = data.get("findings").unwrap_or(&data);
    items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| item(v, types))
        .collect()
}

fn markdown_findings(text: &str, types: &HashSet<String>) -> Vec<Finding> {
    let mut active = false;
    let mut current = serde_json::Map::new();
    let mut content = Vec::new();
    let mut indent = None;
    let mut in_content = false;
    let mut output = Vec::new();
    let flush = |current: &mut serde_json::Map<String, Value>,
                 content: &mut Vec<String>,
                 output: &mut Vec<Finding>| {
        if !content.is_empty() {
            current.insert("content".into(), json!(content.join("\n").trim_end()));
        }
        if let Some(finding) = item(&Value::Object(current.clone()), types) {
            output.push(finding);
        }
        current.clear();
        content.clear();
    };
    for line in text.trim_start_matches('\u{feff}').lines() {
        let trimmed = line.trim();
        if trimmed == "**Findings:**"
            || trimmed == "**Findings：**"
            || trimmed == "**新发现:**"
            || trimmed == "**新发现：**"
        {
            if active {
                flush(&mut current, &mut content, &mut output);
            }
            active = true;
            in_content = false;
            indent = None;
            continue;
        }
        if !active {
            continue;
        }
        if in_content {
            let spaces = line.len() - line.trim_start_matches(' ').len();
            if trimmed.is_empty() {
                content.push(String::new());
                continue;
            }
            if spaces > 0 && indent.is_none() {
                indent = Some(spaces);
            }
            if let Some(width) = indent
                && spaces >= width
            {
                content.push(line[width..].to_owned());
                continue;
            }
            in_content = false;
        }
        if trimmed.starts_with("**") {
            flush(&mut current, &mut content, &mut output);
            break;
        }
        let Some(field) = trimmed.strip_prefix('-') else {
            continue;
        };
        let Some((key, value)) = field.trim_start().split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if key == "type" && !current.is_empty() {
            flush(&mut current, &mut content, &mut output);
        }
        match key {
            "content" if value == "|" => {
                in_content = true;
                indent = None;
            }
            "tags" => {
                current.insert(
                    key.into(),
                    json!(
                        value
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                    ),
                );
            }
            "type" | "importance" | "title" | "evidence" => {
                current.insert(key.into(), json!(value.trim_matches(['\'', '"'])));
            }
            _ => (),
        }
    }
    if active {
        flush(&mut current, &mut content, &mut output);
    }
    output
}

fn parse_findings(text: &str, fmt: &str, types: &HashSet<String>) -> Result<Vec<Finding>> {
    match fmt {
        "json" => Ok(json_findings(text, types)),
        "markdown" => Ok(markdown_findings(text, types)),
        "auto" => {
            let trimmed = text.trim_start_matches('\u{feff}').trim_start();
            if (trimmed.starts_with('{') || trimmed.starts_with('['))
                && serde_json::from_str::<Value>(trimmed).is_ok()
            {
                return Ok(json_findings(trimmed, types));
            }
            Ok(markdown_findings(text, types))
        }
        _ => bail!("unknown findings format: {fmt}"),
    }
}

fn process(findings: Vec<Finding>, source: &str, commit: bool) -> Result<Vec<Value>> {
    let store = destination();
    let mut actions = Vec::new();
    for finding in findings {
        let preview = finding.content.chars().take(120).collect::<String>();
        let request = WriteRequest {
            memory_type: finding.kind.clone(),
            importance: finding.importance,
            title: finding.title.clone(),
            content: finding.content,
            tags: finding.tags.clone(),
            source: source.into(),
            evidence: finding.evidence,
            ..Default::default()
        };
        let written = if commit {
            write_entry(&store, &request)?
        } else {
            classify_entry(&store, &request)?
        };
        let mut result = json!({"type":finding.kind,"importance":finding.importance,"title":finding.title,
            "tags":finding.tags,"content_preview":preview,
            "verdict":if written.status == "duplicate" { "duplicate" } else { "new" },
            "target":written.duplicate_of});
        if commit {
            result["id"] = json!(written.id);
        }
        actions.push(result);
    }
    Ok(actions)
}

pub fn ingest(text: &str, source: &str, commit: bool, fmt: &str) -> Result<Vec<Value>> {
    let config = load_config(Some(&destination()))?;
    process(
        parse_findings(text, fmt, &allowed(&config))?,
        source,
        commit,
    )
}

fn block_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.trim().into();
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| {
            matches!(
                b["type"].as_str(),
                Some("text" | "input_text" | "output_text")
            )
        })
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .into()
}

fn parse_transcript(raw: &str, fmt: &str) -> Result<Vec<Turn>> {
    let fmt = if fmt == "auto" {
        detect_format(raw)
    } else {
        fmt
    };
    if fmt == "text" {
        let mut turns = Vec::new();
        let mut current: Option<Turn> = None;
        for line in raw.lines() {
            if let Some(text) = line.strip_prefix("[user] ") {
                if let Some(turn) = current.take() {
                    turns.push(turn);
                }
                current = Some(Turn {
                    role: "user".into(),
                    text: text.into(),
                });
            } else if let Some(text) = line.strip_prefix("[assistant] ") {
                if let Some(turn) = current.take() {
                    turns.push(turn);
                }
                current = Some(Turn {
                    role: "assistant".into(),
                    text: text.into(),
                });
            } else if let Some(turn) = &mut current {
                turn.text.push('\n');
                turn.text.push_str(line);
            }
        }
        if let Some(turn) = current {
            turns.push(turn);
        }
        turns.retain(|t| !t.text.trim().is_empty());
        if turns.is_empty() && !raw.trim().is_empty() {
            turns.push(Turn {
                role: "assistant".into(),
                text: raw.trim().into(),
            });
        }
        return Ok(turns);
    }
    if !matches!(
        fmt,
        "claude-jsonl" | "codex-jsonl" | "grok-jsonl" | "role-jsonl"
    ) {
        bail!("unknown transcript format: {fmt}");
    }
    let mut turns = Vec::new();
    for line in raw.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let (role, content) = match fmt {
            "claude-jsonl" => (&record["message"]["role"], &record["message"]["content"]),
            "codex-jsonl" => {
                let payload = &record["payload"];
                if record["type"] != "response_item" || payload["type"] != "message" {
                    continue;
                }
                let role = &payload["role"];
                if role == "assistant"
                    && (!payload["channel"].is_null()
                        && payload["channel"] != "final"
                        && payload["channel"] != "commentary"
                        || !payload["recipient"].is_null() && payload["recipient"] != "all")
                {
                    continue;
                }
                (role, &payload["content"])
            }
            "grok-jsonl" => (&record["type"], &record["content"]),
            "role-jsonl" => (&record["role"], &record["text"]),
            _ => unreachable!(),
        };
        let Some(role) = role.as_str().filter(|r| *r == "user" || *r == "assistant") else {
            continue;
        };
        let text = if fmt == "role-jsonl" {
            content.as_str().unwrap_or("").trim().to_owned()
        } else {
            block_text(content)
        };
        if !text.is_empty() {
            turns.push(Turn {
                role: role.into(),
                text,
            });
        }
    }
    Ok(turns)
}

pub(crate) fn detect_format(raw: &str) -> &'static str {
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            return "text";
        };
        if record["message"].is_object() {
            return "claude-jsonl";
        }
        if record["type"] == "response_item" && record["payload"]["type"] == "message" {
            return "codex-jsonl";
        }
        if matches!(record["type"].as_str(), Some("user" | "assistant"))
            && !record["content"].is_null()
        {
            return "grok-jsonl";
        }
        if !record["role"].is_null() && !record["text"].is_null() {
            return "role-jsonl";
        }
    }
    "text"
}

fn has(text: &str, pattern: &str) -> bool {
    Regex::new(pattern).unwrap().is_match(text)
}

fn heuristic(turns: &[Turn], config: &Value) -> Vec<Finding> {
    let threshold = config["distill"]["confidence_threshold"]
        .as_f64()
        .unwrap_or(0.6);
    let max = config["distill"]["max_findings_per_session"]
        .as_u64()
        .unwrap_or(5) as usize;
    let error_re = Regex::new(r"(?i)(错误|报错|Traceback|Exception|Error|failed|失败)").unwrap();
    let fix_re = Regex::new(r"(?i)(根因|原因是|修复|改成|改为|fixed|the fix|因为)").unwrap();
    let mut out = Vec::new();
    for turn in turns {
        let text = turn.text.trim();
        let kind = if turn.role == "user"
            && text.chars().count() <= 200
            && threshold <= 0.6
            && has(
                text,
                r"(?i)(不要用|别用|不用|改用|下次用|下次记得|don't use|use .* instead)",
            ) {
            Some(("preference", 65))
        } else if turn.role == "assistant"
            && text.chars().count() <= 280
            && threshold <= 0.7
            && !has(
                text,
                r"(第[一二三四五六七八九十百\d]+步|阶段\s*[一二三四五六七八九十\d]+|##\s*阶段|Step\s*\d)",
            )
        {
            let error = error_re.find(text);
            let fix = fix_re.find(text);
            if error.zip(fix).is_some_and(|(a, b)| {
                let (lo, hi) = if a.start() < b.start() {
                    (a.start(), b.start())
                } else {
                    (b.start(), a.start())
                };
                text[lo..hi].chars().count() <= 50
            }) {
                Some(("pitfall", 75))
            } else {
                None
            }
        } else {
            None
        };
        if let Some((kind, importance)) = kind {
            out.push(Finding {
                kind: kind.into(),
                importance,
                title: text.chars().take(60).collect(),
                tags: vec!["auto".into(), kind.into()],
                content: text.chars().take(1000).collect(),
                evidence: String::new(),
            });
            if out.len() >= max {
                break;
            }
        }
    }
    out
}

fn llm_findings(turns: &[Turn], config: &Value, types: &HashSet<String>) -> Result<Vec<Finding>> {
    let llm = &config["distill"]["llm"];
    let backend = llm["backend"].as_str().unwrap_or("openai");
    if !matches!(backend, "openai" | "openai-compatible") {
        bail!("Unsupported LLM extraction backend: {backend}");
    }
    let mut names: Vec<_> = types.iter().cloned().collect();
    names.sort();
    let mut prompt = format!(
        "You extract durable memories from a developer/agent conversation. Return ONLY a JSON array. Each element: {{\"type\": one of {}, \"importance\": 50-90, \"title\": <=80 chars, \"tags\": [..], \"content\": \"...\", \"evidence\": short verbatim quote from the conversation supporting this memory}}. Only include reusable facts (pitfalls, decisions, preferences, codebase, handoff). Empty array if nothing is worth saving.",
        names.join("|")
    );
    if config["distill"]["session_summary"] == true {
        prompt.push_str(" Additionally, include exactly one element with type \"session_summary\": 2-4 sentences on what was worked on and the outcome, importance 55.");
    }
    prompt.push_str("\n\nCONVERSATION:\n");
    prompt.push_str(&serde_json::to_string(
        &turns
            .iter()
            .map(|t| json!({"role":t.role,"text":t.text}))
            .collect::<Vec<_>>(),
    )?);
    let response = crate::models::request_json(
        llm,
        "chat/completions",
        &json!({
            "model":llm["model"].as_str().filter(|s| !s.is_empty()).unwrap_or("gpt-4o-mini"),
            "messages":[{"role":"user","content":prompt}],"temperature":0
        }),
    )
    .map_err(|e| anyhow::anyhow!("LLM extraction failed: {e}"))?;
    let content = response["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("LLM extraction did not return message content"))?;
    let max = config["distill"]["max_findings_per_session"]
        .as_u64()
        .unwrap_or(5) as usize;
    parse_llm_payload(content, types, max)
}

fn parse_llm_payload(content: &str, types: &HashSet<String>, max: usize) -> Result<Vec<Finding>> {
    let start = content
        .find('[')
        .ok_or_else(|| anyhow::anyhow!("LLM extraction did not return a JSON array"))?;
    let end = content
        .rfind(']')
        .ok_or_else(|| anyhow::anyhow!("LLM extraction did not return a JSON array"))?;
    if start > end {
        bail!("LLM extraction did not return a JSON array");
    }
    let parsed: Value = serde_json::from_str(&content[start..=end])
        .map_err(|_| anyhow::anyhow!("LLM extraction returned invalid JSON"))?;
    if !parsed.is_array() {
        bail!("LLM extraction did not return a JSON array");
    }
    Ok(parsed
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| item(v, types))
        .take(max)
        .collect())
}

fn distill_turns(turns: &[Turn], source: &str, commit: bool, config: &Value) -> Result<Vec<Value>> {
    let engine = config["distill"]["engine"].as_str().unwrap_or("heuristic");
    let types = allowed(config);
    let findings = match engine {
        "llm" => llm_findings(turns, config, &types)?,
        "host" => turns
            .iter()
            .filter(|turn| turn.role == "assistant")
            .flat_map(|turn| markdown_findings(&turn.text, &types))
            .collect(),
        "heuristic" => heuristic(turns, config)
            .into_iter()
            .filter(|finding| types.contains(&finding.kind))
            .collect(),
        _ => bail!("unknown distill engine: {engine}"),
    };
    process(findings, source, commit)
}

pub fn distill(text: &str, fmt: &str, source: &str, commit: bool) -> Result<Vec<Value>> {
    let config = load_config(Some(&destination()))?;
    distill_turns(&parse_transcript(text, fmt)?, source, commit, &config)
}

fn empty_event() -> Value {
    json!({"context":"","memory_ids":[],"approx_tokens":0})
}

fn turn_hash(turns: &[Turn]) -> String {
    let mut hasher = Sha256::new();
    for turn in turns {
        hasher.update(turn.role.as_bytes());
        hasher.update([0]);
        hasher.update(turn.text.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

pub fn session_end(payload: &Value) -> Result<Value> {
    let store = destination();
    let config = load_config(Some(&store))?;
    if config["distill"]["enabled"] != true {
        return Ok(empty_event());
    }
    let source = payload["source"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("agent");
    let transcript = &payload["transcript"];
    let (turns, key) = if let Some(path) = transcript["path"].as_str() {
        let raw = fs::read_to_string(path)?;
        (
            parse_transcript(&raw, transcript["format"].as_str().unwrap_or("auto"))?,
            Some(path),
        )
    } else {
        (
            parse_transcript(payload["text"].as_str().unwrap_or(""), "text")?,
            None,
        )
    };
    if turns.is_empty() {
        return Ok(empty_event());
    }
    let mut state_guard = None;
    let mut state = json!({"transcripts":{}});
    let mut start = 0;
    let state_path = store.root.join(".distill_state.json");
    if let Some(key) = key {
        fs::create_dir_all(&store.root)?;
        let lock_path = store.root.join(".distill_state.json.lock");
        for path in [&lock_path, &state_path] {
            if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
                bail!("symlink is not allowed: {}", path.display());
            }
        }
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.lock_exclusive()?;
        state_guard = Some(lock);
        if let Ok(raw) = fs::read_to_string(&state_path)
            && let Ok(parsed) = serde_json::from_str::<Value>(&raw)
        {
            state = parsed;
        }
        if !state.is_object() || !state["transcripts"].is_object() {
            state = json!({"transcripts":{}});
        }
        let old = &state["transcripts"][key];
        if old["version"] == 3 {
            let count = old["turns"].as_u64().unwrap_or(0) as usize;
            if count <= turns.len() && old["hash"] == turn_hash(&turns[..count]) {
                start = count;
            }
        }
    }
    if start == turns.len() {
        return Ok(empty_event());
    }
    let actions = distill_turns(&turns[start..], source, true, &config)?;
    if let Some(key) = key {
        state["transcripts"][key] =
            json!({"turns":turns.len(),"hash":turn_hash(&turns),"version":3});
        let temp = store
            .root
            .join(format!(".distill_state.{}.tmp", uuid::Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(serde_json::to_string(&state)?.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, &state_path)?;
        File::open(&store.root)?.sync_all()?;
    }
    drop(state_guard);
    let saved: Vec<_> = actions.iter().filter(|v| v["id"].is_string()).collect();
    if saved.is_empty() {
        return Ok(empty_event());
    }
    let mut lines = vec!["Mnemosyne: auto-saved memories from this session:".to_owned()];
    for action in &saved {
        lines.push(format!(
            "- [{}] {}: {} ({})",
            action["verdict"].as_str().unwrap_or("new"),
            action["type"].as_str().unwrap_or(""),
            action["title"].as_str().unwrap_or(""),
            action["id"].as_str().unwrap_or("")
        ));
    }
    let context = lines.join("\n");
    Ok(
        json!({"context":context,"memory_ids":saved.iter().map(|v| v["id"].clone()).collect::<Vec<_>>(),"approx_tokens":context.chars().count()/4}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codex_roles_are_preserved() {
        let rows = [
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"text","text":"不要用 pip，改用 uv"}]}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","channel":"analysis","content":[{"type":"text","text":"错误根因是配置，修复配置"}]}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","channel":"final","content":[{"type":"text","text":"done"}]}}),
        ];
        let raw = rows
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            parse_transcript(&raw, "auto")
                .unwrap()
                .iter()
                .map(|t| t.role.as_str())
                .collect::<Vec<_>>(),
            ["user", "assistant"]
        );
    }

    #[test]
    fn findings_and_host_respect_role() {
        let types = HashSet::from(["pitfall".to_owned()]);
        let block = "**Findings:**\n- type: pitfall\n- importance: 65\n- title: Port conflict\n- tags: service\n- content: |\n    The proxy uses port 9090\n";
        let findings = markdown_findings(block, &types);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].content, "The proxy uses port 9090");
        let turns = [
            Turn {
                role: "user".into(),
                text: block.into(),
            },
            Turn {
                role: "assistant".into(),
                text: block.into(),
            },
        ];
        let config = json!({"distill":{"engine":"host"},"memory":{"types":["pitfall"]}});
        let host_findings: Vec<_> = turns
            .iter()
            .filter(|t| t.role == "assistant")
            .flat_map(|t| markdown_findings(&t.text, &allowed(&config)))
            .collect();
        assert_eq!(host_findings.len(), 1);
        assert!(
            distill_turns(
                &turns,
                "agent",
                false,
                &json!({"distill":{"engine":"llm","llm":{"backend":"unsupported"}},"memory":{"types":["pitfall"]}})
            )
            .is_err()
        );
        assert_eq!(
            parse_llm_payload(
                "result: [{\"type\":\"pitfall\",\"title\":\"T\",\"content\":\"C\"}]",
                &types,
                5
            )
            .unwrap()
            .len(),
            1
        );
        assert!(parse_llm_payload("invalid reply", &types, 5).is_err());
    }
}
