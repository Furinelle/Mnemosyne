//! Rebuildable vector cache. Markdown remains the source of truth.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::models::{embed, fingerprint};
use crate::schema::{is_expired, parse_memory};
use crate::search::{SearchResult, memory_search_text};
use crate::store::{Store, load_config, load_memories, lock_store};

const CACHE_FILE: &str = "vectors-rust.sqlite";

pub fn backfill(stores: &[Store], include_archive: bool) -> Result<usize> {
    backfill_with(stores, include_archive, embed)
}

fn backfill_with<F>(stores: &[Store], include_archive: bool, mut compute: F) -> Result<usize>
where
    F: FnMut(&Value, &[String]) -> Result<Vec<Vec<f64>>>,
{
    let mut total = 0;
    for store in stores {
        let config = load_config(Some(store))?;
        let settings = &config["embedding"];
        if !settings["enabled"].as_bool().unwrap_or(false) {
            bail!("embedding is disabled for {} store", store.scope);
        }
        let model = fingerprint(settings)?;
        let dimensions = settings["dimensions"].as_u64().unwrap_or(512) as usize;
        let entries = load_memories(store, include_archive)?;
        let existing = cached_hashes(store, &model, dimensions)?;
        let pending = entries
            .into_iter()
            .filter_map(|(path, memory)| {
                let text = memory_search_text(&memory);
                let hash = content_hash(&text);
                (existing.get(&path).map(String::as_str) != Some(hash.as_str()))
                    .then_some((path, text, hash))
            })
            .collect::<Vec<_>>();
        let batch_size = settings["batch_size"].as_u64().unwrap_or(32).max(1) as usize;
        for batch in pending.chunks(batch_size) {
            // Model calls happen outside the store lock.  The post-call CAS
            // below discards vectors for a memory edited while the model ran.
            let texts = batch
                .iter()
                .map(|(_, text, _)| text.clone())
                .collect::<Vec<_>>();
            let vectors = compute(settings, &texts)?;
            if vectors.len() != batch.len() {
                bail!("Embedding count mismatch");
            }
            let _lock = lock_store(store)?;
            let current_config = load_config(Some(store))?;
            let current_settings = &current_config["embedding"];
            if !current_settings["enabled"].as_bool().unwrap_or(false)
                || current_settings["dimensions"].as_u64().unwrap_or(512) as usize != dimensions
                || fingerprint(current_settings)? != model
            {
                continue;
            }
            let connection = open(store)?;
            for ((path, _, hash), vector) in batch.iter().zip(vectors) {
                let current = std::fs::read_to_string(path)
                    .ok()
                    .and_then(|text| parse_memory(&text).ok())
                    .map(|memory| content_hash(&memory_search_text(&memory)));
                if current.as_deref() != Some(hash) {
                    continue;
                }
                connection.execute(
                    "INSERT INTO vectors (path, content_hash, fingerprint, dimensions, vector_json)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(path) DO UPDATE SET content_hash=excluded.content_hash,
                       fingerprint=excluded.fingerprint, dimensions=excluded.dimensions, vector_json=excluded.vector_json",
                    params![path.to_string_lossy(), hash, model, dimensions as i64, serde_json::to_string(&vector)?],
                )?;
                total += 1;
            }
        }
    }
    Ok(total)
}

pub fn lane(
    stores: &[Store],
    query: &str,
    archive: bool,
    config: &Value,
) -> Result<Vec<SearchResult>> {
    let settings = &config["embedding"];
    if !settings["enabled"].as_bool().unwrap_or(false) {
        return Ok(vec![]);
    }
    let model = fingerprint(settings)?;
    let dimensions = settings["dimensions"].as_u64().unwrap_or(512) as usize;
    let query_vector = embed(settings, &[query.to_owned()])?
        .into_iter()
        .next()
        .context("Missing query vector")?;
    let mut results = Vec::new();
    for store in stores {
        let vectors = cached_vectors(store, &model, dimensions)?;
        for (path, memory) in load_memories(store, archive)? {
            if !archive
                && (path.starts_with(store.root.join("archive")) || is_expired(&memory.expires))
            {
                continue;
            }
            let text_hash = content_hash(&memory_search_text(&memory));
            let Some(vector) = vectors.get(&path) else {
                continue;
            };
            if vector.0 != text_hash {
                continue;
            }
            let score = cosine(&query_vector, &vector.1);
            if score <= 0.0 {
                continue;
            }
            results.push(SearchResult {
                store: store.clone(),
                path,
                memory,
                score,
                why_matched: String::new(),
                score_breakdown: json!({"vec": score}),
            });
        }
    }
    results.sort_by(|left, right| right.score.total_cmp(&left.score));
    Ok(results)
}

fn cache_path(store: &Store) -> Result<PathBuf> {
    crate::store::cache_path(store, CACHE_FILE)
}

fn open(store: &Store) -> Result<Connection> {
    std::fs::create_dir_all(&store.root)?;
    let connection = Connection::open(cache_path(store)?)?;
    connection.execute_batch(
        "PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS vectors (
        path TEXT PRIMARY KEY, content_hash TEXT NOT NULL, fingerprint TEXT NOT NULL,
        dimensions INTEGER NOT NULL, vector_json TEXT NOT NULL
    );",
    )?;
    Ok(connection)
}

