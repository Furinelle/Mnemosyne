//! Lexical retrieval over the markdown memory stores.
//!
//! The SQLite file is only a disposable local cache.  Markdown remains the
//! source of truth, and the Rust cache deliberately has a different name from
//! Python's `index.sqlite` so the two implementations can be evaluated side by
//! side.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::Result;
use rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::schema::{Memory, is_expired};
use crate::store::{Store, load_memories, lock_store};

const CACHE_FILE: &str = "rust-index.sqlite";
const CACHE_VERSION: i64 = 2;

/// Split dense-script runs into overlapping bigrams, and preserve ordinary
/// lower-cased word runs.  This intentionally mirrors the Python tokenizer.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut run = String::new();
    let mut dense = None;
    for ch in text.chars().flat_map(char::to_lowercase) {
        let kind = if is_dense(ch) {
            Some(true)
        } else if is_word(ch) {
            Some(false)
        } else {
            None
        };
        if kind == dense && kind.is_some() {
            run.push(ch);
            continue;
        }
        push_run(&mut tokens, &run, dense.unwrap_or(false));
        run.clear();
        dense = kind;
        if kind.is_some() {
            run.push(ch);
        }
    }
    push_run(&mut tokens, &run, dense.unwrap_or(false));
    tokens
}

fn is_dense(ch: char) -> bool {
    matches!(ch,
        '\u{3040}'..='\u{30ff}' | '\u{3400}'..='\u{4dbf}' |
        '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}' |
        '\u{ac00}'..='\u{d7af}'
    )
}

fn is_word(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric() || ('\u{00c0}'..='\u{024f}').contains(&ch)
}

fn push_run(tokens: &mut Vec<String>, run: &str, dense: bool) {
    if run.is_empty() {
        return;
    }
    if !dense {
        tokens.push(run.to_owned());
        return;
    }
    let chars: Vec<_> = run.chars().collect();
    if chars.len() == 1 {
        tokens.push(run.to_owned());
    } else {
        tokens.extend(chars.windows(2).map(|pair| pair.iter().collect()));
    }
}

pub fn memory_search_text(memory: &Memory) -> String {
    [
        memory.canonical_summary.as_str(),
        memory.injection_summary.as_str(),
        memory.body.as_str(),
        &memory.tags.join(" "),
        memory.memory_type.as_str(),
        memory.id.as_str(),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("\n")
}

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub store: Store,
    pub path: PathBuf,
    pub memory: Memory,
    pub score: f64,
    pub why_matched: String,
    pub score_breakdown: Value,
}

#[derive(Clone)]
struct IndexedMemory {
    store: Store,
    path: PathBuf,
    memory: Memory,
    archived: bool,
}

pub fn reindex_store(store: &Store, include_archive: bool) -> Result<usize> {
    let _lock = lock_store(store)?;
    crate::relations::recover_pending(store)?;
    let mut connection = Connection::open(cache_path(store)?)?;
    ensure_cache(&mut connection)?;
    let transaction = connection.transaction()?;
    transaction.execute("DELETE FROM memories", [])?;
    transaction.execute("DELETE FROM memories_fts", [])?;
    let mut count = 0;
    for path in memory_paths(store, include_archive)? {
        if let ConsistentMemory::Ready(memory, fingerprint) = read_memory_consistent(&path)? {
            upsert_index(&transaction, store, &path, &memory, fingerprint)?;
            count += 1;
        }
    }
    transaction.commit()?;
    Ok(count)
}

