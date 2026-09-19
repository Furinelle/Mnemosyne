//! Deterministic lexical retrieval regression gate.
//!
//! This evaluates the real Rust search path in a disposable store.  It never
//! labels the lexical lane as a vector/reranked "full" pipeline.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;

use crate::schema::Memory;
use crate::search;
use crate::store::{Store, ensure_store, working_path, write_memory};

const DEFAULT_CORPUS: &str = include_str!("../assets/eval/default_corpus.jsonl");
const DEFAULT_SEEDS: &str = include_str!("../assets/eval/seed_memories.jsonl");
const LONGMEMEVAL_SAMPLE: &str = include_str!("../assets/eval/fixtures/longmemeval_sample.json");
const MIN_RECALL: f64 = 0.95;

#[derive(Deserialize)]
struct Seed {
    id: String,
    #[serde(rename = "type", default = "default_memory_type")]
    memory_type: String,
    text: String,
    #[serde(default)]
    instance_id: String,
}
fn default_memory_type() -> String {
    "codebase".into()
}

fn convert(args: &[String]) -> Result<()> {
    use std::io::Write;
    anyhow::ensure!(
        args.first().is_some_and(|v| v == "longmemeval"),
        "Expected convert longmemeval"
    );
    let mut raw = None;
    let mut out = None;
    let mut max = usize::MAX;
    let mut values = args[1..].iter();
    while let Some(key) = values.next() {
        let value = values.next().context("Missing conversion option value")?;
        match key.as_str() {
            "--raw" => raw = Some(value),
            "--out" => out = Some(value),
            "--max-instances" => max = value.parse()?,
            _ => bail!("Unknown conversion option: {key}"),
        }
    }
    let raw: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(raw.context("--raw is required")?)?)?;
    let out = Path::new(out.context("--out is required")?);
    std::fs::create_dir_all(out)?;
    let mut seeds = tempfile::NamedTempFile::new_in(out)?;
    let mut corpus = tempfile::NamedTempFile::new_in(out)?;
    let mut cases = 0;
    for instance in raw.iter().take(max) {
        let qid = instance["question_id"]
            .as_str()
            .context("Missing question_id")?;
        let ids = instance["haystack_session_ids"]
            .as_array()
            .context("Missing session IDs")?;
        let sessions = instance["haystack_sessions"]
            .as_array()
            .context("Missing sessions")?;
        anyhow::ensure!(
            ids.len() == sessions.len(),
            "Session IDs and sessions differ in length"
        );
        for (i, (id, session)) in ids.iter().zip(sessions).enumerate() {
            let sid = id.as_str().context("Session ID must be a string")?;
            let text = session
                .as_array()
                .context("Session must be a list")?
                .iter()
                .map(|turn| turn["content"].as_str().unwrap_or(""))
                .collect::<Vec<_>>()
                .join("\n");
            writeln!(
                seeds,
                "{}",
                json!({"id":format!("lme-{qid}-{sid}"),"text":text,"instance_id":qid,"session_date":instance["haystack_dates"].get(i).and_then(serde_json::Value::as_str).unwrap_or("")})
            )?;
        }
        let expected: Vec<_> = instance["answer_session_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .map(|sid| format!("lme-{qid}-{sid}"))
            .collect();
        if expected.is_empty() {
            continue;
        }
        writeln!(
            corpus,
            "{}",
            json!({"query":instance["question"].as_str().context("Missing question")?,"expected_ids":expected,"instance_id":qid,"question_type":instance["question_type"].as_str().unwrap_or(""),"notes":instance["answer"].as_str().unwrap_or(""),"paraphrase_of":""})
        )?;
        cases += 1;
    }
    seeds.as_file().sync_all()?;
    corpus.as_file().sync_all()?;
    seeds.persist(out.join("seed_memories.jsonl"))?;
    corpus.persist(out.join("corpus.jsonl"))?;
    println!("Converted {cases} questions to {}", out.display());
    Ok(())
}

#[derive(Deserialize)]
struct Case {
    query: String,
    expected_ids: Vec<String>,
    #[serde(default)]
    instance_id: String,
    #[serde(default)]
    question_type: String,
}

