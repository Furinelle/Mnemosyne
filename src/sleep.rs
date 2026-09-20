//! Explicit bounded offline review batches. Reports are metadata, never transcript copies.
use crate::{
    checkpoint::Checkpoint,
    provenance::{Clock, MemoryRef, read_manifest},
    schema::{Memory, parse_memory},
    store::{Store, lock_store_read_only},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

const MAX_INPUTS: usize = 10_000;
const MAX_SCAN_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 128 * 1024;
const MAX_ISSUES: usize = 100;
const MAX_PROPOSALS: usize = 20;
const MAX_REPORT_BYTES: usize = 64 * 1024;

fn memory_kind() -> String {
    "memory".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    #[serde(default = "memory_kind")]
    pub kind: String,
    pub memory_ref: MemoryRef,
    pub revision: u64,
    pub semantic_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub version: u32,
    pub snapshot: String,
    pub cursor: usize,
    pub next_cursor: usize,
    pub partial: bool,
    pub inputs: Vec<Input>,
    pub issues: Vec<Value>,
}

fn hash(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

fn directory(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "invalid sleep input directory"
            );
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn bounded_files_in(path: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    if !directory(path)? {
        return Ok(paths);
    }
    for entry in fs::read_dir(path)? {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_symlink()
            && metadata.is_file()
            && path.extension().is_some_and(|value| value == extension)
        {
            paths.push(path);
            ensure!(paths.len() <= MAX_INPUTS, "Sleep scan limit exceeded");
        }
    }
    paths.sort();
    Ok(paths)
}

fn memory_files(store: &Store) -> Result<Vec<PathBuf>> {
    let mut paths = bounded_files_in(&store.working_dir(), "md")?;
    let archive = store.archive_dir();
    if directory(&archive)? {
        for month in fs::read_dir(archive)? {
            let month = month?.path();
            if directory(&month)? {
                paths.extend(bounded_files_in(&month, "md")?);
                ensure!(paths.len() <= MAX_INPUTS, "Sleep scan limit exceeded");
            }
        }
    }
    paths.sort();
    Ok(paths)
}

fn read_bounded(path: &Path, total: &mut u64) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "invalid sleep input file"
    );
    ensure!(
        metadata.len() <= MAX_RECORD_BYTES as u64,
        "Sleep input record too large"
    );
    *total = total
        .checked_add(metadata.len())
        .context("Sleep input size overflow")?;
    ensure!(*total <= MAX_SCAN_BYTES, "Sleep input byte limit exceeded");
    crate::input::read_bytes(File::open(path)?, MAX_RECORD_BYTES)
}

fn inventory_snapshot(
    paths: &[PathBuf],
    checkpoint_count: usize,
    store_id: &str,
) -> Result<String> {
    let mut fingerprint = Sha256::new();
    fingerprint.update(store_id.as_bytes());
    for (index, path) in paths.iter().enumerate() {
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "invalid sleep input file"
        );
        ensure!(
            metadata.len() <= MAX_RECORD_BYTES as u64,
            "Sleep input record too large"
        );
        let name = path.as_os_str().as_encoded_bytes();
        fingerprint.update((name.len() as u64).to_le_bytes());
        fingerprint.update(name);
        fingerprint.update([u8::from(index >= checkpoint_count)]);
        fingerprint.update(metadata.len().to_le_bytes());
        let (before_epoch, modified) = match metadata.modified()?.duration_since(UNIX_EPOCH) {
            Ok(duration) => (false, duration),
            Err(error) => (true, error.duration()),
        };
        fingerprint.update([u8::from(before_epoch)]);
        fingerprint.update(modified.as_secs().to_le_bytes());
        fingerprint.update(modified.subsec_nanos().to_le_bytes());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            fingerprint.update(metadata.dev().to_le_bytes());
            fingerprint.update(metadata.ino().to_le_bytes());
            fingerprint.update(metadata.ctime().to_le_bytes());
            fingerprint.update(metadata.ctime_nsec().to_le_bytes());
        }
    }
    Ok(format!("{:x}", fingerprint.finalize()))
}