fn sync_store(store: &Store, include_archive: bool) -> Result<()> {
    let _timing = crate::timing::Scope::new("sqlite_sync");
    let _lock = lock_store(store)?;
    crate::relations::recover_pending(store)?;
    let mut connection = Connection::open(cache_path(store)?)?;
    ensure_cache(&mut connection)?;
    let indexed = connection
        .prepare("SELECT path, mtime_ns, size FROM memories")?
        .query_map([], |row| {
            Ok((
                PathBuf::from(row.get::<_, String>(0)?),
                (row.get::<_, i64>(1)?, row.get::<_, i64>(2)?),
            ))
        })?
        .collect::<rusqlite::Result<HashMap<_, _>>>()?;
    let transaction = connection.transaction()?;
    let mut seen = HashSet::new();
    for path in memory_paths(store, include_archive)? {
        let fingerprint = match file_fingerprint(&path) {
            Ok(fingerprint) => fingerprint,
            Err(_) => continue,
        };
        seen.insert(path.clone());
        if indexed.get(&path) == Some(&fingerprint) {
            continue;
        }
        match read_memory_consistent(&path)? {
            ConsistentMemory::Changed => continue,
            ConsistentMemory::Invalid => {
                if indexed.contains_key(&path) {
                    transaction.execute(
                        "DELETE FROM memories_fts WHERE path = ?1",
                        params![path.to_string_lossy()],
                    )?;
                    transaction.execute(
                        "DELETE FROM memories WHERE path = ?1",
                        params![path.to_string_lossy()],
                    )?;
                }
            }
            ConsistentMemory::Ready(memory, fingerprint) => {
                if indexed.contains_key(&path) {
                    transaction.execute(
                        "DELETE FROM memories_fts WHERE path = ?1",
                        params![path.to_string_lossy()],
                    )?;
                    transaction.execute(
                        "DELETE FROM memories WHERE path = ?1",
                        params![path.to_string_lossy()],
                    )?;
                }
                upsert_index(&transaction, store, &path, &memory, fingerprint)?;
            }
        }
    }
    for path in indexed.keys().filter(|path| !seen.contains(*path)) {
        transaction.execute(
            "DELETE FROM memories_fts WHERE path = ?1",
            params![path.to_string_lossy()],
        )?;
        transaction.execute(
            "DELETE FROM memories WHERE path = ?1",
            params![path.to_string_lossy()],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

fn ensure_cache(connection: &mut Connection) -> Result<()> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version != CACHE_VERSION {
        connection
            .execute_batch("DROP TABLE IF EXISTS memories; DROP TABLE IF EXISTS memories_fts;")?;
    }
    connection.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS memories (
           path TEXT PRIMARY KEY, memory_json TEXT NOT NULL, text TEXT NOT NULL,
           archived INTEGER NOT NULL, mtime_ns INTEGER NOT NULL, size INTEGER NOT NULL
         );
         CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
           path UNINDEXED, text, tokenize='trigram'
         );
         PRAGMA user_version=2;",
    )?;
    Ok(())
}

fn upsert_index(
    connection: &rusqlite::Transaction<'_>,
    store: &Store,
    path: &Path,
    memory: &Memory,
    (mtime_ns, size): (i64, i64),
) -> Result<()> {
    let text = memory_search_text(memory);
    let archived = path.starts_with(store.root.join("archive"));
    connection.execute(
        "INSERT INTO memories (path, memory_json, text, archived, mtime_ns, size) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![path.to_string_lossy(), serde_json::to_string(memory)?, text, archived, mtime_ns, size],
    )?;
    connection.execute(
        "INSERT INTO memories_fts (path, text) VALUES (?1, ?2)",
        params![path.to_string_lossy(), memory_search_text(memory)],
    )?;
    Ok(())
}

fn file_fingerprint(path: &Path) -> Result<(i64, i64)> {
    let metadata = fs::metadata(path)?;
    let mtime_ns = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(i64::MAX as u128) as i64;
    Ok((mtime_ns, metadata.len().min(i64::MAX as u64) as i64))
}

enum ConsistentMemory {
    Ready(Box<Memory>, (i64, i64)),
    Invalid,
    Changed,
}

fn read_memory_consistent(path: &Path) -> Result<ConsistentMemory> {
    for _ in 0..2 {
        let before = match file_fingerprint(path) {
            Ok(fingerprint) => fingerprint,
            Err(_) => return Ok(ConsistentMemory::Changed),
        };
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(_) => return Ok(ConsistentMemory::Changed),
        };
        let after = match file_fingerprint(path) {
            Ok(fingerprint) => fingerprint,
            Err(_) => return Ok(ConsistentMemory::Changed),
        };
        if before != after {
            continue;
        }
        return Ok(crate::schema::parse_memory(&text)
            .map_or(ConsistentMemory::Invalid, |memory| {
                ConsistentMemory::Ready(Box::new(memory), after)
            }));
    }
    Ok(ConsistentMemory::Changed)
}