fn cached_hashes(
    store: &Store,
    model: &str,
    dimensions: usize,
) -> Result<HashMap<PathBuf, String>> {
    if !cache_path(store)?.exists() {
        return Ok(HashMap::new());
    }
    let connection =
        Connection::open_with_flags(cache_path(store)?, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement = connection
        .prepare("SELECT path, content_hash FROM vectors WHERE fingerprint=?1 AND dimensions=?2")?;
    statement
        .query_map(params![model, dimensions as i64], |row| {
            Ok((PathBuf::from(row.get::<_, String>(0)?), row.get(1)?))
        })?
        .collect::<rusqlite::Result<HashMap<_, _>>>()
        .map_err(Into::into)
}

fn cached_vectors(
    store: &Store,
    model: &str,
    dimensions: usize,
) -> Result<HashMap<PathBuf, (String, Vec<f64>)>> {
    if !cache_path(store)?.exists() {
        return Ok(HashMap::new());
    }
    let connection =
        Connection::open_with_flags(cache_path(store)?, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement = connection.prepare("SELECT path, content_hash, vector_json FROM vectors WHERE fingerprint=?1 AND dimensions=?2")?;
    let rows = statement.query_map(params![model, dimensions as i64], |row| {
        Ok((
            PathBuf::from(row.get::<_, String>(0)?),
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut vectors = HashMap::new();
    for row in rows {
        let (path, hash, raw) = row?;
        if let Ok(vector) = serde_json::from_str::<Vec<f64>>(&raw)
            && valid_vector(&vector, dimensions)
        {
            vectors.insert(path, (hash, vector));
        }
    }
    Ok(vectors)
}

fn content_hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn cosine(left: &[f64], right: &[f64]) -> f64 {
    if left.len() != right.len()
        || left.is_empty()
        || !left.iter().chain(right).all(|value| value.is_finite())
    {
        return 0.0;
    }
    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f64>();
    let left_norm = left.iter().map(|value| value * value).sum::<f64>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f64>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        let score = dot / (left_norm * right_norm);
        if score.is_finite() { score } else { 0.0 }
    }
}

fn valid_vector(vector: &[f64], dimensions: usize) -> bool {
    vector.len() == dimensions
        && vector.iter().all(|value| value.is_finite())
        && vector.iter().any(|value| *value != 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, Store) {
        let root = tempdir().unwrap();
        let store = Store {
            scope: "project".into(),
            root: root.path().join(".mnemosyne"),
        };
        crate::store::ensure_store(&store).unwrap();
        std::fs::write(store.config_path(), "[embedding]\nenabled = true\nbackend = 'openai'\nmodel = 'test-model'\ndimensions = 2\nbatch_size = 8\n").unwrap();
        (root, store)
    }

    fn write(store: &Store, body: &str) -> PathBuf {
        let memory = crate::schema::Memory {
            id: "memory".into(),
            memory_type: "codebase".into(),
            body: body.into(),
            ..Default::default()
        };
        let path = crate::store::working_path(store, &memory).unwrap();
        crate::store::write_memory(&path, &memory).unwrap();
        path
    }

    fn fake(_settings: &Value, texts: &[String]) -> Result<Vec<Vec<f64>>> {
        Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
    }

    #[test]
    fn body_change_and_model_dimension_change_fail_the_write_cas() {
        let (_root, store) = store();
        let path = write(&store, "original body");
        let changed = store.clone();
        assert_eq!(backfill_with(std::slice::from_ref(&store), false, move |settings, texts| {
            std::fs::write(changed.config_path(), "[embedding]\nenabled = true\nbackend = 'openai'\nmodel = 'test-model'\ndimensions = 3\n").unwrap();
            fake(settings, texts)
        }).unwrap(), 0);
        std::fs::write(store.config_path(), "[embedding]\nenabled = true\nbackend = 'openai'\nmodel = 'test-model'\ndimensions = 2\n").unwrap();
        let changed = store.clone();
        assert_eq!(
            backfill_with(
                std::slice::from_ref(&store),
                false,
                move |settings, texts| {
                    let memory = crate::schema::Memory {
                        id: "memory".into(),
                        memory_type: "codebase".into(),
                        body: "changed body".into(),
                        ..Default::default()
                    };
                    crate::store::write_memory(&path, &memory).unwrap();
                    fake(settings, texts)
                }
            )
            .unwrap(),
            0
        );
        assert!(
            cached_hashes(
                &changed,
                &fingerprint(&load_config(Some(&changed)).unwrap()["embedding"]).unwrap(),
                2
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn access_only_change_reuses_the_vector_and_invalid_vectors_are_ignored() {
        let (_root, store) = store();
        let path = write(&store, "stable body");
        assert_eq!(
            backfill_with(std::slice::from_ref(&store), false, fake).unwrap(),
            1
        );
        let mut memory = parse_memory(&std::fs::read_to_string(&path).unwrap()).unwrap();
        memory.access_count += 1;
        memory.last_accessed = "2026-09-19".into();
        crate::store::write_memory(&path, &memory).unwrap();
        assert_eq!(
            backfill_with(std::slice::from_ref(&store), false, |_settings, _| bail!(
                "must not recompute"
            ))
            .unwrap(),
            0
        );
        let config = load_config(Some(&store)).unwrap();
        let model = fingerprint(&config["embedding"]).unwrap();
        let connection = open(&store).unwrap();
        connection
            .execute(
                "UPDATE vectors SET vector_json='[0.0, 0.0]' WHERE path=?1",
                params![path.to_string_lossy()],
            )
            .unwrap();
        assert!(cached_vectors(&store, &model, 2).unwrap().is_empty());
        assert_eq!(cosine(&[f64::NAN, 0.0], &[1.0, 0.0]), 0.0);
    }
}
