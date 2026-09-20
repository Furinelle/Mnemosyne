//! Rebuildable vector cache. Markdown remains the source of truth.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Result, bail};
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[cfg(test)]
use crate::models::fingerprint;
use crate::models::{InvalidEmbedding, ModelProfile, embed, valid_vector};
use crate::schema::{is_expired, parse_memory};
use crate::search::{SearchResult, memory_search_text};
use crate::store::{Store, load_config, load_memories, lock_store};

const CACHE_FILE: &str = "vectors-rust.sqlite";

#[derive(Default, Debug, serde::Serialize)]
pub struct BackfillStats {
    pub computed: usize,
    pub written: usize,
    pub skipped_stale: usize,
    pub invalid: usize,
    pub repaired: usize,
    pub failed: usize,
}

pub fn backfill(stores: &[Store], include_archive: bool) -> Result<usize> {
    let stats = backfill_stats(stores, include_archive)?;
    if stats.failed != 0 || stats.invalid != 0 {
        bail!(
            "Embedding backfill failed: {} failed, {} invalid",
            stats.failed,
            stats.invalid
        );
    }
    Ok(stats.written)
}

pub fn backfill_stats(stores: &[Store], include_archive: bool) -> Result<BackfillStats> {
    backfill_stats_with(stores, include_archive, embed)
}

#[cfg(test)]
fn backfill_with<F>(stores: &[Store], archive: bool, compute: F) -> Result<usize>
where
    F: FnMut(&Value, &[String]) -> Result<Vec<Vec<f64>>>,
{
    Ok(backfill_stats_with(stores, archive, compute)?.written)
}

fn backfill_stats_with<F>(
    stores: &[Store],
    include_archive: bool,
    mut compute: F,
) -> Result<BackfillStats>
where
    F: FnMut(&Value, &[String]) -> Result<Vec<Vec<f64>>>,
{
    let mut stats = BackfillStats::default();
    for store in stores {
        let config = load_config(Some(store))?;
        let settings = &config["embedding"];
        if !settings["enabled"].as_bool().unwrap_or(false) {
            bail!("embedding is disabled for {} store", store.scope);
        }
        let profile = ModelProfile::new(settings)?;
        let entries = load_memories(store, include_archive)?;
        let (existing, corrupt) = cached_vectors(store, &profile.fingerprint, profile.dimensions)?;
        let pending = entries
            .into_iter()
            .filter_map(|(path, memory)| {
                let text = memory_search_text(&memory);
                let hash = content_hash(&text);
                (existing.get(&path).map(|v| v.0.as_str()) != Some(hash.as_str()))
                    .then_some((path, text, hash))
            })
            .collect::<Vec<_>>();
        let batch_size = settings["batch_size"].as_u64().unwrap_or(32).max(1) as usize;
        for batch in pending.chunks(batch_size) {
            // Model calls remain outside the store lock; commit only the snapshot's results.
            let texts = batch
                .iter()
                .map(|(_, text, _)| text.clone())
                .collect::<Vec<_>>();
            let vectors = match compute(profile.settings(), &texts) {
                Ok(vectors) => vectors,
                Err(error) => {
                    if error.is::<InvalidEmbedding>() {
                        stats.invalid += batch.len();
                    } else {
                        stats.failed += batch.len();
                    }
                    continue;
                }
            };
            stats.computed += vectors.len();
            if vectors.len() != batch.len()
                || !vectors.iter().all(|v| valid_vector(v, profile.dimensions))
            {
                stats.invalid += batch.len();
                continue;
            }
            let _lock = lock_store(store)?;
            if !profile.matches(&load_config(Some(store))?["embedding"]) {
                stats.skipped_stale += batch.len();
                continue;
            }
            let commit = (|| -> Result<(usize, usize, usize)> {
                let mut connection = open(store)?;
                let transaction = connection.transaction()?;
                let (mut written, mut repaired, mut stale) = (0, 0, 0);
                for ((path, _, hash), vector) in batch.iter().zip(vectors) {
                    let current = std::fs::read_to_string(path)
                        .ok()
                        .and_then(|text| parse_memory(&text).ok())
                        .map(|memory| content_hash(&memory_search_text(&memory)));
                    if current.as_deref() != Some(hash) {
                        stale += 1;
                        continue;
                    }
                    transaction.execute(
                        "INSERT INTO vectors (path, content_hash, fingerprint, dimensions, vector_json)
                         VALUES (?1, ?2, ?3, ?4, ?5)
                         ON CONFLICT(path) DO UPDATE SET content_hash=excluded.content_hash,
                           fingerprint=excluded.fingerprint, dimensions=excluded.dimensions, vector_json=excluded.vector_json",
                        params![path.to_string_lossy(), hash, profile.fingerprint, profile.dimensions as i64, serde_json::to_string(&vector)?],
                    )?;
                    written += 1;
                    repaired += usize::from(corrupt.contains(path));
                }
                transaction.commit()?;
                Ok((written, repaired, stale))
            })();
            match commit {
                Ok((written, repaired, stale)) => {
                    stats.written += written;
                    stats.repaired += repaired;
                    stats.skipped_stale += stale;
                }
                Err(_) => stats.failed += batch.len(),
            }
        }
    }
    Ok(stats)
}