pub fn run(args: &[String]) -> Result<()> {
    if args.first().is_some_and(|s| s == "convert") {
        return convert(&args[1..]);
    }
    if args.first().is_some_and(|s| s == "fetch") {
        bail!(
            "Automatic LongMemEval download is not configured; obtain the official dataset and use eval convert longmemeval --raw FILE --out DIR"
        );
    }
    let options = Options::parse(args)?;
    if !matches!(options.pipeline.as_str(), "lexical" | "bm25" | "full") {
        bail!("unknown eval pipeline: {}", options.pipeline);
    }
    if options.pipeline == "full" && !fts5_available() {
        bail!("pipeline=full requires SQLite FTS5, which is unavailable");
    }
    let (cases, seeds) = if let Some(corpus) = options.corpus.as_deref() {
        let corpus = std::fs::read_to_string(corpus)?;
        let seed_path = Path::new(options.corpus.as_ref().unwrap())
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("seed_memories.jsonl");
        (
            parse_lines(&corpus)?,
            parse_lines(&std::fs::read_to_string(seed_path)?)?,
        )
    } else if options.longmemeval {
        longmemeval_sample()?
    } else {
        (parse_lines(DEFAULT_CORPUS)?, parse_lines(DEFAULT_SEEDS)?)
    };
    let report = evaluate(&cases, &seeds, options.pipeline == "full")?;
    println!(
        "pipeline: rust-{} (lexical lane={}; vectors=disabled; rerank=disabled) recall@5={:.3} MRR={:.3} p50={:.3}ms p99={:.3}ms queries={}",
        options.pipeline,
        if options.pipeline == "full" {
            "SQLite FTS5 with CJK BM25 fallback"
        } else {
            "BM25"
        },
        report.recall,
        report.mrr,
        report.p50,
        report.p99,
        cases.len()
    );
    if options.by_type {
        for (kind, metrics) in report.by_type {
            println!(
                "  [{}] recall@5={:.3}",
                if kind.is_empty() { "unknown" } else { &kind },
                metrics.0
            );
        }
    }
    let minimum = options.min_recall.unwrap_or(MIN_RECALL);
    if report.recall < minimum {
        bail!(
            "FAIL: recall@5 {:.3} < required {:.3}; inspect per-query misses before changing corpus or threshold",
            report.recall,
            minimum
        );
    }
    Ok(())
}

// The public parser intentionally stays tiny: CLI wiring owns command shape;
// this accepts the arguments passed after `eval`.
struct Options {
    corpus: Option<String>,
    longmemeval: bool,
    by_type: bool,
    min_recall: Option<f64>,
    pipeline: String,
}

impl Options {
    fn parse(args: &[String]) -> Result<Self> {
        let mut result = Self {
            corpus: None,
            longmemeval: false,
            by_type: false,
            min_recall: None,
            pipeline: "bm25".into(),
        };
        let mut values = args.iter().map(String::as_str).peekable();
        if values.peek() == Some(&"run") {
            values.next();
        }
        while let Some(arg) = values.next() {
            match arg {
                "--corpus" => {
                    result.corpus = Some(values.next().context("--corpus needs a path")?.to_owned())
                }
                "--longmemeval" => result.longmemeval = true,
                "--by-type" => result.by_type = true,
                "--min-recall" => {
                    result.min_recall = Some(
                        values
                            .next()
                            .context("--min-recall needs a number")?
                            .parse()?,
                    )
                }
                "--pipeline" => {
                    result.pipeline = values
                        .next()
                        .context("--pipeline needs a value")?
                        .to_owned()
                }
                _ => bail!("unknown eval option: {arg}"),
            }
        }
        anyhow::ensure!(
            matches!(result.pipeline.as_str(), "bm25" | "full" | "lexical"),
            "Unknown evaluation pipeline"
        );
        Ok(result)
    }
}

struct Report {
    recall: f64,
    mrr: f64,
    p50: f64,
    p99: f64,
    by_type: Vec<(String, (f64, usize))>,
}

fn evaluate(cases: &[Case], seeds: &[Seed], indexed: bool) -> Result<Report> {
    let root = tempfile::tempdir()?;
    let mut stores = HashMap::<String, Store>::new();
    for seed in seeds {
        let store = stores
            .entry(seed.instance_id.clone())
            .or_insert_with(|| Store {
                scope: "project".into(),
                // Corpus identifiers are data, never filesystem paths.
                root: root.path().join(uuid::Uuid::new_v4().simple().to_string()),
            });
        ensure_store(store)?;
        let memory = Memory {
            id: seed.id.clone(),
            memory_type: seed.memory_type.clone(),
            source: "eval".into(),
            strength: 50,
            canonical_summary: seed.text.chars().take(200).collect(),
            injection_summary: seed.text.chars().take(200).collect(),
            body: seed.text.clone(),
            status: "active".into(),
            ..Default::default()
        };
        write_memory(&working_path(store, &memory)?, &memory)?;
    }
    let mut recalls = Vec::new();
    let mut ranks = Vec::new();
    let mut durations = Vec::new();
    let mut types = HashMap::<String, (f64, usize)>::new();
    for case in cases {
        let store = stores
            .get(&case.instance_id)
            .or_else(|| stores.get(""))
            .with_context(|| format!("no seed memories for instance {}", case.instance_id))?;
        let started = Instant::now();
        let results = search::search(
            std::slice::from_ref(store),
            &case.query,
            5,
            "",
            false,
            false,
            &json!({"search":{"index_enabled":indexed}, "fusion":{"link_expansion":indexed}}),
        )?;
        durations.push(started.elapsed().as_secs_f64() * 1000.0);
        let ids = results
            .iter()
            .map(|result| result.memory.id.as_str())
            .collect::<Vec<_>>();
        let recall = case
            .expected_ids
            .iter()
            .filter(|id| ids.contains(&id.as_str()))
            .count() as f64
            / case.expected_ids.len().max(1) as f64;
        let rank = ids
            .iter()
            .position(|id| case.expected_ids.iter().any(|expected| expected == id))
            .map_or(0.0, |rank| 1.0 / (rank + 1) as f64);
        recalls.push(recall);
        ranks.push(rank);
        let entry = types.entry(case.question_type.clone()).or_insert((0.0, 0));
        entry.0 += recall;
        entry.1 += 1;
    }
    durations.sort_by(f64::total_cmp);
    Ok(Report {
        recall: mean(&recalls),
        mrr: mean(&ranks),
        p50: percentile(&durations, 0.50),
        p99: percentile(&durations, 0.99),
        by_type: types
            .into_iter()
            .map(|(kind, (sum, count))| (kind, (sum / count as f64, count)))
            .collect(),
    })
}