fn memory_paths(store: &Store, include_archive: bool) -> Result<Vec<PathBuf>> {
    let _timing = crate::timing::Scope::new("file_enumeration");
    let mut paths = regular_markdown(&store.working_dir())?;
    if include_archive {
        let archive = store.archive_dir();
        if archive.exists() {
            if fs::symlink_metadata(&archive)?.file_type().is_symlink() {
                return Ok(paths);
            }
            for month in fs::read_dir(archive)? {
                let month = month?.path();
                if fs::symlink_metadata(&month)?.file_type().is_symlink() || !month.is_dir() {
                    continue;
                }
                paths.extend(regular_markdown(&month)?);
            }
        }
    }
    paths.sort();
    Ok(paths)
}

fn regular_markdown(directory: &Path) -> Result<Vec<PathBuf>> {
    if !directory.exists() || fs::symlink_metadata(directory)?.file_type().is_symlink() {
        return Ok(vec![]);
    }
    Ok(fs::read_dir(directory)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "md")
                && fs::symlink_metadata(path)
                    .map(|metadata| metadata.file_type().is_file())
                    .unwrap_or(false)
        })
        .collect())
}

pub fn search(
    stores: &[Store],
    query: &str,
    limit: usize,
    type_filter: &str,
    include_archive: bool,
    include_superseded: bool,
    config: &Value,
) -> Result<Vec<SearchResult>> {
    if query.trim().is_empty() || limit == 0 {
        return Ok(vec![]);
    }
    let indexed = config_bool(config, &["search", "index_enabled"], true);
    let all = read_stores(stores, include_archive, indexed)?;
    let direct = all
        .iter()
        .filter(|entry| eligible(entry, type_filter, include_archive, include_superseded))
        .cloned()
        .collect::<Vec<_>>();
    let query_tokens = tokenize(query);
    if query_tokens.is_empty() {
        return Ok(vec![]);
    }

    // The indexed lane deliberately uses SQLite FTS ranking, as the Python
    // implementation does.  BM25 remains the fallback for CJK bigrams too
    // short for the trigram tokenizer (and for SQLite builds without FTS5).
    let fts_scores = if indexed {
        fts_scores(stores, query, &direct)?
    } else {
        HashMap::new()
    };
    let short_scores = if indexed {
        short_cjk_scores(&direct, query)
    } else {
        HashMap::new()
    };
    let direct_scores = bm25(&direct, &query_tokens);
    let has_index_matches = !fts_scores.is_empty() || !short_scores.is_empty();
    let mut candidates = HashMap::new();
    for (entry, bm25_score) in direct.into_iter().zip(direct_scores) {
        let key = identity(&entry);
        let fts = fts_scores.get(&key).copied();
        let short = short_scores.get(&key).copied();
        let indexed_lane = fts.is_some() || short.is_some();
        let indexed_score = fts.unwrap_or(0.0) + short.unwrap_or(0.0);
        if (!has_index_matches && bm25_score <= 0.0) || (has_index_matches && !indexed_lane) {
            continue;
        }
        let score = if indexed_lane {
            indexed_score
        } else {
            bm25_score
        };
        let mut score_breakdown = serde_json::Map::new();
        if let Some(score) = fts {
            score_breakdown.insert("fts".into(), Value::from(score));
        }
        if let Some(score) = short {
            score_breakdown.insert("cjk_like".into(), Value::from(score));
        }
        if !indexed_lane {
            score_breakdown.insert("bm25".into(), Value::from(score));
        }
        candidates.insert(
            key,
            SearchResult {
                store: entry.store,
                path: entry.path,
                memory: entry.memory,
                score,
                why_matched: query_tokens.join(" "),
                score_breakdown: Value::Object(score_breakdown),
            },
        );
    }

    let lexical_ranked = sorted_candidates(&candidates);
    let mut vector_diagnostic = None;
    let embedding_enabled = config_bool(config, &["embedding", "enabled"], false);
    let rerank_enabled = config_bool(config, &["rerank", "enabled"], false);
    if !embedding_enabled && !rerank_enabled {
        crate::timing::not_run("model_profile");
        crate::timing::not_run("model_fingerprint");
        crate::timing::not_run("model_initialize");
    }
    let vector_ranked = if embedding_enabled {
        match crate::vectors::lane(stores, query, include_archive, config) {
            Ok(results) => results,
            Err(error) => {
                // Only emit our static diagnostic, never a backend URL or provider body.
                let (status, reason) = error
                    .downcast_ref::<crate::vectors::VectorDiagnostic>()
                    .map(|d| (d.status, d.reason))
                    .unwrap_or(("incompatible", "vector_query_failed"));
                vector_diagnostic = Some(json!({"status":status, "reason":reason}));
                eprintln!(
                    "mnemosyne: vector query {status} ({reason}); falling back to lexical retrieval"
                );
                vec![]
            }
        }
    } else {
        vec![]
    };
    if !vector_ranked.is_empty() {
        for result in &vector_ranked {
            let key = result_identity(result);
            if let Some(existing) = candidates.get_mut(&key) {
                merge_breakdown(&mut existing.score_breakdown, &result.score_breakdown);
            } else {
                candidates.insert(key, result.clone());
            }
        }
        let vector_ids = vector_ranked
            .iter()
            .map(result_identity)
            .collect::<Vec<_>>();
        for (key, score) in rrf(
            &[lexical_ranked, vector_ids],
            config_usize(config, &["fusion", "rrf_k"], 60),
        ) {
            if let Some(candidate) = candidates.get_mut(&key) {
                candidate.score = score;
            }
        }
    }

    if config_bool(config, &["fusion", "link_expansion"], true) {
        let _timing = crate::timing::Scope::new("graph_expansion");
        expand_links(&mut candidates, &all, include_archive, config);
    }

    let mut results = candidates
        .into_values()
        .filter(|result| eligible_result(result, type_filter, include_archive, include_superseded))
        .collect::<Vec<_>>();
    results.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| result_identity(right).cmp(&result_identity(left)))
    });
    if let Some(diagnostic) = vector_diagnostic {
        for result in &mut results {
            result.score_breakdown["vector_diagnostic"] = diagnostic.clone();
        }
    }
    rerank(query, &mut results, config);
    results.truncate(limit);
    Ok(results)
}