/// Cursor is bound to the bounded file inventory, including file change times.
pub fn export(store: &Store, cursor: usize, expected: Option<&str>, limit: usize) -> Result<Batch> {
    ensure!((1..=100).contains(&limit), "Limit must be 1..100");
    // The regular lock observes every memory and evidence file; observe only this page below.
    let _lock = lock_store_read_only(store)?;
    crate::relations::recover_pending(store)?;
    let manifest = read_manifest(store)?.context("Upgrade required")?;
    let mut paths = bounded_files_in(&store.root.join("checkpoints"), "json")?;
    let mut memories = memory_files(store)?;
    memories.sort_by(|left, right| {
        left.file_stem()
            .cmp(&right.file_stem())
            .then_with(|| left.cmp(right))
    });
    ensure!(
        paths.len() + memories.len() <= MAX_INPUTS,
        "Sleep scan limit exceeded"
    );
    let checkpoint_count = paths.len();
    paths.extend(memories);
    let snapshot = inventory_snapshot(&paths, checkpoint_count, &manifest.store_id)?;
    ensure!(
        cursor == 0 || expected == Some(snapshot.as_str()),
        "STALE_CURSOR: restart at zero"
    );
    ensure!(cursor <= paths.len(), "Invalid cursor");
    let ids: HashSet<_> = paths[checkpoint_count..]
        .iter()
        .filter_map(|path| path.file_stem().and_then(|id| id.to_str()))
        .collect();
    let mut total = 0_u64;
    let mut inputs = Vec::new();
    let mut issues = Vec::new();
    let mut observed = false;
    for (index, path) in paths.iter().enumerate().skip(cursor).take(limit) {
        let size = fs::symlink_metadata(path)?.len();
        if total.saturating_add(size) > MAX_SCAN_BYTES {
            break;
        }
        if index < checkpoint_count {
            let checkpoint: Checkpoint = serde_json::from_slice(&read_bounded(path, &mut total)?)?;
            ensure!(
                checkpoint.schema_version == 1
                    && checkpoint.store_id == manifest.store_id
                    && path.file_stem().and_then(|value| value.to_str())
                        == Some(checkpoint.id.as_str())
                    && uuid::Uuid::parse_str(&checkpoint.id)
                        .is_ok_and(|value| value.to_string() == checkpoint.id),
                "invalid checkpoint input"
            );
            inputs.push(Input {
                kind: "checkpoint".into(),
                memory_ref: MemoryRef {
                    store_id: manifest.store_id.clone(),
                    memory_id: format!("checkpoint:{}", checkpoint.id),
                },
                revision: checkpoint.revision,
                semantic_hash: hash(&json!({
                    "id": checkpoint.id,
                    "revision": checkpoint.revision,
                    "state": checkpoint.state,
                    "updated_at": checkpoint.updated_at,
                }))?,
            });
        } else {
            observed |=
                crate::revisions::observe_memory(store, path, &crate::provenance::SystemClock)?;
            let memory: Memory =
                parse_memory(&String::from_utf8(read_bounded(path, &mut total)?)?)?;
            ensure!(
                path.file_stem().and_then(|id| id.to_str()) == Some(memory.id.as_str()),
                "invalid memory input"
            );
            let revision = crate::revisions::snapshot(store, path)?;
            let input = Input {
                kind: memory_kind(),
                memory_ref: MemoryRef {
                    store_id: manifest.store_id.clone(),
                    memory_id: memory.id,
                },
                revision: revision.semantic_rev,
                semantic_hash: revision.semantic_hash,
            };
            for link in &memory.links {
                if !ids.contains(link.id.as_str()) && issues.len() < MAX_ISSUES {
                    issues.push(json!({"kind":"unresolved_link","memory_ref":input.memory_ref}));
                }
            }
            inputs.push(input);
        }
    }
    ensure!(
        !inputs.is_empty() || cursor == paths.len(),
        "Sleep page cannot make progress"
    );
    let end = cursor + inputs.len();
    let snapshot = if observed {
        inventory_snapshot(&paths, checkpoint_count, &manifest.store_id)?
    } else {
        snapshot
    };
    Ok(Batch {
        version: 1,
        snapshot,
        cursor,
        next_cursor: end,
        partial: end < paths.len(),
        inputs,
        issues,
    })
}