fn parse_lines<T: for<'a> Deserialize<'a>>(input: &str) -> Result<Vec<T>> {
    input
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(Into::into))
        .collect()
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len().max(1) as f64
}
fn percentile(values: &[f64], quantile: f64) -> f64 {
    values
        .get((values.len().saturating_sub(1) as f64 * quantile).round() as usize)
        .copied()
        .unwrap_or(0.0)
}

fn fts5_available() -> bool {
    rusqlite::Connection::open_in_memory()
        .and_then(|connection| {
            connection
                .execute("CREATE VIRTUAL TABLE t USING fts5(text)", [])
                .map(|_| ())
        })
        .is_ok()
}

fn longmemeval_sample() -> Result<(Vec<Case>, Vec<Seed>)> {
    let raw: Vec<serde_json::Value> = serde_json::from_str(LONGMEMEVAL_SAMPLE)?;
    let mut cases = Vec::new();
    let mut seeds = Vec::new();
    for item in raw {
        let id = item["question_id"].as_str().unwrap_or_default();
        let sessions = item["haystack_sessions"]
            .as_array()
            .context("sample sessions")?;
        let session_ids = item["haystack_session_ids"]
            .as_array()
            .context("sample session ids")?;
        for (session, session_id) in sessions.iter().zip(session_ids) {
            let text = session
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|turn| turn["content"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            seeds.push(Seed {
                id: format!("lme-{id}-{}", session_id.as_str().unwrap_or_default()),
                memory_type: "codebase".into(),
                text,
                instance_id: id.into(),
            });
        }
        let expected_ids: Vec<String> = item["answer_session_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|session| session.as_str())
            .map(|session| format!("lme-{id}-{session}"))
            .collect();
        if !expected_ids.is_empty() {
            cases.push(Case {
                query: item["question"].as_str().unwrap_or_default().into(),
                expected_ids,
                instance_id: id.into(),
                question_type: item["question_type"].as_str().unwrap_or_default().into(),
            });
        }
    }
    Ok((cases, seeds))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_real_search_gate_meets_the_fixed_threshold() {
        let cases = parse_lines(DEFAULT_CORPUS).unwrap();
        let seeds = parse_lines(DEFAULT_SEEDS).unwrap();
        assert!(evaluate(&cases, &seeds, true).unwrap().recall >= MIN_RECALL);
    }
    #[test]
    fn full_names_the_actual_disabled_lanes() {
        assert!(run(&["run".into(), "--pipeline".into(), "full".into()]).is_ok());
    }
    #[test]
    fn external_converted_corpus_is_used_in_longmemeval_mode() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let raw = temp.path().join("raw.json");
        let out = temp.path().join("converted");
        std::fs::write(&raw, LONGMEMEVAL_SAMPLE)?;
        convert(&[
            "longmemeval".into(),
            "--raw".into(),
            raw.to_string_lossy().into(),
            "--out".into(),
            out.to_string_lossy().into(),
        ])?;
        let cases: Vec<Case> = parse_lines(&std::fs::read_to_string(out.join("corpus.jsonl"))?)?;
        let seeds: Vec<Seed> =
            parse_lines(&std::fs::read_to_string(out.join("seed_memories.jsonl"))?)?;
        assert_eq!(cases.len(), 2);
        assert!(evaluate(&cases, &seeds, true)?.recall >= MIN_RECALL);
        // If run accidentally substitutes its built-in sample, this deliberately
        // impossible external oracle would pass instead of rejecting the gate.
        std::fs::write(
            out.join("corpus.jsonl"),
            "{\"query\":\"unfindable\",\"expected_ids\":[\"missing\"]}\n",
        )?;
        assert!(
            run(&[
                "run".into(),
                "--longmemeval".into(),
                "--corpus".into(),
                out.join("corpus.jsonl").to_string_lossy().into(),
                "--min-recall".into(),
                "0.95".into()
            ])
            .is_err()
        );
        Ok(())
    }
}
