//! Semantic revisions backed by the existing recoverable mutation journal.
use crate::{
    provenance::{Clock, read_manifest},
    relations::{MutationChange, MutationPlan, execute_mutation},
    schema::{Memory, parse_memory, serialize_memory},
    store::{Store, load_memories_unlocked},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

const MAX_MEMORY: u64 = 8 * 1024 * 1024;
const MAX_MANIFEST: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub semantic_rev: u64,
    pub semantic_hash: String,
    pub raw_hash: Option<String>,
    #[serde(skip)]
    strength: i64,
    #[serde(skip)]
    last_accessed: String,
    #[serde(skip)]
    access_count: i64,
}

#[derive(Clone, Debug)]
pub struct RevisionUpdate {
    pub path: PathBuf,
    pub memory: Memory,
    pub expected: Option<Snapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RevisionEntry {
    pub revision: u64,
    pub recorded_at: String,
    pub reason: String,
    pub semantic_hash: String,
    pub raw_hash: String,
    pub unknown_gap: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryManifest {
    pub version: u32,
    pub memory_id: String,
    pub coverage_start: String,
    pub entries: Vec<RevisionEntry>,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Knowledge content only; access/strength bookkeeping cannot advance a revision.
pub fn semantic_digest(memory: &Memory) -> String {
    let mut extra = memory.extra.clone();
    extra.remove("semantic_rev");
    extra.remove("last_maintained_at");
    let value = json!({
        "id": memory.id, "type": memory.memory_type, "source": memory.source,
        "created": memory.created, "tags": memory.tags, "links": memory.links,
        "canonical_summary": memory.canonical_summary,
        "injection_summary": memory.injection_summary, "status": memory.status,
        "expires": memory.expires, "body": memory.body, "extra": extra,
    });
    hash(&serde_json::to_vec(&value).expect("Memory is JSON serializable"))
}

fn history_enabled(store: &Store) -> Result<bool> {
    match read_manifest(store)? {
        None => Ok(false),
        Some(manifest) => {
            ensure!(
                manifest.schema_version == 2
                    && manifest.min_writer_version <= crate::provenance::WRITER_VERSION,
                "INCOMPATIBLE_SCHEMA"
            );
            Ok(manifest.min_writer_version == crate::provenance::WRITER_VERSION)
        }
    }
}

pub(crate) fn safe_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "invalid memory ID"
    );
    Ok(())
}

fn checked_memory_path(store: &Store, path: &Path) -> Result<PathBuf> {
    let relative = path.strip_prefix(&store.root).unwrap_or(path);
    ensure!(
        relative
            .components()
            .all(|c| matches!(c, Component::Normal(_))),
        "unsafe memory path"
    );
    let parts: Vec<_> = relative.components().collect();
    ensure!(
        (parts.len() == 2 && parts[0].as_os_str() == "working")
            || (parts.len() == 3 && parts[0].as_os_str() == "archive"),
        "memory path outside working/archive"
    );
    ensure!(
        relative.extension().is_some_and(|ext| ext == "md"),
        "invalid memory extension"
    );
    let mut component = store.root.clone();
    ensure!(
        !fs::symlink_metadata(&component)?.file_type().is_symlink(),
        "symlink store root"
    );
    for part in relative.components() {
        component.push(part.as_os_str());
        if let Ok(meta) = fs::symlink_metadata(&component) {
            ensure!(!meta.file_type().is_symlink(), "symlink memory path");
        }
    }
    Ok(relative.to_path_buf())
}

fn relative(store: &Store, path: &Path, id: &str) -> Result<PathBuf> {
    safe_id(id)?;
    let relative = checked_memory_path(store, path)?;
    ensure!(
        relative.file_name().and_then(|n| n.to_str()) == Some(format!("{id}.md").as_str()),
        "memory path and ID differ"
    );
    Ok(relative)
}

fn history_paths(id: &str) -> Result<(PathBuf, PathBuf)> {
    safe_id(id)?;
    let dir = PathBuf::from("history").join(id);
    Ok((dir.clone(), dir.join("manifest.json")))
}

fn checked_history(store: &Store, path: &Path) -> Result<()> {
    let mut component = store.root.clone();
    for part in path.components() {
        component.push(part.as_os_str());
        match fs::symlink_metadata(&component) {
            Ok(meta) => ensure!(!meta.file_type().is_symlink(), "symlink history path"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn read_limited(path: &Path, limit: u64) -> Result<Option<String>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(
                meta.is_file() && meta.len() <= limit,
                "invalid or oversized revision file"
            );
            let bytes = crate::input::read_bytes(fs::File::open(path)?, limit as usize)?;
            Ok(Some(String::from_utf8(bytes)?))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn read_history(
    store: &Store,
    id: &str,
) -> Result<(Option<String>, Option<HistoryManifest>)> {
    let (dir, path) = history_paths(id)?;
    checked_history(store, &path)?;
    let raw = read_limited(&store.root.join(&path), MAX_MANIFEST)?;
    let Some(text) = &raw else {
        return Ok((None, None));
    };
    let manifest: HistoryManifest = serde_json::from_str(text)?;
    ensure!(
        manifest.version == 1 && manifest.memory_id == id && !manifest.entries.is_empty(),
        "invalid history manifest"
    );
    // ponytail: full image validation costs O(history) per read; add a trusted
    // index only if measured store size makes this a real bottleneck.
    for (i, entry) in manifest.entries.iter().enumerate() {
        ensure!(
            entry.revision == i as u64 + 1,
            "non-monotonic history revision"
        );
        let image = dir.join(format!("{}.md", entry.revision));
        checked_history(store, &image)?;
        let content =
            read_limited(&store.root.join(image), MAX_MEMORY)?.context("missing history image")?;
        ensure!(
            hash(content.as_bytes()) == entry.raw_hash,
            "corrupt history image"
        );
        ensure!(
            semantic_digest(&parse_memory(&content)?) == entry.semantic_hash,
            "corrupt history semantics"
        );
    }
    Ok((raw, Some(manifest)))
}

fn disk_memory(store: &Store, path: &Path) -> Result<Option<(PathBuf, String, Memory)>> {
    let path = checked_memory_path(store, path)?;
    let absolute = store.root.join(&path);
    let raw = read_limited(&absolute, MAX_MEMORY)?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let memory = parse_memory(&raw)?;
    let relative = relative(store, &path, &memory.id)?;
    Ok(Some((relative, raw, memory)))
}

/// Verified historical image; never recover or observe current records here.
pub(crate) fn history_image(store: &Store, id: &str, entry: &RevisionEntry) -> Result<Memory> {
    let (dir, _) = history_paths(id)?;
    let path = dir.join(format!("{}.md", entry.revision));
    checked_history(store, &path)?;
    let raw = read_limited(&store.root.join(path), MAX_MEMORY)?.context("Missing history image")?;
    let memory = parse_memory(&raw)?;
    ensure!(
        memory.id == id
            && hash(raw.as_bytes()) == entry.raw_hash
            && semantic_digest(&memory) == entry.semantic_hash,
        "Corrupt history image"
    );
    Ok(memory)
}

/// Read-only snapshot under a store lock. Observation should run first on writable locks.
pub fn snapshot(store: &Store, path: &Path) -> Result<Snapshot> {
    let (_, raw, memory) = disk_memory(store, path)?.context("memory missing")?;
    let (_, history) = read_history(store, &memory.id)?;
    let semantic_hash = semantic_digest(&memory);
    let semantic_rev = history.as_ref().map_or(0, |h| h.entries.len() as u64);
    if let Some(latest) = history.as_ref().and_then(|h| h.entries.last()) {
        ensure!(
            latest.semantic_hash == semantic_hash,
            "UNOBSERVED_EXTERNAL_EDIT"
        );
    }
    Ok(Snapshot {
        semantic_rev,
        semantic_hash,
        raw_hash: Some(hash(raw.as_bytes())),
        strength: memory.strength,
        last_accessed: memory.last_accessed,
        access_count: memory.access_count,
    })
}

fn add_entry(
    store: &Store,
    plan: &mut MutationPlan,
    manifest: &mut HistoryManifest,
    raw: String,
    reason: &str,
    now: &str,
    unknown_gap: bool,
) -> Result<()> {
    let revision = manifest.entries.len() as u64 + 1;
    let path = PathBuf::from("history")
        .join(&manifest.memory_id)
        .join(format!("{revision}.md"));
    checked_history(store, &path)?;
    ensure!(
        read_limited(&store.root.join(&path), MAX_MEMORY)?.is_none(),
        "history image already exists"
    );
    let parsed = parse_memory(&raw)?;
    ensure!(parsed.id == manifest.memory_id, "history image ID mismatch");
    ensure!(raw.len() as u64 <= MAX_MEMORY, "history image too large");
    manifest.entries.push(RevisionEntry {
        revision,
        recorded_at: now.into(),
        reason: reason.into(),
        semantic_hash: semantic_digest(&parsed),
        raw_hash: hash(raw.as_bytes()),
        unknown_gap,
    });
    plan.changes.push(MutationChange {
        path,
        before: None,
        after: Some(raw),
    });
    Ok(())
}

fn manifest_image(manifest: &HistoryManifest) -> Result<String> {
    let image = format!("{}\n", serde_json::to_string_pretty(manifest)?);
    ensure!(
        image.len() as u64 <= MAX_MANIFEST,
        "history manifest too large"
    );
    Ok(image)
}

/// Caller holds the writable store lock. Plans include exact disk CAS images.
pub(crate) fn plan_updates(
    store: &Store,
    updates: &[RevisionUpdate],
    reason: &str,
    clock: &impl Clock,
) -> Result<MutationPlan> {
    let enabled = history_enabled(store)?;
    ensure!(
        enabled || read_manifest(store)?.is_none(),
        "UPGRADE_REQUIRED: mutation needs revision-aware writer"
    );
    let now = clock.now().to_rfc3339();
    let mut plan = MutationPlan {
        changes: Vec::new(),
    };
    let mut seen = std::collections::HashSet::new();
    for update in updates {
        let path = relative(store, &update.path, &update.memory.id)?;
        ensure!(
            seen.insert(update.memory.id.clone()),
            "duplicate revision target"
        );
        let current = disk_memory(store, &path)?;
        let old_raw = current.as_ref().map(|(_, raw, _)| raw.clone());
        let old = current.as_ref().map(|(_, _, memory)| memory);
        if let Some(old) = old {
            ensure!(old.id == update.memory.id, "memory ID changed");
        }
        let (old_manifest_raw, mut history) = if enabled {
            read_history(store, &update.memory.id)?
        } else {
            (None, None)
        };
        let current_hash = old.map(semantic_digest);
        let current_rev = history.as_ref().map_or(0, |h| h.entries.len() as u64);
        if let Some(expected) = &update.expected {
            ensure!(
                old.is_some()
                    && current_hash.as_deref() == Some(&expected.semantic_hash)
                    && current_rev == expected.semantic_rev,
                "VERSION_CONFLICT: semantic revision changed"
            );
        } else {
            ensure!(old.is_none(), "VERSION_CONFLICT: memory already exists");
        }
        let mut desired = update.memory.clone();
        if let (Some(expected), Some(current)) = (&update.expected, old) {
            if desired.strength == expected.strength {
                desired.strength = current.strength;
            }
            if desired.last_accessed == expected.last_accessed {
                desired.last_accessed = current.last_accessed.clone();
            }
            if desired.access_count == expected.access_count {
                desired.access_count = current.access_count;
            }
        }
        if enabled {
            let (dir, manifest_path) = history_paths(&desired.id)?;
            let mut manifest = history.take().unwrap_or_else(|| HistoryManifest {
                version: 1,
                memory_id: desired.id.clone(),
                coverage_start: now.clone(),
                entries: vec![],
            });
            if let Some(raw) = &old_raw {
                let known = manifest.entries.last();
                if known.is_none() {
                    add_entry(
                        store,
                        &mut plan,
                        &mut manifest,
                        raw.clone(),
                        "adopted",
                        &now,
                        false,
                    )?;
                } else if known
                    .is_some_and(|e| current_hash.as_deref() != Some(e.semantic_hash.as_str()))
                {
                    add_entry(
                        store,
                        &mut plan,
                        &mut manifest,
                        raw.clone(),
                        "external_edit",
                        &now,
                        true,
                    )?;
                }
            }
            let target_hash = semantic_digest(&desired);
            let current_semantic = old.map(semantic_digest);
            if old.is_none() || current_semantic.as_deref() != Some(target_hash.as_str()) {
                let next = manifest.entries.len() as u64 + 1;
                desired
                    .extra
                    .insert("semantic_rev".into(), Value::from(next));
                let after = serialize_memory(&desired);
                add_entry(
                    store,
                    &mut plan,
                    &mut manifest,
                    after.clone(),
                    if old.is_none() { "created" } else { reason },
                    &now,
                    false,
                )?;
                plan.changes.push(MutationChange {
                    path,
                    before: old_raw,
                    after: Some(after),
                });
            } else {
                // Heat-only writes carry the latest semantic_rev without adding history.
                desired.extra.insert(
                    "semantic_rev".into(),
                    Value::from(manifest.entries.len() as u64),
                );
                let after = serialize_memory(&desired);
                ensure!(after.len() as u64 <= MAX_MEMORY, "memory image too large");
                if old_raw.as_deref() != Some(after.as_str()) {
                    plan.changes.push(MutationChange {
                        path,
                        before: old_raw,
                        after: Some(after),
                    });
                }
            }
            let manifest_after = manifest_image(&manifest)?;
            if old_manifest_raw.as_deref() != Some(manifest_after.as_str()) {
                checked_history(store, &dir)?;
                plan.changes.push(MutationChange {
                    path: manifest_path,
                    before: old_manifest_raw,
                    after: Some(manifest_after),
                });
            }
        } else {
            let after = serialize_memory(&desired);
            ensure!(after.len() as u64 <= MAX_MEMORY, "memory image too large");
            if old_raw.as_deref() != Some(after.as_str()) {
                plan.changes.push(MutationChange {
                    path,
                    before: old_raw,
                    after: Some(after),
                });
            }
        }
    }
    Ok(plan)
}

/// Commit one memory update while the caller holds the writable store lock.
pub fn write_locked(
    store: &Store,
    path: &Path,
    memory: &Memory,
    expected: Option<Snapshot>,
    reason: &str,
    clock: &impl Clock,
) -> Result<()> {
    let plan = plan_updates(
        store,
        &[RevisionUpdate {
            path: path.to_path_buf(),
            memory: memory.clone(),
            expected,
        }],
        reason,
        clock,
    )?;
    if !plan.changes.is_empty() {
        execute_mutation(store, plan)?;
    }
    Ok(())
}

/// Adopt current raw Markdown, or record the first observed external semantic edit.
/// This never rewrites current Markdown and never guesses past versions or edit times.
pub fn observe_store(store: &Store, clock: &impl Clock) -> Result<()> {
    if !history_enabled(store)? {
        return Ok(());
    }
    for (path, _) in load_memories_unlocked(store, true)? {
        observe_memory_enabled(store, &path, clock)?;
    }
    Ok(())
}

/// Observe one memory under the store lock. Returns whether canonical metadata changed.
pub fn observe_memory(store: &Store, path: &Path, clock: &impl Clock) -> Result<bool> {
    if !history_enabled(store)? {
        return Ok(false);
    }
    observe_memory_enabled(store, path, clock)
}

fn observe_memory_enabled(store: &Store, path: &Path, clock: &impl Clock) -> Result<bool> {
    let Some((_, raw, memory)) = disk_memory(store, path)? else {
        return Ok(false);
    };
    let (manifest_raw, manifest) = read_history(store, &memory.id)?;
    if manifest
        .as_ref()
        .and_then(|h| h.entries.last())
        .is_some_and(|e| e.semantic_hash == semantic_digest(&memory))
    {
        return Ok(false);
    }
    let now = clock.now().to_rfc3339();
    let mut manifest = manifest.unwrap_or_else(|| HistoryManifest {
        version: 1,
        memory_id: memory.id.clone(),
        coverage_start: now.clone(),
        entries: vec![],
    });
    let reason = if manifest.entries.is_empty() {
        "adopted"
    } else {
        "external_edit"
    };
    let gap = !manifest.entries.is_empty();
    let mut plan = MutationPlan {
        changes: vec![MutationChange {
            path: relative(store, path, &memory.id)?,
            before: Some(raw.clone()),
            after: Some(raw.clone()),
        }],
    };
    add_entry(store, &mut plan, &mut manifest, raw, reason, &now, gap)?;
    let (_, path) = history_paths(&memory.id)?;
    plan.changes.push(MutationChange {
        path,
        before: manifest_raw,
        after: Some(manifest_image(&manifest)?),
    });
    execute_mutation(store, plan)?;
    Ok(true)
}