fn report_path(store: &Store, key: &str) -> Result<PathBuf> {
    ensure!(
        key == "cursor" || (key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit())),
        "invalid sleep report key"
    );
    ensure!(
        !store.root.join("sleep").is_symlink(),
        "Symlink sleep directory"
    );
    Ok(store.root.join("sleep").join(format!("{key}.json")))
}

pub fn finish(
    store: &Store,
    batch: &Batch,
    requests: Vec<crate::proposals::Request>,
    clock: &impl Clock,
) -> Result<Value> {
    ensure!(
        batch.version == 1
            && requests.len() <= MAX_PROPOSALS
            && serde_json::to_vec(&requests)?.len() <= MAX_REPORT_BYTES,
        "Sleep output limit exceeded"
    );
    let current = export(
        store,
        batch.cursor,
        Some(&batch.snapshot),
        (batch
            .next_cursor
            .checked_sub(batch.cursor)
            .context("Invalid cursor range")?)
        .max(1),
    )?;
    ensure!(hash(&current)? == hash(batch)?, "STALE_SLEEP_INPUT");
    // Validate the entire batch before any proposal. Failed partial publication can
    // replay safely because proposal IDs are hashes of structured requests.
    for request in &requests {
        ensure!(
            [
                "REFINE",
                "CONTEXTUALIZE",
                "CONTRADICT",
                "SUPERSEDE",
                "CAUSED_BY",
            ]
            .contains(&request.decision.as_str()),
            "Invalid decision"
        );
        ensure!(
            !request.targets.is_empty()
                && request
                    .targets
                    .iter()
                    .all(
                        |target| batch.inputs.iter().any(|input| input.kind == "memory"
                            && input.memory_ref == target.memory_ref
                            && input.revision == target.expected_rev
                            && input.semantic_hash == target.expected_hash)
                    ),
            "Proposal target outside input snapshot"
        );
    }
    let mut proposal_ids = Vec::new();
    for request in requests {
        proposal_ids.push(crate::proposals::propose(store, request, clock)?.id);
    }
    let report = json!({
        "version": 1,
        "batch": batch,
        "proposal_ids": proposal_ids,
        "mode": "offline_or_host",
        "facts_modified": false,
    });
    ensure!(
        serde_json::to_vec(&report)?.len() <= MAX_REPORT_BYTES,
        "Report budget exceeded"
    );
    let _lock = lock_store_read_only(store)?;
    let key = hash(batch)?;
    crate::provenance::atomic_json(&report_path(store, &key)?, &report)?;
    crate::provenance::atomic_json(
        &report_path(store, "cursor")?,
        &json!({"snapshot": batch.snapshot, "next_cursor": batch.next_cursor}),
    )?;
    Ok(report)
}

pub fn rules(
    store: &Store,
    cursor: usize,
    snapshot: Option<&str>,
    limit: usize,
    clock: &impl Clock,
) -> Result<Value> {
    let batch = export(store, cursor, snapshot, limit)?;
    finish(store, &batch, vec![], clock)
}

pub fn cursor(store: &Store) -> Result<Value> {
    let path = report_path(store, "cursor")?;
    ensure!(!path.is_symlink(), "Symlink cursor");
    if !path.exists() {
        return Ok(json!({"next_cursor": 0}));
    }
    let value: Value = serde_json::from_slice(&crate::input::read_bytes(File::open(path)?, 4096)?)?;
    ensure!(
        value["next_cursor"].is_u64()
            && value["snapshot"]
                .as_str()
                .is_some_and(|value| value.len() == 64),
        "invalid sleep cursor"
    );
    Ok(value)
}