fn sorted_candidates(candidates: &HashMap<String, SearchResult>) -> Vec<String> {
    let mut results = candidates.values().collect::<Vec<_>>();
    results.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| result_identity(right).cmp(&result_identity(left)))
    });
    results.into_iter().map(result_identity).collect()
}

fn rrf(lanes: &[Vec<String>], k: usize) -> HashMap<String, f64> {
    let mut scores = HashMap::new();
    for lane in lanes {
        for (index, id) in lane.iter().enumerate() {
            *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (k + index + 1) as f64;
        }
    }
    scores
}

fn merge_breakdown(target: &mut Value, extra: &Value) {
    if let (Some(target), Some(extra)) = (target.as_object_mut(), extra.as_object()) {
        target.extend(extra.clone());
    }
}

fn rerank(query: &str, results: &mut [SearchResult], config: &Value) {
    if !config_bool(config, &["rerank", "enabled"], false) {
        return;
    }
    let count = config_usize(config, &["rerank", "top_n"], 5)
        .saturating_mul(2)
        .min(results.len());
    if count == 0 {
        return;
    }
    let docs = results[..count]
        .iter()
        .map(|result| memory_search_text(&result.memory))
        .collect::<Vec<_>>();
    match crate::models::rerank(&config["rerank"], query, &docs) {
        Ok(scores) if scores.len() == count && scores.iter().any(|score| *score != 0.0) => {
            for (result, score) in results[..count].iter_mut().zip(scores) {
                result.score = score;
                result.score_breakdown["rerank"] = Value::from(score);
            }
            results[..count].sort_by(|left, right| right.score.total_cmp(&left.score));
        }
        _ => eprintln!("mnemosyne: reranking failed; keeping lexical/vector ordering"),
    }
}

fn cache_path(store: &Store) -> Result<PathBuf> {
    crate::store::cache_path(store, CACHE_FILE)
}

