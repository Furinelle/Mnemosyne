//! Explicit v2 store identity and source-event ledger.
use crate::{
    schema::{Memory, parse_memory},
    store::{
        Store, ensure_store, load_memories_unlocked, lock_store, memory_type_allowed, working_path,
    },
};
use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const SCHEMA: u32 = 2;
pub const WRITER_VERSION: u32 = 3;
const MAX_EVIDENCE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoreManifest {
    pub schema_version: u32,
    pub store_id: String,
    pub min_writer_version: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryRef {
    pub store_id: String,
    pub memory_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceEvent {
    pub origin: String,
    pub source_session_id: String,
    pub source_event_id: String,
    pub finding_key: String,
    pub source_kind: String,
    pub verification_state: String,
    pub content_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redacted_summary: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct WriteRequestV2 {
    #[serde(rename = "type")]
    pub memory_type: String,
    pub title: String,
    pub content: String,
    pub importance: i64,
    pub tags: Vec<String>,
    pub source: String,
    pub expires: String,
    pub origin: String,
    pub source_session_id: String,
    pub source_event_id: String,
    pub finding_key: String,
    pub source_kind: String,
    pub verification_state: String,
    pub fact_key: String,
    pub source_summary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WriteOutcome {
    pub status: String,
    pub memory_ref: MemoryRef,
    pub source_event: SourceEvent,
    pub evidence_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct UpgradePreview {
    pub status: String,
    pub manifest: Option<StoreManifest>,
    pub legacy_memories: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProvenanceView {
    pub source_kind: String,
    pub verification_state: String,
    pub source_events: Vec<SourceEvent>,
    pub evidence_count: usize,
}

pub trait Clock {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct EvidenceFile {
    schema_version: u32,
    memory: Memory,
    markdown_published: bool,
    fact_key: String,
    events: Vec<SourceEvent>,
}

fn checked_file(path: &Path) -> Result<()> {
    if path.is_symlink() {
        bail!("symlink is not allowed: {}", path.display());
    }
    Ok(())
}

pub(crate) fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().context("JSON path has no parent")?;
    ensure!(!parent.is_symlink(), "symlink directory is not allowed");
    checked_file(path)?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temp, value)?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    checked_file(path)?;
    temp.persist(path)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

pub fn read_manifest(store: &Store) -> Result<Option<StoreManifest>> {
    let path = store.root.join("store.json");
    checked_file(&path)?;
    let text = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let manifest: StoreManifest = serde_json::from_str(&text)?;
    uuid::Uuid::parse_str(&manifest.store_id).context("invalid store ID")?;
    Ok(Some(manifest))
}

fn require_v2(store: &Store) -> Result<StoreManifest> {
    let manifest = read_manifest(store)?.context("store needs explicit v2 upgrade")?;
    ensure!(
        manifest.schema_version == SCHEMA && manifest.min_writer_version <= WRITER_VERSION,
        "INCOMPATIBLE_SCHEMA"
    );
    ensure!(
        manifest.min_writer_version == WRITER_VERSION,
        "UPGRADE_REQUIRED: run store-upgrade --commit for revision-aware writes"
    );
    Ok(manifest)
}

pub fn ensure_v1_writer_compatible(store: &Store) -> Result<()> {
    if let Some(manifest) = read_manifest(store)? {
        ensure!(
            manifest.min_writer_version <= 1,
            "INCOMPATIBLE_SCHEMA: v1 writer refused"
        );
    }
    Ok(())
}

pub fn preview_upgrade(store: &Store) -> Result<UpgradePreview> {
    let manifest = read_manifest(store)?;
    let status = match &manifest {
        None => "upgrade_available",
        Some(m) if m.schema_version == SCHEMA && m.min_writer_version < WRITER_VERSION => {
            "upgrade_available"
        }
        Some(m) if m.schema_version == SCHEMA && m.min_writer_version == WRITER_VERSION => {
            "current"
        }
        Some(_) => "incompatible",
    };
    let legacy_memories = if store.root.exists() {
        load_memories_unlocked(store, true)?.len()
    } else {
        0
    };
    Ok(UpgradePreview {
        status: status.into(),
        manifest,
        legacy_memories,
    })
}

pub fn upgrade_store(store: &Store) -> Result<StoreManifest> {
    ensure_store(store)?;
    let _guard = lock_store(store)?;
    if let Some(mut manifest) = read_manifest(store)? {
        ensure!(
            manifest.schema_version == SCHEMA && manifest.min_writer_version <= WRITER_VERSION,
            "INCOMPATIBLE_SCHEMA"
        );
        if manifest.min_writer_version < WRITER_VERSION {
            manifest.min_writer_version = WRITER_VERSION;
            atomic_json(&store.root.join("store.json"), &manifest)?;
        }
        return Ok(manifest);
    }
    let manifest = StoreManifest {
        schema_version: SCHEMA,
        store_id: uuid::Uuid::new_v4().to_string(),
        min_writer_version: WRITER_VERSION,
    };
    atomic_json(&store.root.join("store.json"), &manifest)?;
    Ok(manifest)
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn expired_at(value: &str, now: &DateTime<Utc>) -> bool {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok_and(|date| date < now.date_naive())
}
fn evidence_dir(store: &Store) -> Result<PathBuf> {
    let path = store.root.join("evidence");
    ensure!(
        !path.is_symlink(),
        "symlink evidence directory is not allowed"
    );
    Ok(path)
}
fn evidence_path(store: &Store, id: &str) -> Result<PathBuf> {
    ensure!(
        !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "invalid memory id"
    );
    Ok(evidence_dir(store)?.join(format!("{id}.json")))
}
fn all_evidence(store: &Store) -> Result<Vec<(PathBuf, EvidenceFile)>> {
    let dir = evidence_dir(store)?;
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        checked_file(&path)?;
        if path.extension().is_some_and(|e| e == "json") {
            let bytes = crate::input::read_bytes(fs::File::open(&path)?, MAX_EVIDENCE_BYTES)?;
            let file: EvidenceFile = serde_json::from_slice(&bytes)?;
            ensure!(file.schema_version == SCHEMA, "INCOMPATIBLE_SCHEMA");
            ensure!(
                path.file_stem().and_then(|s| s.to_str()) == Some(file.memory.id.as_str()),
                "IDENTITY_CONFLICT: evidence filename differs from memory ID"
            );
            ensure!(!file.events.is_empty(), "Invalid empty evidence ledger");
            out.push((path, file));
        }
    }
    Ok(out)
}
fn write_evidence(path: &Path, file: &EvidenceFile) -> Result<()> {
    ensure!(
        serde_json::to_vec_pretty(file)?.len() < MAX_EVIDENCE_BYTES,
        "EVIDENCE_TOO_LARGE"
    );
    atomic_json(path, file)
}
// Source ledger and its semantic projection share the relation mutation protocol.
fn commit_evidence(
    store: &Store,
    path: &Path,
    before: Option<String>,
    file: &mut EvidenceFile,
    expected: Option<crate::revisions::Snapshot>,
    clock: &impl Clock,
) -> Result<()> {
    file.memory.extra.insert(
        "evidence_count".into(),
        serde_json::json!(evidence_count(&file.events)),
    );
    file.memory.extra.insert(
        "source_event_count".into(),
        serde_json::json!(file.events.len()),
    );
    file.memory.extra.insert(
        "source_event_ledger_hash".into(),
        serde_json::json!(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&file.events)?)
        )),
    );
    file.markdown_published = true;
    let after = format!("{}\n", serde_json::to_string_pretty(file)?);
    ensure!(after.len() <= MAX_EVIDENCE_BYTES, "EVIDENCE_TOO_LARGE");
    let mut plan = crate::revisions::plan_updates(
        store,
        &[crate::revisions::RevisionUpdate {
            path: working_path(store, &file.memory)?,
            memory: file.memory.clone(),
            expected,
        }],
        "source_evidence",
        clock,
    )?;
    plan.changes.push(crate::relations::MutationChange {
        path: path.strip_prefix(&store.root)?.to_path_buf(),
        before,
        after: Some(after),
    });
    crate::relations::execute_mutation(store, plan)?;
    if let Err(error) = crate::api::update_markdown_index(store, None) {
        eprintln!("mnemosyne: derived MEMORY.md update failed: {error}");
    }
    Ok(())
}

fn repair_unpublished(store: &Store, files: &mut [(PathBuf, EvidenceFile)]) -> Result<()> {
    for (path, file) in files {
        if !file.markdown_published {
            let markdown = working_path(store, &file.memory)?;
            if markdown.exists() {
                let existing = crate::schema::parse_memory(&fs::read_to_string(&markdown)?)?;
                ensure!(
                    existing.id == file.memory.id && existing.body == file.memory.body,
                    "IDENTITY_CONFLICT: pending Markdown differs from sidecar"
                );
            } else {
                if read_manifest(store)?.is_some_and(|m| m.min_writer_version >= WRITER_VERSION) {
                    crate::revisions::write_locked(
                        store,
                        &markdown,
                        &file.memory,
                        None,
                        "created",
                        &SystemClock,
                    )?;
                } else {
                    crate::store::write_memory(&markdown, &file.memory)?;
                }
            }
            file.markdown_published = true;
            write_evidence(path, file)?;
            if let Err(error) = crate::api::update_markdown_index(store, None) {
                eprintln!("mnemosyne: derived MEMORY.md update failed: {error}");
            }
        }
    }
    Ok(())
}

/// Called with the store lock held, including by legacy readers.
pub(crate) fn recover_pending(store: &Store) -> Result<()> {
    // ponytail: scan sidecars under the existing lock; add a pending marker if reads slow down.
    let mut files = all_evidence(store)?;
    repair_unpublished(store, &mut files)
}

fn event_identity(event: &SourceEvent) -> (&str, &str, &str) {
    (
        &event.origin,
        &event.source_session_id,
        &event.source_event_id,
    )
}
fn evidence_count(events: &[SourceEvent]) -> usize {
    events
        .iter()
        .map(event_identity)
        .collect::<HashSet<_>>()
        .len()
}
fn outcome(
    status: &str,
    manifest: &StoreManifest,
    file: &EvidenceFile,
    event: SourceEvent,
) -> WriteOutcome {
    WriteOutcome {
        status: status.into(),
        memory_ref: MemoryRef {
            store_id: manifest.store_id.clone(),
            memory_id: file.memory.id.clone(),
        },
        source_event: event,
        evidence_count: evidence_count(&file.events),
    }
}

pub fn write_v2(
    store: &Store,
    request: &WriteRequestV2,
    clock: &impl Clock,
) -> Result<WriteOutcome> {
    ensure!(!request.content.trim().is_empty(), "No content provided");
    ensure!(
        !request.memory_type.is_empty()
            && request
                .memory_type
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "Invalid memory type"
    );
    for value in [
        &request.origin,
        &request.source_session_id,
        &request.source_event_id,
        &request.finding_key,
    ] {
        ensure!(
            !value.trim().is_empty() && value.len() <= 256,
            "origin/session/event/finding must be nonempty and at most 256 bytes"
        );
    }
    ensure!(
        matches!(
            request.source_kind.as_str(),
            "user_statement" | "tool_output" | "code_observation" | "agent_inference"
        ),
        "Invalid source kind"
    );
    ensure!(
        matches!(
            request.verification_state.as_str(),
            "unverified" | "evidence_attached" | "verified"
        ),
        "Invalid verification state"
    );
    let _guard = lock_store(store)?;
    let manifest = require_v2(store)?;
    let now = clock.now();
    ensure!(
        memory_type_allowed(store, &request.memory_type)?,
        "Invalid memory type for this store"
    );
    let identity = serde_json::to_vec(&(
        &manifest.store_id,
        &request.origin,
        &request.source_session_id,
        &request.source_event_id,
        &request.finding_key,
    ))?;
    let identity_hash = hash(&identity);
    // `source` names the retelling agent. The underlying origin/event is the identity.
    let payload_hash = hash(&serde_json::to_vec(&(
        &request.memory_type,
        &request.title,
        &request.content,
        request.importance,
        &request.tags,
        &request.expires,
        &request.fact_key,
        &request.source_kind,
        &request.verification_state,
        &request.source_summary,
    ))?);
    let event = SourceEvent {
        origin: request.origin.clone(),
        source_session_id: request.source_session_id.clone(),
        source_event_id: request.source_event_id.clone(),
        finding_key: request.finding_key.clone(),
        source_kind: request.source_kind.clone(),
        verification_state: if request.verification_state == "verified" {
            "unverified".into()
        } else {
            request.verification_state.clone()
        },
        content_hash: payload_hash,
        redacted_summary: (!request.source_summary.is_empty())
            .then(|| request.source_summary.chars().take(200).collect()),
    };
    let mut files = all_evidence(store)?;
    for (_, file) in &files {
        if let Some(previous) = file.events.iter().find(|old| {
            old.origin == event.origin
                && old.source_session_id == event.source_session_id
                && old.source_event_id == event.source_event_id
                && old.finding_key == event.finding_key
        }) {
            ensure!(
                previous.content_hash == event.content_hash,
                "IDENTITY_CONFLICT"
            );
            return Ok(outcome("duplicate", &manifest, file, previous.clone()));
        }
    }
    if !request.fact_key.is_empty() {
        ensure!(
            files
                .iter()
                .filter(|(_, file)| file.fact_key == request.fact_key)
                .count()
                <= 1,
            "IDENTITY_CONFLICT: fact key is ambiguous"
        );
        for (path, file) in &mut files {
            if file.fact_key == request.fact_key {
                let markdown = working_path(store, &file.memory)?;
                let current =
                    parse_memory(&fs::read_to_string(&markdown).context(
                        "IDENTITY_CONFLICT: fact Markdown is missing from working store",
                    )?)?;
                ensure!(
                    current.id == file.memory.id
                        && current.status == "active"
                        && !expired_at(&current.expires, &now)
                        && current
                            .extra
                            .get("invalidated_by")
                            .is_none_or(|v| v.as_str() == Some(""))
                        && current.extra.get("fact_key").and_then(|v| v.as_str())
                            == Some(request.fact_key.as_str())
                        && current.memory_type == request.memory_type
                        && current.body == file.memory.body
                        && current
                            .body
                            .split_once("\n\n")
                            .is_some_and(|(_, content)| content == request.content.trim()),
                    "IDENTITY_CONFLICT: fact Markdown changed or is inactive"
                );
                let before = fs::read_to_string(&*path)?;
                ensure!(
                    serde_json::from_str::<serde_json::Value>(&before)?
                        == serde_json::to_value(&*file)?,
                    "IDENTITY_CONFLICT: source ledger changed"
                );
                let expected = crate::revisions::snapshot(store, &markdown)?;
                ensure!(
                    expected.semantic_hash == crate::revisions::semantic_digest(&current),
                    "REVISION_CONFLICT: fact changed"
                );
                file.memory = current;
                file.events.push(event.clone());
                commit_evidence(store, path, Some(before), file, Some(expected), clock)?;
                return Ok(outcome("supported", &manifest, file, event));
            }
        }
    }
    let date = now.format("%Y-%m-%d").to_string();
    let title = if request.title.trim().is_empty() {
        request.content.lines().next().unwrap_or("").trim()
    } else {
        request.title.trim()
    };
    let body = format!(
        "## {title}\n\n{}",
        request.content.trim().replace("\r\n", "\n")
    );
    let summary = format!(
        "{title}: {}",
        request
            .content
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    )
    .chars()
    .take(220)
    .collect();
    let mut memory = Memory {
        id: format!("{}-{}", request.memory_type, &identity_hash[..20]),
        memory_type: request.memory_type.clone(),
        source: if request.source.trim().is_empty() {
            "agent".into()
        } else {
            request.source.trim().to_lowercase()
        },
        strength: request.importance.clamp(0, 100),
        created: date.clone(),
        last_accessed: date,
        tags: request.tags.clone(),
        canonical_summary: summary,
        injection_summary: String::new(),
        status: "active".into(),
        body,
        expires: request.expires.clone(),
        ..Default::default()
    };
    memory.injection_summary = memory.canonical_summary.clone();
    for (key, value) in [
        (
            "recorded_at",
            now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        ),
        ("source_event_id", request.source_event_id.clone()),
        ("source_session_id", request.source_session_id.clone()),
        ("source_kind", request.source_kind.clone()),
        ("verification_state", event.verification_state.clone()),
    ] {
        memory.extra.insert(key.into(), value.into());
    }
    if !request.fact_key.is_empty() {
        memory
            .extra
            .insert("fact_key".into(), request.fact_key.clone().into());
    }
    let mut file = EvidenceFile {
        schema_version: SCHEMA,
        memory,
        markdown_published: false,
        fact_key: request.fact_key.clone(),
        events: vec![event.clone()],
    };
    let path = evidence_path(store, &file.memory.id)?;
    ensure!(!path.exists(), "IDENTITY_CONFLICT: memory ID collision");
    let markdown = working_path(store, &file.memory)?;
    ensure!(!markdown.exists(), "IDENTITY_CONFLICT: memory ID collision");
    commit_evidence(store, &path, None, &mut file, None, clock)?;
    Ok(outcome("created", &manifest, &file, event))
}

pub fn resolve_ref(
    stores: &[Store],
    reference: &MemoryRef,
) -> Result<Option<(Store, PathBuf, Memory)>> {
    let mut matching = Vec::new();
    for store in stores {
        if let Some(manifest) = read_manifest(store)?
            && manifest.store_id == reference.store_id
        {
            matching.push(store);
        }
    }
    ensure!(matching.len() <= 1, "AMBIGUOUS_STORE");
    let Some(store) = matching.first() else {
        return Ok(None);
    };
    let _guard = lock_store(store)?;
    Ok(load_memories_unlocked(store, true)?
        .into_iter()
        .find(|(_, m)| m.id == reference.memory_id)
        .map(|(p, m)| ((*store).clone(), p, m)))
}

pub fn resolve_id_v2(
    stores: &[Store],
    memory_id: &str,
    store_id: Option<&str>,
) -> Result<Option<(Store, PathBuf, Memory)>> {
    let mut found = None;
    for store in stores {
        let Some(manifest) = read_manifest(store)? else {
            continue;
        };
        if store_id.is_some_and(|id| id != manifest.store_id) {
            continue;
        }
        if let Some(candidate) = resolve_ref(
            std::slice::from_ref(store),
            &MemoryRef {
                store_id: manifest.store_id,
                memory_id: memory_id.into(),
            },
        )? {
            ensure!(found.is_none(), "AMBIGUOUS_MEMORY_ID");
            found = Some(candidate);
        }
    }
    Ok(found)
}

pub fn read_provenance(store: &Store, memory_id: &str) -> Result<ProvenanceView> {
    if !store.root.exists() {
        return Ok(ProvenanceView {
            source_kind: "unknown".into(),
            verification_state: "unknown".into(),
            source_events: vec![],
            evidence_count: 0,
        });
    }
    let _guard = lock_store(store)?;
    read_provenance_unlocked(store, memory_id)
}

/// Caller holds the store lock; use with a matching Markdown/revision snapshot.
pub(crate) fn read_provenance_unlocked(store: &Store, memory_id: &str) -> Result<ProvenanceView> {
    let files = all_evidence(store)?;
    let Some((_, file)) = files.into_iter().find(|(_, f)| f.memory.id == memory_id) else {
        return Ok(ProvenanceView {
            source_kind: "unknown".into(),
            verification_state: "unknown".into(),
            source_events: vec![],
            evidence_count: 0,
        });
    };
    Ok(ProvenanceView {
        source_kind: file
            .events
            .first()
            .map_or("unknown", |e| &e.source_kind)
            .into(),
        verification_state: file
            .events
            .first()
            .map_or("unknown", |e| &e.verification_state)
            .into(),
        evidence_count: evidence_count(&file.events),
        source_events: file.events,
    })
}
