//! Typed relations and recoverable two-file mutations across local stores.
use crate::{
    provenance::{SystemClock, read_manifest},
    revisions::{RevisionUpdate, plan_updates, semantic_digest, snapshot},
    schema::{Link, Memory, serialize_memory},
    store::{
        Store, global_store, load_config, load_memories_unlocked, lock_store, lock_store_read_only,
    },
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};

const JOURNAL: &str = ".relations-operation.json";
const COMMITS: &str = ".relations-commits";
const MAX_MUTATION_IMAGE: u64 = 8 * 1024 * 1024;
const MAX_JOURNAL: u64 = 64 * 1024 * 1024;

pub fn reverse(rel: &str) -> Option<&'static str> {
    match rel {
        "caused_by" => Some("causes"),
        "refines" => Some("refined_by"),
        "supersedes" => Some("superseded_by"),
        "contradicts" => Some("contradicts"),
        "related" => Some("related"),
        _ => None,
    }
}
pub fn weight(rel: &str) -> f64 {
    match rel {
        "caused_by" => 0.6,
        "refines" => 0.7,
        "supersedes" => 0.3,
        _ => 0.5,
    }
}
pub fn is_demoting(rel: &str) -> bool {
    rel == "supersedes"
}
pub fn is_symmetric(rel: &str) -> bool {
    matches!(rel, "contradicts" | "related")
}
pub fn warns(rel: &str) -> bool {
    rel == "contradicts"
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

#[derive(Clone, Serialize, Deserialize)]
struct Change {
    path: PathBuf,
    before: String,
    after: String,
    before_hash: String,
    after_hash: String,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    version: u32,
    changes: Vec<Change>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    coordinator_key: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct CommitDecision {
    version: u32,
    participants: BTreeMap<String, String>,
}

/// A single-store, byte-level mutation. The caller holds the store lock and
/// supplies the exact current image (or absence) for every target.
pub(crate) struct MutationChange {
    pub path: PathBuf,
    pub before: Option<String>,
    pub after: Option<String>,
}

pub(crate) struct MutationPlan {
    pub changes: Vec<MutationChange>,
}

#[derive(Clone, Serialize, Deserialize)]
struct MutationRecord {
    path: PathBuf,
    before: Option<String>,
    after: Option<String>,
    before_hash: Option<String>,
    after_hash: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct MutationJournal {
    version: u32,
    changes: Vec<MutationRecord>,
    committed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    coordinator_key: Option<String>,
}

fn store_key(store: &Store) -> Result<String> {
    let root = fs::canonicalize(&store.root)?;
    Ok(digest(root.to_str().context("Store path is not UTF-8")?))
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => ensure!(
            !meta.file_type().is_symlink(),
            "Symlink in relation coordinator"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

// The coordinator comes only from trusted host configuration, never a journal
// path. The journal can select only a canonical UUID within this fixed directory.
fn decision_path(coordinator: &Store, operation_id: &str) -> Result<PathBuf> {
    ensure!(
        uuid::Uuid::parse_str(operation_id)?.to_string() == operation_id,
        "Invalid relation operation ID"
    );
    reject_symlink(&coordinator.root)?;
    let directory = coordinator.root.join(COMMITS);
    reject_symlink(&directory)?;
    let path = directory.join(format!("{operation_id}.json"));
    reject_symlink(&path)?;
    Ok(path)
}

fn prepare_coordinator(coordinator: &Store) -> Result<()> {
    reject_symlink(&coordinator.root)?;
    fs::create_dir_all(&coordinator.root)?;
    if let Some(parent) = coordinator.root.parent() {
        File::open(parent)?.sync_all()?;
    }
    let directory = coordinator.root.join(COMMITS);
    reject_symlink(&directory)?;
    fs::create_dir_all(&directory)?;
    File::open(&coordinator.root)?.sync_all()?;
    Ok(())
}

fn atomic_write(path: &Path, text: &str) -> Result<()> {
    let parent = path.parent().context("Missing parent")?;
    let tmp = parent.join(format!(".relations-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new().create_new(true).write(true).open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn checked_path(store: &Store, path: &Path) -> Result<PathBuf> {
    ensure!(
        !path.as_os_str().is_empty()
            && path.components().all(|c| matches!(c, Component::Normal(_))),
        "Unsafe relation journal path"
    );
    ensure!(
        matches!(path.components().next(),Some(Component::Normal(p)) if p == "working" || p == "archive"),
        "Journal target outside memory directories"
    );
    let root = fs::canonicalize(&store.root)?;
    let mut current = root.clone();
    for component in path.components() {
        current.push(component.as_os_str());
        ensure!(
            !fs::symlink_metadata(&current)?.file_type().is_symlink(),
            "Symlink in relation journal target"
        );
    }
    ensure!(current.is_file(), "Missing journal target");
    Ok(current)
}

fn safe_segment(segment: &std::ffi::OsStr) -> bool {
    segment.to_str().is_some_and(|value| {
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}

fn is_snapshot(path: &Path) -> bool {
    let parts: Vec<_> = path.components().collect();
    parts.len() == 3
        && matches!(parts[0], Component::Normal(p) if p == "history")
        && matches!(parts[2], Component::Normal(p) if p != "manifest.json")
}

fn is_manifest(path: &Path) -> bool {
    path.components().count() == 3 && path.file_name().is_some_and(|p| p == "manifest.json")
}

fn is_evidence(path: &Path) -> bool {
    matches!(path.components().next(), Some(Component::Normal(p)) if p == "evidence")
}

fn checked_mutation_path(store: &Store, path: &Path) -> Result<PathBuf> {
    let parts: Vec<_> = path.components().collect();
    ensure!(
        !parts.is_empty()
            && parts
                .iter()
                .all(|part| matches!(part, Component::Normal(_))),
        "Unsafe mutation path"
    );
    match parts.as_slice() {
        [Component::Normal(root), Component::Normal(name)] if *root == "working" => {
            ensure!(
                path.extension().is_some_and(|ext| ext == "md")
                    && path.file_stem().is_some_and(safe_segment)
                    && *name != ".md",
                "Invalid memory mutation target"
            );
        }
        [
            Component::Normal(root),
            Component::Normal(month),
            Component::Normal(name),
        ] if *root == "archive" => {
            ensure!(
                safe_segment(month)
                    && path.extension().is_some_and(|ext| ext == "md")
                    && path.file_stem().is_some_and(safe_segment)
                    && *name != ".md",
                "Invalid memory mutation target"
            );
        }
        [
            Component::Normal(root),
            Component::Normal(id),
            Component::Normal(name),
        ] if *root == "history" => {
            ensure!(safe_segment(id), "Unsafe history memory ID");
            ensure!(
                *name == "manifest.json"
                    || (path.extension().is_some_and(|ext| ext == "md")
                        && path.file_stem().is_some_and(safe_segment)),
                "Invalid history target"
            );
        }
        [Component::Normal(root), Component::Normal(name)]
            if *root == "evidence" || *root == "proposals" =>
        {
            ensure!(
                path.extension().is_some_and(|ext| ext == "json")
                    && path.file_stem().is_some_and(safe_segment)
                    && *name != ".json",
                "Invalid evidence target"
            );
        }
        [Component::Normal(root), Component::Normal(_)] if *root == "sleep" => {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            ensure!(
                path.extension().is_some_and(|ext| ext == "json")
                    && (stem == "cursor"
                        || (stem.len() == 64 && stem.bytes().all(|b| b.is_ascii_hexdigit()))),
                "Invalid sleep mutation target"
            );
        }
        _ => anyhow::bail!("Mutation target outside memory directories"),
    }
    ensure!(
        !fs::symlink_metadata(&store.root)?.file_type().is_symlink(),
        "Symlink store root"
    );
    let mut current = store.root.clone();
    for (index, component) in parts.iter().enumerate() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(meta) => {
                ensure!(!meta.file_type().is_symlink(), "Symlink in mutation target");
                if index + 1 == parts.len() {
                    ensure!(meta.is_file(), "Mutation target is not a file");
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        ensure!(meta.nlink() == 1, "Hardlink mutation target");
                    }
                } else {
                    ensure!(meta.is_dir(), "Mutation parent is not a directory");
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(current)
}

fn current_image(path: &Path) -> Result<Option<String>> {
    match fs::metadata(path) {
        Ok(meta) => ensure!(meta.len() <= MAX_MUTATION_IMAGE, "Mutation image too large"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn validate_mutation(
    store: &Store,
    changes: &[MutationRecord],
    committed: bool,
) -> Result<Vec<(PathBuf, Option<String>)>> {
    ensure!(!changes.is_empty(), "Empty mutation plan");
    let mut seen = HashSet::new();
    let mut pending = Vec::new();
    for change in changes {
        ensure!(
            change
                .before
                .as_ref()
                .is_none_or(|text| text.len() as u64 <= MAX_MUTATION_IMAGE)
                && change
                    .after
                    .as_ref()
                    .is_none_or(|text| text.len() as u64 <= MAX_MUTATION_IMAGE),
            "Mutation image too large"
        );
        ensure!(
            change.before.as_ref().map(|text| digest(text)) == change.before_hash
                && change.after.as_ref().map(|text| digest(text)) == change.after_hash,
            "Corrupt mutation journal images"
        );
        ensure!(
            change.before != change.after
                || (change.before.is_some()
                    && change.path.components().next().is_some_and(|root| {
                        matches!(root, Component::Normal(p) if p == "working" || p == "archive")
                    })),
            "Invalid no-op mutation change"
        );
        let target = checked_mutation_path(store, &change.path)?;
        ensure!(seen.insert(target.clone()), "Duplicate mutation target");
        if is_snapshot(&change.path) {
            ensure!(
                change.before.is_none() && change.after.is_some(),
                "History snapshots are immutable"
            );
        }
        if is_manifest(&change.path) {
            ensure!(change.after.is_some(), "Cannot delete history manifest");
            serde_json::from_str::<Value>(change.after.as_deref().unwrap())?;
        }
        if is_evidence(&change.path) {
            let after = change.after.as_deref().context("Cannot delete evidence")?;
            let value: Value = serde_json::from_str(after)?;
            ensure!(
                value.is_object()
                    && value["schema_version"] == 2
                    && value["memory"]["id"].as_str()
                        == change.path.file_stem().and_then(|stem| stem.to_str()),
                "Invalid evidence ledger"
            );
        }
        let current = current_image(&target)?;
        ensure!(
            current == change.before
                || ((committed || is_snapshot(&change.path)) && current == change.after),
            "RECOVERY_CONFLICT: external edit at {}; journal retained",
            target.display()
        );
        if committed {
            if current != change.after {
                pending.push((target, change.after.clone()));
            }
        } else if is_snapshot(&change.path) && current.is_some() {
            pending.push((target, None));
        }
    }
    Ok(pending)
}

fn create_parent_durable(path: &Path) -> Result<()> {
    let parent = path.parent().context("Missing mutation parent")?;
    if parent.exists() {
        return Ok(());
    }
    create_parent_durable(parent)?;
    fs::create_dir(parent)?;
    File::open(parent.parent().context("Missing ancestor")?)?.sync_all()?;
    Ok(())
}

fn materialize(path: &Path, after: Option<&str>) -> Result<()> {
    match after {
        Some(text) => {
            create_parent_durable(path)?;
            atomic_write(path, text)
        }
        None => {
            fs::remove_file(path)?;
            File::open(path.parent().context("Missing parent")?)?.sync_all()?;
            Ok(())
        }
    }
}

#[cfg(test)]
std::thread_local! {
    pub(crate) static ABORT_RECOVERY_AFTER: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn recovery_materialized_checkpoint() {
    ABORT_RECOVERY_AFTER.with(|remaining| {
        let count = remaining.get();
        if count > 0 {
            remaining.set(count - 1);
            if count == 1 {
                std::process::abort();
            }
        }
    });
}

fn recover_mutation(
    store: &Store,
    journal_path: &Path,
    raw: &str,
    coordinator: &Store,
) -> Result<()> {
    let journal: MutationJournal = serde_json::from_str(raw)?;
    ensure!(journal.version == 3, "Unsupported mutation journal");
    let committed = if let Some(operation) = journal.operation_id.as_deref() {
        ensure!(!journal.committed, "Invalid cross-store mutation journal");
        ensure!(
            journal.coordinator_key.as_deref() == Some(store_key(coordinator)?.as_str()),
            "RECOVERY_CONFLICT: trusted coordinator changed"
        );
        let decision = decision_path(coordinator, operation)?;
        if decision.exists() {
            let decision: CommitDecision = serde_json::from_str(&fs::read_to_string(decision)?)?;
            ensure!(
                decision.version == 1
                    && decision.participants.len() == 2
                    && decision.participants.get(&store_key(store)?) == Some(&digest(raw)),
                "RECOVERY_CONFLICT: journal does not match commit decision"
            );
            true
        } else {
            false
        }
    } else {
        ensure!(
            journal.coordinator_key.is_none(),
            "Invalid mutation coordinator"
        );
        journal.committed
    };
    let pending = validate_mutation(store, &journal.changes, committed)?;
    // Committed snapshots precede current records; manifests are published last.
    for (target, after) in pending.iter().filter(|(path, _)| {
        let relative = path.strip_prefix(&store.root).unwrap();
        is_snapshot(relative)
    }) {
        materialize(target, after.as_deref())?;
        #[cfg(test)]
        recovery_materialized_checkpoint();
    }
    for (target, after) in pending.iter().filter(|(path, _)| {
        let relative = path.strip_prefix(&store.root).unwrap();
        !is_snapshot(relative) && !is_manifest(relative)
    }) {
        materialize(target, after.as_deref())?;
        #[cfg(test)]
        recovery_materialized_checkpoint();
    }
    for (target, after) in pending
        .iter()
        .filter(|(path, _)| is_manifest(path.strip_prefix(&store.root).unwrap()))
    {
        materialize(target, after.as_deref())?;
        #[cfg(test)]
        recovery_materialized_checkpoint();
    }
    fs::remove_file(journal_path)?;
    File::open(&store.root)?.sync_all()?;
    Ok(())
}

/// Caller holds the store lock. Roll forward only when every file is a known
/// before/after image; an external edit leaves the journal and all files intact.
pub fn recover_pending(store: &Store) -> Result<()> {
    recover_with_coordinator(store, &global_store())
}

fn recover_with_coordinator(store: &Store, coordinator: &Store) -> Result<()> {
    let path = store.root.join(JOURNAL);
    match fs::symlink_metadata(&path) {
        Ok(meta) => {
            ensure!(!meta.file_type().is_symlink(), "Symlink relation journal");
            ensure!(meta.len() <= MAX_JOURNAL, "Relation journal too large");
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    let raw = fs::read_to_string(&path)?;
    if serde_json::from_str::<Value>(&raw)?["version"] == 3 {
        return recover_mutation(store, &path, &raw, coordinator);
    }
    let journal: Journal = serde_json::from_str(&raw)?;
    let committed = match journal.version {
        1 => {
            ensure!(
                journal.changes.len() == 2
                    && journal.operation_id.is_none()
                    && journal.coordinator_key.is_none(),
                "Unsupported relation journal"
            );
            true
        }
        2 => {
            ensure!(journal.changes.len() == 1, "Unsupported relation journal");
            let operation = journal
                .operation_id
                .as_deref()
                .context("Missing operation ID")?;
            ensure!(
                journal.coordinator_key.as_deref() == Some(store_key(coordinator)?.as_str()),
                "RECOVERY_CONFLICT: trusted coordinator changed"
            );
            let decision = decision_path(coordinator, operation)?;
            if decision.exists() {
                let decision: CommitDecision =
                    serde_json::from_str(&fs::read_to_string(decision)?)?;
                ensure!(
                    decision.version == 1
                        && decision.participants.len() == 2
                        && decision.participants.get(&store_key(store)?) == Some(&digest(&raw)),
                    "RECOVERY_CONFLICT: journal does not match commit decision"
                );
                true
            } else {
                false
            }
        }
        _ => anyhow::bail!("Unsupported relation journal"),
    };
    let mut seen = HashSet::new();
    let mut pending = Vec::new();
    for change in &journal.changes {
        ensure!(
            digest(&change.before) == change.before_hash
                && digest(&change.after) == change.after_hash,
            "Corrupt relation journal images"
        );
        let target = checked_path(store, &change.path)?;
        ensure!(
            seen.insert(target.clone()),
            "Duplicate relation journal target"
        );
        let current = fs::read_to_string(&target)?;
        ensure!(
            current == change.before || (committed && current == change.after),
            "RECOVERY_CONFLICT: external edit at {}; journal retained",
            target.display()
        );
        if committed && current != change.after {
            pending.push((target, &change.after));
        }
    }
    for (target, after) in pending {
        atomic_write(&target, after)?;
    }
    fs::remove_file(&path)?;
    File::open(&store.root)?.sync_all()?;
    Ok(())
}

fn prepare_mutation(store: &Store, plan: MutationPlan) -> Result<MutationJournal> {
    match fs::symlink_metadata(store.root.join(JOURNAL)) {
        Ok(_) => anyhow::bail!("Pending relation journal"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let journal = MutationJournal {
        version: 3,
        changes: plan
            .changes
            .into_iter()
            .map(|change| MutationRecord {
                before_hash: change.before.as_ref().map(|text| digest(text)),
                after_hash: change.after.as_ref().map(|text| digest(text)),
                path: change.path,
                before: change.before,
                after: change.after,
            })
            .collect(),
        committed: false,
        operation_id: None,
        coordinator_key: None,
    };
    validate_mutation(store, &journal.changes, false)?;
    // A prepared snapshot may already equal its intended image during recovery,
    // but a new operation must never claim an existing immutable image.
    for change in &journal.changes {
        let target = checked_mutation_path(store, &change.path)?;
        ensure!(
            current_image(&target)? == change.before,
            "VERSION_CONFLICT: mutation target changed before intent"
        );
    }
    Ok(journal)
}

fn write_snapshots(store: &Store, journal: &MutationJournal) -> Result<()> {
    for change in &journal.changes {
        if is_snapshot(&change.path) {
            let path = checked_mutation_path(store, &change.path)?;
            ensure!(
                current_image(&path)?.is_none(),
                "History snapshot already exists"
            );
            materialize(&path, change.after.as_deref())?;
        }
    }
    Ok(())
}

pub(crate) fn execute_mutation_with_checkpoint(
    store: &Store,
    plan: MutationPlan,
    mut checkpoint: impl FnMut(usize) -> Result<()>,
) -> Result<()> {
    let mut journal = prepare_mutation(store, plan)?;
    let journal_path = store.root.join(JOURNAL);
    ensure!(
        !fs::symlink_metadata(&store.root)?.file_type().is_symlink(),
        "Symlink store root"
    );
    let intent = serde_json::to_string(&journal)?;
    ensure!(
        intent.len() as u64 <= MAX_JOURNAL,
        "Mutation journal too large"
    );
    atomic_write(&journal_path, &intent)?;
    checkpoint(1)?;
    write_snapshots(store, &journal)?;
    checkpoint(2)?;
    validate_mutation(store, &journal.changes, false)?;
    journal.committed = true;
    atomic_write(&journal_path, &serde_json::to_string(&journal)?)?;
    checkpoint(3)?;
    recover_pending(store)?;
    checkpoint(4)?;
    Ok(())
}

/// Execute a validated single-store plan under the caller's store lock.
pub(crate) fn execute_mutation(store: &Store, plan: MutationPlan) -> Result<()> {
    execute_mutation_with_checkpoint(store, plan, |_| Ok(()))
}

fn commit_cross_store_mutations_with_checkpoint(
    stores: [&Store; 2],
    plans: [MutationPlan; 2],
    coordinator: &Store,
    mut checkpoint: impl FnMut(usize) -> Result<()>,
) -> Result<()> {
    let [plan0, plan1] = plans;
    // Validate both participants before creating any durable state.
    let mut first = prepare_mutation(stores[0], plan0)?;
    let mut second = prepare_mutation(stores[1], plan1)?;
    ensure!(
        store_key(stores[0])? != store_key(stores[1])?,
        "Duplicate participant store"
    );
    prepare_coordinator(coordinator)?;
    let operation = uuid::Uuid::new_v4().to_string();
    let mut participants = BTreeMap::new();
    let mut intents = Vec::new();
    for (store, journal) in stores.iter().zip([&mut first, &mut second]) {
        journal.operation_id = Some(operation.clone());
        journal.coordinator_key = Some(store_key(coordinator)?);
        let raw = serde_json::to_string(journal)?;
        ensure!(
            raw.len() as u64 <= MAX_JOURNAL,
            "Mutation journal too large"
        );
        participants.insert(store_key(store)?, digest(&raw));
        intents.push(raw);
    }
    let decision = decision_path(coordinator, &operation)?;
    for (index, ((store, journal), raw)) in stores
        .iter()
        .zip([&first, &second])
        .zip(intents)
        .enumerate()
    {
        atomic_write(&store.root.join(JOURNAL), &raw)?;
        checkpoint(index + 1)?;
        write_snapshots(store, journal)?;
        checkpoint(index + 3)?;
    }
    for (store, journal) in stores.iter().zip([&first, &second]) {
        validate_mutation(store, &journal.changes, false)?;
    }
    atomic_write(
        &decision,
        &serde_json::to_string(&CommitDecision {
            version: 1,
            participants,
        })?,
    )?;
    checkpoint(5)?;
    for (index, store) in stores.iter().enumerate() {
        recover_with_coordinator(store, coordinator)?;
        checkpoint(index + 6)?;
    }
    fs::remove_file(&decision)?;
    File::open(decision.parent().context("Missing decision directory")?)?.sync_all()?;
    Ok(())
}

/// Coordinate two schema-2 plans using the existing trusted commit directory.
/// Caller holds both participant locks in canonical order.
pub(crate) fn commit_cross_store_mutations(
    stores: [&Store; 2],
    plans: [MutationPlan; 2],
    coordinator: &Store,
) -> Result<()> {
    commit_cross_store_mutations_with_checkpoint(stores, plans, coordinator, |_| Ok(()))
}

// Caller holds both participant locks in canonical order. No memory is changed
// before the one durable decision. The callback is for deterministic crash-stage
// tests; ordinary callers supply a no-op.
fn commit_cross_store(
    stores: [&Store; 2],
    changes: Vec<Change>,
    coordinator: &Store,
    mut checkpoint: impl FnMut(usize) -> Result<()>,
) -> Result<()> {
    ensure!(changes.len() == 2, "Expected two relation changes");
    prepare_coordinator(coordinator)?;
    let operation = uuid::Uuid::new_v4().to_string();
    let decision_path = decision_path(coordinator, &operation)?;
    let mut participants = BTreeMap::new();
    for (index, (store, change)) in stores.iter().zip(changes).enumerate() {
        let journal = Journal {
            version: 2,
            changes: vec![change],
            operation_id: Some(operation.clone()),
            coordinator_key: Some(store_key(coordinator)?),
        };
        let text = serde_json::to_string(&journal)?;
        ensure!(
            participants
                .insert(store_key(store)?, digest(&text))
                .is_none(),
            "Duplicate participant store"
        );
        ensure!(
            !store.root.join(JOURNAL).exists(),
            "Pending relation journal"
        );
        atomic_write(&store.root.join(JOURNAL), &text)?;
        checkpoint(index + 1)?;
    }
    atomic_write(
        &decision_path,
        &serde_json::to_string(&CommitDecision {
            version: 1,
            participants,
        })?,
    )?;
    checkpoint(3)?;
    for (index, store) in stores.iter().enumerate() {
        recover_with_coordinator(store, coordinator)?;
        checkpoint(index + 4)?;
    }
    // Both journal deletions are already fsynced before removing the decision.
    // ponytail: interrupted operations retain a small decision file so a delayed
    // peer can still recover; reclaim only with a future all-participant audit.
    fs::remove_file(&decision_path)?;
    File::open(
        decision_path
            .parent()
            .context("Missing decision directory")?,
    )?
    .sync_all()?;
    Ok(())
}

fn add_link(memory: &mut Memory, id: &str, rel: &str) -> bool {
    if memory.links.iter().any(|l| l.id == id && l.rel == rel) {
        return false;
    }
    memory.links.push(Link {
        id: id.to_owned(),
        rel: rel.to_owned(),
    });
    true
}

pub fn link_entries(
    stores: &[Store],
    id1: &str,
    id2: &str,
    rel: &str,
    allow_custom: bool,
) -> Result<Value> {
    link_entries_checked(stores, id1, id2, rel, allow_custom, None)
}

pub fn link_entries_checked(
    stores: &[Store],
    id1: &str,
    id2: &str,
    rel: &str,
    allow_custom: bool,
    expected: Option<(&Memory, &Memory)>,
) -> Result<Value> {
    link_entries_inner(
        stores,
        id1,
        id2,
        rel,
        allow_custom,
        expected,
        &global_store(),
    )
}

fn link_entries_inner(
    stores: &[Store],
    id1: &str,
    id2: &str,
    rel: &str,
    allow_custom: bool,
    expected: Option<(&Memory, &Memory)>,
    coordinator: &Store,
) -> Result<Value> {
    ensure!(id1 != id2, "Cannot link a memory to itself");
    ensure!(!rel.trim().is_empty(), "Empty relation");
    // Stable lock order also prevents a concurrent participant changing either
    // endpoint while we resolve legacy bare IDs across the visible stores.
    let mut selected: Vec<_> = stores.iter().filter(|s| s.root.exists()).collect();
    selected.sort_by_key(|s| fs::canonicalize(&s.root).unwrap_or_else(|_| s.root.clone()));
    selected.dedup_by(|a, b| fs::canonicalize(&a.root).ok() == fs::canonicalize(&b.root).ok());
    let _guards = selected
        .iter()
        .map(|s| lock_store(s))
        .collect::<Result<Vec<_>>>()?;
    let mut entries = Vec::new();
    for store in selected {
        recover_pending(store)?;
        for (path, memory) in load_memories_unlocked(store, true)? {
            entries.push((store, path, memory));
        }
    }
    let locate = |id: &str| -> Result<usize> {
        let matches: Vec<_> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.2.id == id)
            .map(|(i, _)| i)
            .collect();
        ensure!(matches.len() == 1, "Memory missing or ambiguous: {id}");
        Ok(matches[0])
    };
    let a = locate(id1)?;
    let b = locate(id2)?;
    if let Some((first, second)) = expected {
        for (current, old) in [(&entries[a].2, first), (&entries[b].2, second)] {
            ensure!(
                semantic_digest(current) == semantic_digest(old),
                "VERSION_CONFLICT: consolidation candidate changed"
            );
        }
    }
    let same_store = fs::canonicalize(&entries[a].0.root)? == fs::canonicalize(&entries[b].0.root)?;
    let store = entries[a].0;
    let config = load_config(Some(store))?;
    ensure!(
        reverse(rel).is_some()
            || allow_custom
            || config["relations"]["allow_custom"]
                .as_bool()
                .unwrap_or(false),
        "Unknown relation: {rel}"
    );
    if is_demoting(rel) {
        let mut queue = vec![id2.to_owned()];
        let mut visited = HashSet::new();
        while let Some(id) = queue.pop() {
            ensure!(id != id1, "Supersedes would create a cycle");
            if !visited.insert(id.clone()) {
                continue;
            }
            let matches = entries
                .iter()
                .filter(|(_, _, m)| m.id == id)
                .collect::<Vec<_>>();
            ensure!(matches.len() <= 1, "Ambiguous supersedes target: {id}");
            if let Some((_, _, m)) = matches.first() {
                queue.extend(
                    m.links
                        .iter()
                        .filter(|l| l.rel == "supersedes")
                        .map(|l| l.id.clone()),
                );
            }
        }
    }
    let mut first = entries[a].2.clone();
    let mut second = entries[b].2.clone();
    let added = add_link(&mut first, id2, rel);
    add_link(&mut second, id1, reverse(rel).unwrap_or(rel));
    if is_demoting(rel) && added {
        second.strength = (second.strength - 20).max(0);
        second.status = "superseded".into();
        second.extra.insert("invalidated_by".into(), json!(id1));
    }
    let revised = [entries[a].0, entries[b].0]
        .iter()
        .map(|store| {
            let manifest = read_manifest(store)?;
            if let Some(manifest) = &manifest {
                ensure!(
                    manifest.schema_version == 2
                        && manifest.min_writer_version <= crate::provenance::WRITER_VERSION,
                    "INCOMPATIBLE_SCHEMA"
                );
            }
            if let Some(manifest) = &manifest {
                ensure!(
                    manifest.min_writer_version == crate::provenance::WRITER_VERSION,
                    "UPGRADE_REQUIRED: relation writes require store-upgrade --commit"
                );
            }
            Ok(manifest.is_some())
        })
        .collect::<Result<Vec<_>>>()?;
    if revised.iter().any(|enabled| *enabled) {
        ensure!(
            revised.iter().all(|enabled| *enabled),
            "UPGRADE_REQUIRED: both relation stores must support revision-aware writes"
        );
        let clock = SystemClock;
        let updates = [(a, first), (b, second)];
        if same_store {
            let updates = updates
                .into_iter()
                .map(|(index, memory)| {
                    let path = entries[index].1.clone();
                    Ok(RevisionUpdate {
                        expected: Some(snapshot(store, &path)?),
                        path,
                        memory,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let plan = plan_updates(store, &updates, "relation", &clock)?;
            if !plan.changes.is_empty() {
                execute_mutation(store, plan)?;
            }
        } else {
            let mut plans = Vec::new();
            for (index, memory) in updates {
                let destination = entries[index].0;
                let path = entries[index].1.clone();
                let update = RevisionUpdate {
                    expected: Some(snapshot(destination, &path)?),
                    path,
                    memory,
                };
                plans.push(plan_updates(destination, &[update], "relation", &clock)?);
            }
            let [first_plan, second_plan] = plans
                .try_into()
                .map_err(|_| anyhow::anyhow!("Expected two relation plans"))?;
            match (
                first_plan.changes.is_empty(),
                second_plan.changes.is_empty(),
            ) {
                (false, false) => commit_cross_store_mutations(
                    [entries[a].0, entries[b].0],
                    [first_plan, second_plan],
                    coordinator,
                )?,
                (false, true) => execute_mutation(entries[a].0, first_plan)?,
                (true, false) => execute_mutation(entries[b].0, second_plan)?,
                (true, true) => {}
            }
        }
        return Ok(json!({"ok":true,"rel":rel,"id1":id1,"id2":id2}));
    }
    let mut changes = Vec::new();
    for (index, memory) in [(a, first), (b, second)] {
        let path = &entries[index].1;
        let destination = entries[index].0;
        let relative = path.strip_prefix(&destination.root)?.to_path_buf();
        checked_path(destination, &relative)?;
        let before = fs::read_to_string(path)?;
        // Detect an edit made outside our lock since endpoint resolution.
        ensure!(
            serialize_memory(&crate::schema::parse_memory(&before)?)
                == serialize_memory(&entries[index].2),
            "VERSION_CONFLICT: memory changed during relation operation"
        );
        let after = serialize_memory(&memory);
        changes.push(Change {
            path: relative,
            before_hash: digest(&before),
            after_hash: digest(&after),
            before,
            after,
        });
    }
    if changes.iter().any(|c| c.before != c.after) && !same_store {
        commit_cross_store([entries[a].0, entries[b].0], changes, coordinator, |_| {
            Ok(())
        })?;
    } else if changes.iter().any(|c| c.before != c.after) {
        atomic_write(
            &store.root.join(JOURNAL),
            &serde_json::to_string(&Journal {
                version: 1,
                changes,
                operation_id: None,
                coordinator_key: None,
            })?,
        )?;
        recover_pending(store)?;
    }
    Ok(json!({"ok":true,"rel":rel,"id1":id1,"id2":id2}))
}

pub fn graph(stores: &[Store], id: &str, depth: usize, format: &str) -> Result<String> {
    ensure!(
        matches!(format, "ascii" | "json" | "mermaid"),
        "Unknown graph format: {format}"
    );
    let mut selected: Vec<_> = stores.iter().filter(|s| s.root.exists()).collect();
    selected.sort_by_key(|s| fs::canonicalize(&s.root).unwrap_or_else(|_| s.root.clone()));
    selected.dedup_by(|a, b| fs::canonicalize(&a.root).ok() == fs::canonicalize(&b.root).ok());
    // Graph is read-only: hold the normal sidecar lock but never recover files.
    let mut guards = Vec::new();
    for store in &selected {
        let file = lock_store_read_only(store)?;
        ensure!(
            !store.root.join(JOURNAL).exists(),
            "Pending relation operation; run a writable command to recover"
        );
        guards.push(file);
    }
    let mut entries = HashMap::new();
    for store in selected {
        for (_, memory) in load_memories_unlocked(store, true)? {
            ensure!(
                !entries.contains_key(&memory.id),
                "Ambiguous memory ID: {}",
                memory.id
            );
            entries.insert(memory.id.clone(), (store.scope.clone(), memory));
        }
    }
    ensure!(entries.contains_key(id), "Memory not found: {id}");
    let mut queue = VecDeque::from([(id.to_owned(), 0usize)]);
    let mut visited = HashSet::from([id.to_owned()]);
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut edge_keys = HashSet::new();
    while let Some((current, distance)) = queue.pop_front() {
        let (scope, memory) = &entries[&current];
        nodes.push(
            json!({"id":current,"title":memory.title(),"type":memory.memory_type,"scope":scope}),
        );
        if distance >= depth {
            continue;
        }
        for link in &memory.links {
            if !entries.contains_key(&link.id) {
                continue;
            }
            if edge_keys.insert((current.clone(), link.id.clone(), link.rel.clone())) {
                edges.push(json!({"source":current,"target":link.id,"rel":link.rel}));
            }
            if visited.insert(link.id.clone()) {
                queue.push_back((link.id.clone(), distance + 1));
            }
        }
    }
    if format == "json" {
        return Ok(serde_json::to_string_pretty(
            &json!({"root":id,"nodes":nodes,"edges":edges}),
        )?);
    }
    let mut lines = Vec::new();
    if format == "mermaid" {
        lines.push("graph LR".to_owned());
        // Index-based diagram IDs avoid collisions in legacy punctuation IDs.
        let identifiers: HashMap<_, _> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n["id"].as_str().unwrap(), format!("m{i}")))
            .collect();
        for node in &nodes {
            lines.push(format!(
                "  {}[\"{}\"]",
                identifiers[node["id"].as_str().unwrap()],
                escape(node["title"].as_str().unwrap())
            ));
        }
        for edge in &edges {
            lines.push(format!(
                "  {} -- {} --> {}",
                identifiers[edge["source"].as_str().unwrap()],
                escape(edge["rel"].as_str().unwrap()),
                identifiers[edge["target"].as_str().unwrap()]
            ));
        }
    } else {
        lines.push(id.to_owned());
        let mut visited = HashSet::from([id.to_owned()]);
        let mut stack = edges
            .iter()
            .filter(|e| e["source"] == id)
            .rev()
            .map(|e| (e, 1usize))
            .collect::<Vec<_>>();
        while let Some((edge, level)) = stack.pop() {
            let target = edge["target"].as_str().unwrap();
            let new = visited.insert(target.to_owned());
            lines.push(format!(
                "{}-> [{}] {}{}",
                "  ".repeat(level),
                edge["rel"].as_str().unwrap(),
                target,
                if new { "" } else { " (cycle)" }
            ));
            if new {
                stack.extend(
                    edges
                        .iter()
                        .filter(|e| e["source"] == target)
                        .rev()
                        .map(|e| (e, level + 1)),
                );
            }
        }
    }
    Ok(lines.join("\n"))
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace(['\n', '\r'], " ")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ensure_store, load_memories, working_path, write_memory};
    fn fixture() -> Result<(tempfile::TempDir, Store)> {
        let tmp = tempfile::tempdir()?;
        let store = Store {
            scope: "project".into(),
            root: tmp.path().join(".mnemosyne"),
        };
        ensure_store(&store)?;
        for id in ["a", "b", "c"] {
            let memory = Memory {
                id: id.into(),
                memory_type: "codebase".into(),
                strength: 70,
                status: "active".into(),
                body: format!("## Title {id}"),
                ..Default::default()
            };
            write_memory(&working_path(&store, &memory)?, &memory)?;
        }
        Ok((tmp, store))
    }
    #[test]
    fn typed_links_are_idempotent_and_graph_honors_depth() -> Result<()> {
        let (_tmp, store) = fixture()?;
        let stores = std::slice::from_ref(&store);
        link_entries(stores, "a", "b", "supersedes", false)?;
        link_entries(stores, "a", "b", "supersedes", false)?;
        link_entries(stores, "b", "c", "related", false)?;
        assert!(link_entries(stores, "b", "a", "supersedes", false).is_err());
        let entries = load_memories(&store, true)?;
        let b = &entries.iter().find(|(_, m)| m.id == "b").unwrap().1;
        assert_eq!(b.strength, 50);
        assert_eq!(b.status, "superseded");
        let graph: Value = serde_json::from_str(&graph(stores, "a", 1, "json")?)?;
        assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(graph["edges"].as_array().unwrap().len(), 1);
        assert!(super::graph(stores, "a", 2, "ascii")?.contains("-> [supersedes] b"));
        assert!(super::graph(stores, "a", 2, "mermaid")?.contains("graph LR"));
        Ok(())
    }
    #[test]
    fn partial_commit_recovers_but_external_edit_is_preserved() -> Result<()> {
        let (_tmp, store) = fixture()?;
        let memories = load_memories(&store, true)?;
        let mut changes = Vec::new();
        for (path, memory) in memories.iter().take(2) {
            let before = fs::read_to_string(path)?;
            let mut after_memory = memory.clone();
            after_memory.strength = 42;
            let after = serialize_memory(&after_memory);
            changes.push(Change {
                path: path.strip_prefix(&store.root)?.to_path_buf(),
                before_hash: digest(&before),
                after_hash: digest(&after),
                before,
                after,
            });
        }
        let journal = Journal {
            version: 1,
            changes,
            operation_id: None,
            coordinator_key: None,
        };
        atomic_write(&store.root.join(JOURNAL), &serde_json::to_string(&journal)?)?;
        atomic_write(
            &store.root.join(&journal.changes[0].path),
            &journal.changes[0].after,
        )?;
        assert!(graph(std::slice::from_ref(&store), "a", 1, "json").is_err());
        assert!(store.root.join(JOURNAL).exists());
        let second = store.root.join(&journal.changes[1].path);
        atomic_write(&second, "external edit")?;
        assert!(recover_pending(&store).is_err());
        assert_eq!(fs::read_to_string(&second)?, "external edit");
        assert!(store.root.join(JOURNAL).exists());
        atomic_write(&second, &journal.changes[1].before)?;
        let _guard = lock_store(&store)?;
        recover_pending(&store)?;
        assert!(!store.root.join(JOURNAL).exists());
        assert_eq!(fs::read_to_string(second)?, journal.changes[1].after);
        Ok(())
    }
    #[test]
    fn cross_store_links_keep_reverse_status_and_reject_cycles() -> Result<()> {
        let (_tmp, store) = fixture()?;
        let other_tmp = tempfile::tempdir()?;
        let other = Store {
            scope: "global".into(),
            root: other_tmp.path().join("other"),
        };
        let coordinator = Store {
            scope: "global".into(),
            root: other_tmp.path().join("coordinator"),
        };
        ensure_store(&other)?;
        let memory = Memory {
            id: "other".into(),
            body: "other".into(),
            strength: 70,
            status: "active".into(),
            ..Default::default()
        };
        write_memory(&working_path(&other, &memory)?, &memory)?;
        let stores = [store, other];
        for _ in 0..2 {
            link_entries_inner(
                &stores,
                "a",
                "other",
                "supersedes",
                false,
                None,
                &coordinator,
            )?;
        }
        let target = &load_memories(&stores[1], true)?[0].1;
        assert_eq!(target.status, "superseded");
        assert_eq!(target.strength, 50);
        assert_eq!(target.extra["invalidated_by"], "a");
        assert!(
            target
                .links
                .iter()
                .any(|link| link.id == "a" && link.rel == "superseded_by")
        );
        assert!(
            link_entries_inner(
                &stores,
                "other",
                "a",
                "supersedes",
                false,
                None,
                &coordinator
            )
            .is_err()
        );
        let result: Value = serde_json::from_str(&graph(&stores, "a", 1, "json")?)?;
        assert_eq!(result["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(fs::read_dir(coordinator.root.join(COMMITS))?.count(), 0);
        Ok(())
    }

    fn test_change(store: &Store) -> Result<Change> {
        let (path, mut memory) = load_memories(store, true)?.remove(0);
        let before = fs::read_to_string(&path)?;
        memory.strength = 42;
        let after = serialize_memory(&memory);
        Ok(Change {
            path: path.strip_prefix(&store.root)?.into(),
            before_hash: digest(&before),
            after_hash: digest(&after),
            before,
            after,
        })
    }

    #[test]
    fn cross_store_interruption_recovers_at_every_durable_stage() -> Result<()> {
        for stage in 1..=5 {
            let (_left_tmp, left) = fixture()?;
            let (_right_tmp, right) = fixture()?;
            let coordinator_tmp = tempfile::tempdir()?;
            let coordinator = Store {
                scope: "global".into(),
                root: coordinator_tmp.path().join("global"),
            };
            let changes = vec![test_change(&left)?, test_change(&right)?];
            {
                let _left_lock = lock_store_read_only(&left)?;
                let _right_lock = lock_store_read_only(&right)?;
                assert!(
                    commit_cross_store([&left, &right], changes.clone(), &coordinator, |point| {
                        ensure!(point != stage, "injected crash at stage {stage}");
                        Ok(())
                    })
                    .is_err()
                );
            }
            // A new caller has only its own store and trusted global config.
            // Recover peers in reverse order to exercise delayed participants.
            for (store, change) in [(&right, &changes[1]), (&left, &changes[0])] {
                let _guard = lock_store_read_only(store)?;
                recover_with_coordinator(store, &coordinator)?;
                assert_eq!(
                    fs::read_to_string(store.root.join(&change.path))?,
                    if stage >= 3 {
                        &change.after
                    } else {
                        &change.before
                    }
                    .to_owned()
                );
                assert!(!store.root.join(JOURNAL).exists());
            }
        }
        Ok(())
    }

    #[test]
    fn cross_store_recovery_preserves_external_edits_and_rejects_journal_tampering() -> Result<()> {
        let (_left_tmp, left) = fixture()?;
        let (_right_tmp, right) = fixture()?;
        let coordinator_tmp = tempfile::tempdir()?;
        let coordinator = Store {
            scope: "global".into(),
            root: coordinator_tmp.path().join("global"),
        };
        let changes = vec![test_change(&left)?, test_change(&right)?];
        assert!(
            commit_cross_store([&left, &right], changes.clone(), &coordinator, |point| {
                ensure!(point != 3, "injected crash after decision");
                Ok(())
            })
            .is_err()
        );
        let _guard = lock_store_read_only(&left)?;
        let target = left.root.join(&changes[0].path);
        atomic_write(&target, "external edit")?;
        assert!(recover_with_coordinator(&left, &coordinator).is_err());
        assert_eq!(fs::read_to_string(&target)?, "external edit");
        assert!(left.root.join(JOURNAL).exists());
        atomic_write(&target, &changes[0].before)?;
        let original = fs::read_to_string(left.root.join(JOURNAL))?;
        let mut tampered: Journal = serde_json::from_str(&original)?;
        tampered.operation_id = Some("../../outside".into());
        atomic_write(&left.root.join(JOURNAL), &serde_json::to_string(&tampered)?)?;
        assert!(recover_with_coordinator(&left, &coordinator).is_err());
        assert_eq!(fs::read_to_string(&target)?, changes[0].before);
        tampered = serde_json::from_str(&original)?;
        tampered.changes[0].path = PathBuf::from("../outside.md");
        atomic_write(&left.root.join(JOURNAL), &serde_json::to_string(&tampered)?)?;
        assert!(recover_with_coordinator(&left, &coordinator).is_err());
        assert!(left.root.join(JOURNAL).exists());
        atomic_write(&left.root.join(JOURNAL), &original)?;
        recover_with_coordinator(&left, &coordinator)?;
        assert_eq!(fs::read_to_string(target)?, changes[0].after);
        Ok(())
    }

    fn mutation_fixture(store: &Store) -> Result<(MutationPlan, PathBuf)> {
        let (path, _) = load_memories(store, true)?.remove(0);
        let relative = path.strip_prefix(&store.root)?.to_path_buf();
        let before = fs::read_to_string(&path)?;
        let snapshot = PathBuf::from("history/a/rev-1.md");
        Ok((
            MutationPlan {
                changes: vec![
                    MutationChange {
                        path: relative.clone(),
                        before: Some(before),
                        after: Some("revised memory".into()),
                    },
                    MutationChange {
                        path: snapshot,
                        before: None,
                        after: Some("revised memory".into()),
                    },
                    MutationChange {
                        path: PathBuf::from("history/a/manifest.json"),
                        before: None,
                        after: Some("{\"current\":\"rev-1\"}".into()),
                    },
                ],
            },
            relative,
        ))
    }

    #[test]
    fn single_store_mutation_recovers_at_durable_stages() -> Result<()> {
        for stage in 1..=4 {
            let (_tmp, store) = fixture()?;
            let (plan, working) = mutation_fixture(&store)?;
            let original = fs::read_to_string(store.root.join(&working))?;
            assert!(
                execute_mutation_with_checkpoint(&store, plan, |point| {
                    ensure!(point != stage, "injected crash at stage {stage}");
                    Ok(())
                })
                .is_err()
            );
            recover_pending(&store)?;
            let committed = stage >= 3;
            assert_eq!(
                fs::read_to_string(store.root.join(&working))?,
                if committed {
                    "revised memory"
                } else {
                    &original
                }
            );
            assert_eq!(store.root.join("history/a/rev-1.md").exists(), committed);
            assert_eq!(
                store.root.join("history/a/manifest.json").exists(),
                committed
            );
            assert!(!store.root.join(JOURNAL).exists());
        }
        Ok(())
    }

    #[test]
    fn mutation_rejects_invalid_targets_before_intent_and_preserves_conflicts() -> Result<()> {
        let (_tmp, store) = fixture()?;
        let (plan, working) = mutation_fixture(&store)?;
        for bad in [
            "../outside.md",
            "history/a/../bad.md",
            "history/a/manifest.txt",
            "evidence/a/other.json",
            "evidence/../a.json",
            "evidence/a.txt",
            "other/a.md",
        ] {
            let mut invalid = mutation_fixture(&store)?.0;
            invalid.changes[1].path = bad.into();
            assert!(execute_mutation(&store, invalid).is_err());
            assert!(!store.root.join(JOURNAL).exists());
        }
        let mut duplicate = mutation_fixture(&store)?.0;
        duplicate.changes[1].path = working.clone();
        assert!(execute_mutation(&store, duplicate).is_err());
        let mut invalid_ledger = mutation_fixture(&store)?.0;
        invalid_ledger.changes[1].path = "evidence/a.json".into();
        invalid_ledger.changes[1].after =
            Some("{\"schema_version\":2,\"memory\":{\"id\":\"b\"}}".into());
        assert!(execute_mutation(&store, invalid_ledger).is_err());
        let history = store.root.join("history");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(store.root.join("working"), &history)?;
            assert!(execute_mutation(&store, mutation_fixture(&store)?.0).is_err());
            fs::remove_file(history)?;
        }
        assert!(
            execute_mutation_with_checkpoint(&store, plan, |stage| {
                ensure!(stage != 3, "injected crash after decision");
                Ok(())
            })
            .is_err()
        );
        let target = store.root.join(&working);
        atomic_write(&target, "outside edit")?;
        assert!(recover_pending(&store).is_err());
        assert_eq!(fs::read_to_string(&target)?, "outside edit");
        assert!(store.root.join(JOURNAL).exists());
        Ok(())
    }

    #[test]
    fn mutation_rejects_corrupt_hash_and_cleans_prepared_snapshot() -> Result<()> {
        let (_tmp, store) = fixture()?;
        let (plan, working) = mutation_fixture(&store)?;
        let mut journal = prepare_mutation(&store, plan)?;
        let raw = serde_json::to_string(&journal)?;
        atomic_write(&store.root.join(JOURNAL), &raw)?;
        materialize(
            &store.root.join("history/a/rev-1.md"),
            Some("revised memory"),
        )?;
        journal.changes[0].after_hash = Some("wrong".into());
        atomic_write(&store.root.join(JOURNAL), &serde_json::to_string(&journal)?)?;
        assert!(recover_pending(&store).is_err());
        assert!(store.root.join("history/a/rev-1.md").exists());
        atomic_write(&store.root.join(JOURNAL), &raw)?;
        recover_pending(&store)?;
        assert!(!store.root.join("history/a/rev-1.md").exists());
        assert!(!store.root.join("history/a/manifest.json").exists());
        assert!(store.root.join(working).exists());
        Ok(())
    }

    #[test]
    fn mutation_never_claims_preexisting_history_image() -> Result<()> {
        let (_tmp, store) = fixture()?;
        let (plan, _) = mutation_fixture(&store)?;
        let snapshot = store.root.join("history/a/rev-1.md");
        materialize(&snapshot, Some("revised memory"))?;
        assert!(execute_mutation(&store, plan).is_err());
        assert_eq!(fs::read_to_string(snapshot)?, "revised memory");
        assert!(!store.root.join(JOURNAL).exists());
        Ok(())
    }

    #[test]
    fn committed_mutation_recovers_partial_current_materialization() -> Result<()> {
        let (_tmp, store) = fixture()?;
        let (plan, working) = mutation_fixture(&store)?;
        assert!(
            execute_mutation_with_checkpoint(&store, plan, |point| {
                ensure!(point != 3, "injected crash after decision");
                Ok(())
            })
            .is_err()
        );
        atomic_write(&store.root.join(&working), "revised memory")?;
        recover_pending(&store)?;
        assert_eq!(
            fs::read_to_string(store.root.join(working))?,
            "revised memory"
        );
        assert!(store.root.join("history/a/manifest.json").exists());
        assert!(!store.root.join(JOURNAL).exists());
        Ok(())
    }

    #[test]
    fn cross_store_mutation_keeps_history_with_decision() -> Result<()> {
        for stage in 1..=7 {
            let (_left_tmp, left) = fixture()?;
            let (_right_tmp, right) = fixture()?;
            let coordinator_tmp = tempfile::tempdir()?;
            let coordinator = Store {
                scope: "global".into(),
                root: coordinator_tmp.path().join("global"),
            };
            let (left_plan, left_path) = mutation_fixture(&left)?;
            let (right_plan, right_path) = mutation_fixture(&right)?;
            let left_before = fs::read_to_string(left.root.join(&left_path))?;
            let right_before = fs::read_to_string(right.root.join(&right_path))?;
            assert!(
                commit_cross_store_mutations_with_checkpoint(
                    [&left, &right],
                    [left_plan, right_plan],
                    &coordinator,
                    |point| {
                        ensure!(point != stage, "injected crash at stage {stage}");
                        Ok(())
                    }
                )
                .is_err()
            );
            for (store, path, before) in [
                (&right, right_path, right_before),
                (&left, left_path, left_before),
            ] {
                recover_with_coordinator(store, &coordinator)?;
                assert_eq!(
                    fs::read_to_string(store.root.join(path))?,
                    if stage >= 5 {
                        "revised memory"
                    } else {
                        &before
                    }
                );
                assert_eq!(store.root.join("history/a/rev-1.md").exists(), stage >= 5);
                assert_eq!(
                    store.root.join("history/a/manifest.json").exists(),
                    stage >= 5
                );
                assert!(!store.root.join(JOURNAL).exists());
            }
        }
        Ok(())
    }

    #[test]
    fn mutation_moves_markdown_with_absence_cas() -> Result<()> {
        let (_tmp, store) = fixture()?;
        let (path, _) = load_memories(&store, true)?.remove(0);
        let before = fs::read_to_string(&path)?;
        let source = path.strip_prefix(&store.root)?.to_path_buf();
        let destination = PathBuf::from("archive/test/a.md");
        execute_mutation(
            &store,
            MutationPlan {
                changes: vec![
                    MutationChange {
                        path: source.clone(),
                        before: Some(before.clone()),
                        after: None,
                    },
                    MutationChange {
                        path: destination.clone(),
                        before: None,
                        after: Some(before.clone()),
                    },
                ],
            },
        )?;
        assert!(!store.root.join(source).exists());
        assert_eq!(fs::read_to_string(store.root.join(destination))?, before);
        Ok(())
    }

    #[test]
    fn mutation_commits_evidence_with_history() -> Result<()> {
        let (_tmp, store) = fixture()?;
        let (mut plan, _) = mutation_fixture(&store)?;
        let evidence = "{\"schema_version\":2,\"memory\":{\"id\":\"a\"},\"events\":[{}]}";
        plan.changes.push(MutationChange {
            path: "evidence/a.json".into(),
            before: None,
            after: Some(evidence.into()),
        });
        execute_mutation(&store, plan)?;
        assert_eq!(
            fs::read_to_string(store.root.join("evidence/a.json"))?,
            evidence
        );
        assert!(store.root.join("history/a/manifest.json").exists());
        Ok(())
    }

    #[test]
    fn upgraded_links_record_both_histories_without_duplicate_revisions() -> Result<()> {
        let (_tmp, store) = fixture()?;
        crate::provenance::upgrade_store(&store)?;
        link_entries(std::slice::from_ref(&store), "a", "b", "related", false)?;
        for id in ["a", "b"] {
            let raw =
                fs::read_to_string(store.root.join("history").join(id).join("manifest.json"))?;
            let manifest: crate::revisions::HistoryManifest = serde_json::from_str(&raw)?;
            assert_eq!(manifest.entries.len(), 2);
            assert_eq!(manifest.entries[0].reason, "adopted");
            assert_eq!(manifest.entries[1].reason, "relation");
        }
        link_entries(std::slice::from_ref(&store), "a", "b", "related", false)?;
        let raw = fs::read_to_string(store.root.join("history/a/manifest.json"))?;
        let manifest: crate::revisions::HistoryManifest = serde_json::from_str(&raw)?;
        assert_eq!(manifest.entries.len(), 2);
        Ok(())
    }

    #[test]
    fn upgraded_cross_store_link_records_both_histories() -> Result<()> {
        let (_tmp, left) = fixture()?;
        let (_right_tmp, right) = fixture()?;
        let coordinator_tmp = tempfile::tempdir()?;
        let coordinator = Store {
            scope: "global".into(),
            root: coordinator_tmp.path().join("global"),
        };
        let (right_path, mut memory) = load_memories(&right, true)?.remove(0);
        memory.id = "right".into();
        let unique = working_path(&right, &memory)?;
        write_memory(&unique, &memory)?;
        fs::remove_file(right_path)?;
        crate::provenance::upgrade_store(&left)?;
        crate::provenance::upgrade_store(&right)?;
        link_entries_inner(
            &[left.clone(), right.clone()],
            "a",
            "right",
            "related",
            false,
            None,
            &coordinator,
        )?;
        for (store, id) in [(&left, "a"), (&right, "right")] {
            let raw =
                fs::read_to_string(store.root.join("history").join(id).join("manifest.json"))?;
            let manifest: crate::revisions::HistoryManifest = serde_json::from_str(&raw)?;
            assert_eq!(manifest.entries.len(), 2);
            assert_eq!(manifest.entries[1].reason, "relation");
        }
        assert_eq!(fs::read_dir(coordinator.root.join(COMMITS))?.count(), 0);
        Ok(())
    }
}