fn read_stores(
    stores: &[Store],
    include_archive: bool,
    indexed: bool,
) -> Result<Vec<IndexedMemory>> {
    let mut entries = Vec::new();
    for store in stores {
        if indexed {
            sync_store(store, include_archive)?;
            let _timing = crate::timing::Scope::new("candidate_read");
            let connection = Connection::open(cache_path(store)?)?;
            let mut statement =
                connection.prepare("SELECT path, memory_json, archived FROM memories")?;
            let rows = statement.query_map([], |row| {
                Ok((
                    PathBuf::from(row.get::<_, String>(0)?),
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            })?;
            for row in rows {
                let (path, memory, archived) = row?;
                entries.push(IndexedMemory {
                    store: store.clone(),
                    path,
                    archived,
                    memory: serde_json::from_str(&memory)?,
                });
            }
        } else {
            let _timing = crate::timing::Scope::new("candidate_read");
            for (path, memory) in load_memories(store, include_archive)? {
                let archived = path.starts_with(store.root.join("archive"));
                entries.push(IndexedMemory {
                    store: store.clone(),
                    path,
                    memory,
                    archived,
                });
            }
        }
    }
    Ok(entries)
}

fn eligible(
    entry: &IndexedMemory,
    type_filter: &str,
    include_archive: bool,
    include_superseded: bool,
) -> bool {
    (type_filter.is_empty() || entry.memory.memory_type == type_filter)
        && (include_archive || (!entry.archived && !is_expired(&entry.memory.expires)))
        && (include_superseded || entry.memory.status != "superseded")
}

fn eligible_result(
    result: &SearchResult,
    type_filter: &str,
    include_archive: bool,
    include_superseded: bool,
) -> bool {
    (type_filter.is_empty() || result.memory.memory_type == type_filter)
        && (include_archive
            || (!result.path.starts_with(result.store.root.join("archive"))
                && !is_expired(&result.memory.expires)))
        && (include_superseded || result.memory.status != "superseded")
}

fn bm25(entries: &[IndexedMemory], query: &[String]) -> Vec<f64> {
    let tokenized = entries
        .iter()
        .map(|entry| tokenize(&memory_search_text(&entry.memory)))
        .collect::<Vec<_>>();
    let lengths = tokenized.iter().map(Vec::len).collect::<Vec<_>>();
    let average = lengths.iter().sum::<usize>() as f64 / lengths.len().max(1) as f64;
    let mut document_frequency = HashMap::<&str, usize>::new();
    for tokens in &tokenized {
        let unique = tokens.iter().map(String::as_str).collect::<HashSet<_>>();
        for token in unique {
            *document_frequency.entry(token).or_default() += 1;
        }
    }
    tokenized
        .iter()
        .zip(lengths)
        .map(|(tokens, length)| {
            let frequencies =
                tokens
                    .iter()
                    .fold(HashMap::<&str, usize>::new(), |mut counts, token| {
                        *counts.entry(token).or_default() += 1;
                        counts
                    });
            query
                .iter()
                .filter_map(|token| {
                    let frequency = *frequencies.get(token.as_str())? as f64;
                    let df = *document_frequency.get(token.as_str()).unwrap_or(&0) as f64;
                    let total = entries.len() as f64;
                    let idf = (1.0 + (total - df + 0.5) / (df + 0.5)).ln();
                    let denominator =
                        frequency + 1.5 * (1.0 - 0.75 + 0.75 * length as f64 / average.max(1.0));
                    Some(idf * frequency * 2.5 / denominator)
                })
                .sum()
        })
        .collect()
}

fn fts_scores(
    stores: &[Store],
    query: &str,
    eligible_entries: &[IndexedMemory],
) -> Result<HashMap<String, f64>> {
    let expression = fts_expression(query);
    if expression.is_empty() {
        return Ok(HashMap::new());
    }
    let allowed = eligible_entries
        .iter()
        .map(|entry| (entry.path.clone(), (identity(entry), entry.memory.strength)))
        .collect::<HashMap<_, _>>();
    let mut scores = HashMap::new();
    for store in stores {
        let connection = Connection::open(cache_path(store)?)?;
        let mut statement = connection.prepare(
            "SELECT path, -bm25(memories_fts) FROM memories_fts WHERE memories_fts MATCH ?1",
        )?;
        let rows = statement.query_map(params![expression], |row| {
            Ok((
                PathBuf::from(row.get::<_, String>(0)?),
                row.get::<_, f64>(1)?,
            ))
        })?;
        for row in rows {
            let (path, score) = row?;
            if let Some((key, strength)) = allowed.get(&path) {
                scores.insert(key.clone(), score + *strength as f64 / 1000.0);
            }
        }
    }
    Ok(scores)
}

fn short_cjk_scores(entries: &[IndexedMemory], query: &str) -> HashMap<String, f64> {
    let tokens = tokenize(query)
        .into_iter()
        .filter(|token| token.chars().count() < 3 && token.chars().next().is_some_and(is_dense))
        .collect::<Vec<_>>();
    let mut scores = HashMap::new();
    for entry in entries {
        let text = memory_search_text(&entry.memory);
        let hits = tokens
            .iter()
            .filter(|token| text.contains(token.as_str()))
            .count();
        if hits > 0 {
            scores.insert(
                identity(entry),
                hits as f64 + entry.memory.strength as f64 / 1000.0,
            );
        }
    }
    scores
}

fn fts_expression(query: &str) -> String {
    let mut terms = Vec::new();
    let mut run = String::new();
    let mut dense = None;
    for ch in query.chars().flat_map(char::to_lowercase) {
        let kind = if is_dense(ch) {
            Some(true)
        } else if is_word(ch) {
            Some(false)
        } else {
            None
        };
        if kind == dense && kind.is_some() {
            run.push(ch);
            continue;
        }
        fts_terms(&mut terms, &run, dense.unwrap_or(false));
        run.clear();
        dense = kind;
        if kind.is_some() {
            run.push(ch);
        }
    }
    fts_terms(&mut terms, &run, dense.unwrap_or(false));
    terms.sort();
    terms.dedup();
    terms
        .into_iter()
        .map(|term| format!("\"{}\"", term.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn fts_terms(terms: &mut Vec<String>, run: &str, dense: bool) {
    if run.is_empty() {
        return;
    }
    if !dense {
        terms.push(run.to_owned());
        return;
    }
    let chars: Vec<_> = run.chars().collect();
    if chars.len() < 3 {
        return;
    }
    terms.extend(chars.windows(3).map(|part| part.iter().collect()));
    if chars.len() > 3 {
        terms.push(run.to_owned());
    }
}

fn expand_links(
    candidates: &mut HashMap<String, SearchResult>,
    all: &[IndexedMemory],
    include_archive: bool,
    config: &Value,
) {
    // Bare IDs are a legacy format.  Python resolves them by the caller's
    // store order, so retain only the first visible target rather than fanning
    // a link out across same-id records in other scopes.
    let mut by_id = HashMap::<&str, &IndexedMemory>::new();
    for entry in all {
        by_id.entry(&entry.memory.id).or_insert(entry);
    }
    let decay = config_f64(config, &["fusion", "link_expansion_decay_fallback"], 0.5);
    let max_hops = config_usize(config, &["fusion", "link_expansion_max_hops"], 1);
    let max_sources = config_usize(config, &["fusion", "link_expansion_max_sources"], 10);
    let boost_ratio = config_f64(config, &["fusion", "link_expansion_max_boost_ratio"], 0.8);
    let mut ranked = candidates.values().cloned().collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.score.total_cmp(&left.score));
    let boost_cap = ranked
        .first()
        .map_or(0.0, |entry| entry.score * boost_ratio);
    let mut frontier = if max_sources == 0 {
        ranked
    } else {
        ranked.into_iter().take(max_sources).collect()
    };
    let mut visited = HashSet::new();
    for hop in 0..max_hops {
        let mut next = Vec::new();
        for source in frontier {
            let source_key = result_identity(&source);
            if !visited.insert(source_key) {
                continue;
            }
            for link in &source.memory.links {
                let Some(target) = by_id.get(link.id.as_str()) else {
                    continue;
                };
                if !include_archive && (target.archived || is_expired(&target.memory.expires)) {
                    continue;
                }
                let target_key = identity(target);
                let warning = link.rel == "contradicts";
                let boost = if link.rel == "supersedes" || warning {
                    0.0
                } else {
                    source.score * relation_weight(&link.rel, config) * decay.powi(hop as i32)
                };
                let inserted = if !candidates.contains_key(&target_key) {
                    if boost <= 0.0 && !warning {
                        continue;
                    }
                    candidates.insert(
                        target_key.clone(),
                        SearchResult {
                            store: target.store.clone(),
                            path: target.path.clone(),
                            memory: target.memory.clone(),
                            score: 0.0,
                            why_matched: String::new(),
                            score_breakdown: json!({"link_boost": 0.0}),
                        },
                    );
                    true
                } else {
                    false
                };
                let candidate = candidates.get_mut(&target_key).expect("inserted above");
                let accumulated = candidate
                    .score_breakdown
                    .get("link_boost")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                let base = candidate.score - accumulated;
                let mut total = accumulated + boost;
                if boost_cap > 0.0 {
                    total = total.min(boost_cap);
                }
                candidate.score = base + total;
                candidate.score_breakdown["link_boost"] = Value::from(total);
                if warning {
                    let values = candidate
                        .score_breakdown
                        .as_object_mut()
                        .expect("object")
                        .entry("contradicts_with")
                        .or_insert_with(|| Value::Array(vec![]));
                    if let Some(values) = values.as_array_mut()
                        && !values.iter().any(|value| value == &source.memory.id)
                    {
                        values.push(Value::from(source.memory.id.clone()));
                    }
                }
                if inserted {
                    next.push(candidate.clone());
                }
            }
        }
        frontier = next;
    }
}

fn relation_weight(relation: &str, config: &Value) -> f64 {
    config
        .get("fusion")
        .and_then(|value| value.get("relation_weight_override"))
        .and_then(|value| value.get(relation))
        .and_then(Value::as_f64)
        .unwrap_or(match relation {
            "caused_by" => 0.6,
            "refines" => 0.7,
            "supersedes" => 0.3,
            "contradicts" | "related" => 0.5,
            _ => 0.5,
        })
}

fn identity(entry: &IndexedMemory) -> String {
    format!(
        "{}:{}:{}",
        entry.store.scope,
        entry.store.root.display(),
        entry.memory.id
    )
}

fn result_identity(result: &SearchResult) -> String {
    format!(
        "{}:{}:{}",
        result.store.scope,
        result.store.root.display(),
        result.memory.id
    )
}

fn config_at<'a>(config: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(config, |value, key| value.get(*key))
}

fn config_bool(config: &Value, path: &[&str], default: bool) -> bool {
    config_at(config, path)
        .and_then(Value::as_bool)
        .unwrap_or(default)
}

fn config_f64(config: &Value, path: &[&str], default: f64) -> f64 {
    config_at(config, path)
        .and_then(Value::as_f64)
        .unwrap_or(default)
}

fn config_usize(config: &Value, path: &[&str], default: usize) -> usize {
    config_at(config, path)
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn memory(id: &str, body: &str) -> Memory {
        Memory {
            id: id.to_owned(),
            memory_type: "codebase".to_owned(),
            body: body.to_owned(),
            ..Default::default()
        }
    }

    fn write(store: &Store, memory: &Memory) {
        let path = store.root.join("working").join(format!("{}.md", memory.id));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, crate::schema::serialize_memory(memory)).unwrap();
    }

    #[test]
    fn cjk_bigrams_match_and_expired_entries_do_not_consume_the_limit() {
        let dir = tempdir().unwrap();
        let store = Store {
            scope: "project".to_owned(),
            root: dir.path().to_owned(),
        };
        let mut expired = memory("expired", "认证失败 认证失败");
        expired.expires = "2000-01-01".to_owned();
        write(&store, &expired);
        write(&store, &memory("active", "调试认证失败"));
        let results = search(&[store], "认证", 1, "", false, false, &json!({})).unwrap();
        assert_eq!(
            results
                .iter()
                .map(|result| result.memory.id.as_str())
                .collect::<Vec<_>>(),
            ["active"]
        );
    }

    #[test]
    fn graph_expansion_obeys_hops_and_boost_cap() {
        let dir = tempdir().unwrap();
        let store = Store {
            scope: "project".to_owned(),
            root: dir.path().to_owned(),
        };
        let mut source = memory("source", "needle");
        source.links = vec![crate::schema::Link {
            id: "middle".to_owned(),
            rel: "refines".to_owned(),
        }];
        let mut middle = memory("middle", "other");
        middle.links = vec![crate::schema::Link {
            id: "tail".to_owned(),
            rel: "related".to_owned(),
        }];
        write(&store, &source);
        write(&store, &middle);
        write(&store, &memory("tail", "other"));
        let config = json!({"fusion": {"link_expansion_max_hops": 1}});
        let one_hop = search(
            std::slice::from_ref(&store),
            "needle",
            5,
            "",
            false,
            false,
            &config,
        )
        .unwrap();
        assert_eq!(
            one_hop
                .iter()
                .map(|result| result.memory.id.as_str())
                .collect::<Vec<_>>(),
            ["source", "middle"]
        );
        let two_hops = search(
            &[store],
            "needle",
            5,
            "",
            false,
            false,
            &json!({"fusion": {"link_expansion_max_hops": 2}}),
        )
        .unwrap();
        assert!(two_hops.iter().any(|result| result.memory.id == "tail"));
    }

    #[test]
    fn superseded_is_filtered_and_vector_failure_degrades_to_lexical() {
        let dir = tempdir().unwrap();
        let store = Store {
            scope: "project".to_owned(),
            root: dir.path().to_owned(),
        };
        let mut old = memory("old", "needle needle needle");
        old.status = "superseded".to_owned();
        write(&store, &old);
        write(&store, &memory("current", "needle"));
        let results = search(
            std::slice::from_ref(&store),
            "needle",
            1,
            "",
            false,
            false,
            &json!({}),
        )
        .unwrap();
        assert_eq!(results[0].memory.id, "current");
        let fallback = search(
            &[store],
            "needle",
            1,
            "",
            false,
            false,
            &json!({"embedding": {"enabled": true}}),
        )
        .unwrap();
        assert_eq!(fallback[0].memory.id, "current");
    }

    #[test]
    fn default_indexed_search_reports_fts_not_bm25() {
        let dir = tempdir().unwrap();
        let store = Store {
            scope: "project".to_owned(),
            root: dir.path().to_owned(),
        };
        write(&store, &memory("fts", "persistent lexical needle"));
        let results = search(&[store], "needle", 1, "", false, false, &json!({})).unwrap();
        assert!(results[0].score_breakdown.get("fts").is_some());
    }

    #[test]
    fn mixed_fts_and_short_cjk_lanes_are_merged() {
        let dir = tempdir().unwrap();
        let store = Store {
            scope: "project".to_owned(),
            root: dir.path().to_owned(),
        };
        write(&store, &memory("cjk-only", "混合 evidence"));
        write(&store, &memory("english-only", "portalocker unrelated"));
        let results = search(
            &[store],
            "混合 portalocker",
            5,
            "",
            false,
            false,
            &json!({}),
        )
        .unwrap();
        assert_eq!(
            results
                .iter()
                .map(|result| result.memory.id.as_str())
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from(["english-only", "cjk-only"])
        );
        assert!(
            results
                .iter()
                .any(|result| result.memory.id == "english-only"
                    && result.score_breakdown.get("fts").is_some())
        );
        assert!(results.iter().any(|result| result.memory.id == "cjk-only"
            && result.score_breakdown.get("cjk_like").is_some()));
    }

    #[test]
    fn legacy_bare_links_resolve_the_first_store_target() {
        let dir = tempdir().unwrap();
        let global = Store {
            scope: "global".to_owned(),
            root: dir.path().join("global"),
        };
        let project = Store {
            scope: "project".to_owned(),
            root: dir.path().join("project"),
        };
        write(&global, &memory("target", "global target"));
        write(&project, &memory("target", "project target"));
        let mut source = memory("source", "link needle");
        source.links = vec![crate::schema::Link {
            id: "target".into(),
            rel: "refines".into(),
        }];
        write(&project, &source);
        let results = search(
            &[global, project],
            "link needle",
            5,
            "",
            false,
            false,
            &json!({}),
        )
        .unwrap();
        let targets = results
            .iter()
            .filter(|result| result.memory.id == "target")
            .collect::<Vec<_>>();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].store.scope, "global");
    }

    #[test]
    fn incremental_sync_refreshes_manual_canonical_edits_and_removes_deleted_files() {
        let dir = tempdir().unwrap();
        let store = Store {
            scope: "project".to_owned(),
            root: dir.path().to_owned(),
        };
        let mut original = memory("record", "unchanged body");
        original.canonical_summary = "old canonical".into();
        write(&store, &original);
        assert_eq!(
            search(
                std::slice::from_ref(&store),
                "old",
                1,
                "",
                false,
                false,
                &json!({})
            )
            .unwrap()[0]
                .memory
                .id,
            "record"
        );
        let mut edited = original.clone();
        edited.canonical_summary = "manual canonical replacement".into();
        write(&store, &edited);
        assert_eq!(
            search(
                std::slice::from_ref(&store),
                "replacement",
                1,
                "",
                false,
                false,
                &json!({})
            )
            .unwrap()[0]
                .memory
                .id,
            "record"
        );
        std::fs::remove_file(store.root.join("working/record.md")).unwrap();
        assert!(
            search(&[store], "replacement", 1, "", false, false, &json!({}))
                .unwrap()
                .is_empty()
        );
    }
}