#[derive(Debug)]
pub struct VectorDiagnostic {
    pub status: &'static str,
    pub reason: &'static str,
}
impl std::fmt::Display for VectorDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.reason)
    }
}
impl std::error::Error for VectorDiagnostic {}
fn diagnostic(status: &'static str, reason: &'static str) -> anyhow::Error {
    VectorDiagnostic { status, reason }.into()
}

pub fn lane(
    stores: &[Store],
    query: &str,
    archive: bool,
    config: &Value,
) -> Result<Vec<SearchResult>> {
    lane_with(stores, query, archive, config, embed)
}

fn lane_with<F>(
    stores: &[Store],
    query: &str,
    archive: bool,
    config: &Value,
    mut compute: F,
) -> Result<Vec<SearchResult>>
where
    F: FnMut(&Value, &[String]) -> Result<Vec<Vec<f64>>>,
{
    let settings = &config["embedding"];
    if !settings["enabled"].as_bool().unwrap_or(false) {
        return Ok(vec![]);
    }
    if settings["backend"].as_str().unwrap_or("onnx") != "onnx" {
        crate::timing::not_run("model_initialize");
    }
    let profile = ModelProfile::new(settings)
        .map_err(|_| diagnostic("missing_assets", "model_or_vocabulary_unavailable"))?;
    // Keep the caller's selected profile and each store's configuration snapshots.
    // Different scopes can have distinct settings; none may change during inference.
    let snapshots = stores
        .iter()
        .map(|store| Ok((store, load_config(Some(store))?["embedding"].clone())))
        .collect::<Result<Vec<_>>>()?;
    let vectors = compute(profile.settings(), &[query.to_owned()])
        .map_err(|_| diagnostic("incompatible", "provider_unavailable_or_invalid"))?;
    if vectors.len() != 1 || !valid_vector(&vectors[0], profile.dimensions) {
        return Err(diagnostic("incompatible", "invalid_query_vector"));
    }
    if !profile.matches(settings)
        || snapshots.iter().any(|(store, snapshot)| {
            load_config(Some(store)).map_or(true, |current| current["embedding"] != *snapshot)
        })
    {
        return Err(diagnostic("stale", "profile_changed_during_inference"));
    }
    let query_vector = &vectors[0];
    let mut results = Vec::new();
    for store in stores {
        let (vectors, corrupt) = cached_vectors(store, &profile.fingerprint, profile.dimensions)
            .map_err(|_| diagnostic("corrupt", "cache_unreadable"))?;
        if !corrupt.is_empty() {
            return Err(diagnostic("corrupt", "invalid_cached_vector"));
        }
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
            let score = cosine(query_vector, &vector.1);
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

/// Read-only status; never starts inference or creates/rebuilds a cache.
pub fn diagnose(store: &Store) -> Value {
    let check = || -> Result<Value> {
        let config = load_config(Some(store))?;
        let settings = &config["embedding"];
        if !settings["enabled"].as_bool().unwrap_or(false) {
            return Ok(json!({"status":"disabled"}));
        }
        let profile = match ModelProfile::new(settings) {
            Ok(profile) => profile,
            Err(_) => return Ok(json!({"status":"missing_assets"})),
        };
        let backend = settings["backend"].as_str().unwrap_or("onnx");
        if !matches!(backend, "onnx" | "openai" | "openai-compatible")
            || (backend == "onnx" && !cfg!(feature = "onnx"))
        {
            return Ok(json!({"status":"incompatible", "reason":"unsupported_backend_or_build"}));
        }
        if !cache_path(store)?.exists() {
            return Ok(json!({"status":"stale", "reason":"cache_missing"}));
        }
        let (cached, corrupt) = cached_vectors(store, &profile.fingerprint, profile.dimensions)?;
        if !corrupt.is_empty() {
            return Ok(json!({"status":"corrupt", "invalid":corrupt.len()}));
        }
        let connection =
            Connection::open_with_flags(cache_path(store)?, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let incompatible: i64 = connection.query_row(
            "SELECT count(*) FROM vectors WHERE fingerprint != ?1 OR dimensions != ?2",
            params![profile.fingerprint, profile.dimensions as i64],
            |row| row.get(0),
        )?;
        if incompatible != 0 {
            return Ok(json!({"status":"incompatible", "rows":incompatible}));
        }
        let stale = load_memories(store, true)?
            .iter()
            .filter(|(path, memory)| {
                cached
                    .get(path)
                    .is_none_or(|v| v.0 != content_hash(&memory_search_text(memory)))
            })
            .count();
        Ok(if stale != 0 {
            json!({"status":"stale", "rows":stale})
        } else {
            json!({"status":"ready", "rows":cached.len()})
        })
    };
    check().unwrap_or_else(|_| json!({"status":"corrupt", "reason":"cache_or_config_unreadable"}))
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

type CachedVectors = HashMap<PathBuf, (String, Vec<f64>)>;

fn cached_vectors(
    store: &Store,
    model: &str,
    dimensions: usize,
) -> Result<(CachedVectors, HashSet<PathBuf>)> {
    if !cache_path(store)?.exists() {
        return Ok((HashMap::new(), HashSet::new()));
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
    let mut corrupt = HashSet::new();
    for row in rows {
        let (path, hash, raw) = row?;
        if let Ok(vector) = serde_json::from_str::<Vec<f64>>(&raw)
            && valid_vector(&vector, dimensions)
        {
            vectors.insert(path, (hash, vector));
        } else {
            corrupt.insert(path);
        }
    }
    Ok((vectors, corrupt))
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
    fn corrupt_vectors_are_repaired_and_diagnostics_distinguish_states() {
        let (_root, store) = store();
        write(&store, "stable body");
        assert_eq!(diagnose(&store)["status"], "stale");
        assert_eq!(
            backfill_with(std::slice::from_ref(&store), false, fake).unwrap(),
            1
        );
        assert_eq!(diagnose(&store)["status"], "ready");
        for raw in ["[0,0]", "[1]", "{broken", "[null,1]"] {
            open(&store)
                .unwrap()
                .execute("UPDATE vectors SET vector_json=?1", [raw])
                .unwrap();
            assert_eq!(diagnose(&store)["status"], "corrupt");
            let stats = backfill_stats_with(std::slice::from_ref(&store), false, fake).unwrap();
            assert_eq!(
                (stats.computed, stats.written, stats.repaired, stats.failed),
                (1, 1, 1, 0)
            );
            assert_eq!(diagnose(&store)["status"], "ready");
        }
        open(&store)
            .unwrap()
            .execute("UPDATE vectors SET fingerprint='old'", [])
            .unwrap();
        assert_eq!(diagnose(&store)["status"], "incompatible");
        std::fs::write(store.config_path(), "[embedding]\nenabled=false\n").unwrap();
        assert_eq!(diagnose(&store)["status"], "disabled");
        std::fs::write(
            store.config_path(),
            "[embedding]\nenabled=true\nbackend='onnx'\nmodel='no-such-assets'\n",
        )
        .unwrap();
        assert_eq!(diagnose(&store)["status"], "missing_assets");
    }

    #[test]
    fn inference_config_changes_are_stale_and_calls_are_unlocked() {
        for change in ["dimensions = 3", "model = 'different'", "revision = 'v2'"] {
            let (_root, store) = store();
            write(&store, "body");
            let stats =
                backfill_stats_with(std::slice::from_ref(&store), false, |settings, texts| {
                    let _lock = lock_store(&store).unwrap();
                    let config =
                        format!("[embedding]\nenabled=true\nbackend='openai'\n{}\n", change);
                    std::fs::write(store.config_path(), config).unwrap();
                    fake(settings, texts)
                })
                .unwrap();
            assert_eq!(
                (stats.computed, stats.written, stats.skipped_stale),
                (1, 0, 1)
            );
        }
    }

    #[test]
    fn query_discards_profile_and_local_asset_changes() {
        let (_root, store) = store();
        write(&store, "body");
        backfill_with(std::slice::from_ref(&store), false, fake).unwrap();
        let config = load_config(Some(&store)).unwrap();
        let error = lane_with(
            std::slice::from_ref(&store),
            "body",
            false,
            &config,
            |settings, texts| {
                let _lock = lock_store(&store).unwrap();
                let current = std::fs::read_to_string(store.config_path()).unwrap();
                std::fs::write(store.config_path(), current + "revision='v2'\n").unwrap();
                fake(settings, texts)
            },
        )
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<VectorDiagnostic>().unwrap().reason,
            "profile_changed_during_inference"
        );
        let model = store.root.join("model.onnx");
        std::fs::write(&model, "model bytes").unwrap();
        std::fs::write(model.with_file_name("vocab.txt"), "vocabulary").unwrap();
        let local = json!({"embedding":{"enabled":true, "backend":"onnx", "dimensions":2, "onnx_path":model}});
        let error = lane_with(&[store], "body", false, &local, |settings, texts| {
            std::fs::write(&model, "changed model").unwrap();
            fake(settings, texts)
        })
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<VectorDiagnostic>().unwrap().status,
            "stale"
        );
    }

    #[test]
    fn invalid_provider_results_never_commit_and_batch_failure_rolls_back() {
        let (_root, store) = store();
        write(&store, "body");
        for vector in [
            vec![0.0, 0.0],
            vec![1.0],
            vec![f64::NAN, 0.0],
            vec![f64::INFINITY, 0.0],
        ] {
            let stats = backfill_stats_with(std::slice::from_ref(&store), false, |_, _| {
                Ok(vec![vector.clone()])
            })
            .unwrap();
            assert_eq!((stats.written, stats.invalid), (0, 1));
        }
        let stats =
            backfill_stats_with(std::slice::from_ref(&store), false, |_, _| bail!("offline"))
                .unwrap();
        assert_eq!((stats.written, stats.failed), (0, 1));
        let other = crate::schema::Memory {
            id: "other".into(),
            memory_type: "codebase".into(),
            body: "other".into(),
            ..Default::default()
        };
        crate::store::write_memory(&crate::store::working_path(&store, &other).unwrap(), &other)
            .unwrap();
        open(&store).unwrap().execute_batch("CREATE TRIGGER fail_second BEFORE INSERT ON vectors WHEN (SELECT count(*) FROM vectors) = 1 BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
        let stats = backfill_stats_with(std::slice::from_ref(&store), false, fake).unwrap();
        assert_eq!((stats.written, stats.failed), (0, 2));
        let count: i64 = open(&store)
            .unwrap()
            .query_row("SELECT count(*) FROM vectors", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn corrupt_sqlite_and_missing_cache_preserve_lexical_results() {
        let (_root, store) = store();
        write(&store, "lexical needle");
        std::fs::write(cache_path(&store).unwrap(), "broken sqlite").unwrap();
        assert_eq!(diagnose(&store)["status"], "corrupt");
        let results = crate::search::search(
            std::slice::from_ref(&store),
            "needle",
            5,
            "",
            false,
            false,
            &json!({}),
        )
        .unwrap();
        assert_eq!(results[0].memory.id, "memory");
        assert!(
            lane_with(
                std::slice::from_ref(&store),
                "needle",
                false,
                &load_config(Some(&store)).unwrap(),
                fake
            )
            .is_err()
        );
        std::fs::remove_file(cache_path(&store).unwrap()).unwrap();
        assert!(
            lane_with(
                std::slice::from_ref(&store),
                "needle",
                false,
                &load_config(Some(&store)).unwrap(),
                fake
            )
            .unwrap()
            .is_empty()
        );
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
            cached_vectors(
                &changed,
                &fingerprint(&load_config(Some(&changed)).unwrap()["embedding"]).unwrap(),
                2
            )
            .unwrap()
            .0
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
        assert!(cached_vectors(&store, &model, 2).unwrap().0.is_empty());
        assert_eq!(
            backfill_with(std::slice::from_ref(&store), false, fake).unwrap(),
            1,
            "corrupt cache must be repaired"
        );
        assert_eq!(cosine(&[f64::NAN, 0.0], &[1.0, 0.0]), 0.0);
    }
}
