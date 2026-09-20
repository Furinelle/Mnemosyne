//! Short-lived task handoff, independent of durable knowledge ingestion.
use crate::{
    provenance::{Clock, atomic_json, read_manifest},
    store::{Store, lock_store, lock_store_read_only},
};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TestEvidence {
    pub command: String,
    pub cwd: String,
    pub worktree_fingerprint: String,
    pub exit_code: Option<i32>,
    pub executed_at: Option<String>,
    pub summary_ref: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CheckpointInput {
    pub task_id: String,
    pub goal: String,
    pub completed_actions: Vec<String>,
    pub artifacts: Vec<String>,
    pub tests: Vec<TestEvidence>,
    pub unresolved: Vec<String>,
    pub next_action: String,
    pub source_session: String,
    pub source_agent: String,
    pub scoped_paths: Vec<String>,
    pub expires: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Worktree {
    pub observed_commit: Option<String>,
    pub fingerprint: String,
    pub scoped_paths: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub schema_version: u32,
    pub id: String,
    pub store_id: String,
    pub revision: u64,
    pub state: String,
    pub created_at: String,
    pub updated_at: String,
    pub worktree: Worktree,
    /// These commands/results are caller reports, never trusted execution receipts.
    pub evidence_origin: String,
    pub data: CheckpointInput,
}
fn relative(value: &str) -> Result<&Path> {
    let path = Path::new(value);
    ensure!(
        !value.is_empty()
            && !value.contains('\\')
            && !path.is_absolute()
            && path
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir)),
        "Expected a project-relative path"
    );
    ensure!(
        !path
            .components()
            .any(|c| c.as_os_str() == ".git" || c.as_os_str() == ".mnemosyne"),
        "Private metadata is not a checkpoint artifact"
    );
    Ok(path)
}
fn no_links(path: &Path) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        ensure!(
            !metadata.file_type().is_symlink(),
            "Symlink checkpoint path rejected"
        );
    }
    Ok(())
}
fn directory(store: &Store, create: bool) -> Result<PathBuf> {
    no_links(&store.root)?;
    let path = store.root.join("checkpoints");
    no_links(&path)?;
    if create {
        fs::create_dir_all(&path)?;
    }
    if path.exists() {
        ensure!(path.is_dir(), "Invalid checkpoints directory");
    }
    Ok(path)
}
fn record_path(store: &Store, id: &str) -> Result<PathBuf> {
    let uuid = uuid::Uuid::parse_str(id)?;
    ensure!(uuid.to_string() == id, "Invalid checkpoint id");
    let path = directory(store, false)?.join(format!("{id}.json"));
    no_links(&path)?;
    Ok(path)
}
fn validate(data: &CheckpointInput) -> Result<()> {
    ensure!(
        !data.task_id.trim().is_empty()
            && data.task_id.len() <= 256
            && !data.goal.trim().is_empty(),
        "task_id and goal are required"
    );
    ensure!(
        !data.scoped_paths.is_empty() && data.scoped_paths.len() <= 64,
        "Provide 1..64 scoped_paths"
    );
    ensure!(
        data.tests.len() <= 64
            && data.completed_actions.len() <= 128
            && data.unresolved.len() <= 128
            && data.artifacts.len() <= 128,
        "Checkpoint list limit exceeded"
    );
    ensure!(
        serde_json::to_vec(data)?.len() <= 256 * 1024,
        "Checkpoint input too large"
    );
    for path in data.scoped_paths.iter().chain(data.artifacts.iter()) {
        relative(path)?;
    }
    DateTime::parse_from_rfc3339(&data.expires).context("expires must be RFC3339")?;
    for test in &data.tests {
        ensure!(!test.command.trim().is_empty(), "Test command is required");
        relative(if test.cwd.is_empty() { "." } else { &test.cwd })?;
        if let Some(date) = &test.executed_at {
            DateTime::parse_from_rfc3339(date)?;
        }
        ensure!(
            test.exit_code.is_none() || test.executed_at.is_some(),
            "Reported exit code requires executed_at"
        );
    }
    Ok(())
}
/// Reads only explicitly scoped files. Git identity is unavailable when git is absent.
pub fn observe(store: &Store, paths: &[String]) -> Result<Worktree> {
    ensure!(
        store.scope == "project",
        "Checkpoints require a project store"
    );
    let root = store
        .root
        .parent()
        .context("Missing project root")?
        .canonicalize()?;
    let mut paths = paths.to_vec();
    paths.sort();
    paths.dedup();
    ensure!(
        !paths.is_empty() && paths.len() <= 64,
        "Provide 1..64 scoped_paths"
    );
    let mut hash = Sha256::new();
    let mut total = 0usize;
    let commit = Command::new("git")
        .args(["--no-optional-locks", "-C"])
        .arg(&root)
        .args(["rev-parse", "--verify", "HEAD"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned());
    hash.update(serde_json::to_vec(&commit)?);
    for name in &paths {
        let relative = relative(name)?;
        let mut full = root.clone();
        for part in relative.components() {
            full.push(part);
            no_links(&full)?;
        }
        hash.update(serde_json::to_vec(name)?);
        match fs::metadata(&full) {
            Ok(meta) => {
                ensure!(meta.is_file(), "Scoped path must be a regular file");
                ensure!(
                    meta.len() <= 8 * 1024 * 1024
                        && total.saturating_add(meta.len() as usize) <= 16 * 1024 * 1024,
                    "Scoped files exceed checkpoint byte limit"
                );
                let bytes = crate::input::read_bytes(fs::File::open(&full)?, 8 * 1024 * 1024)?;
                total += bytes.len();
                ensure!(total <= 16 * 1024 * 1024, "Scoped byte limit exceeded");
                hash.update(b"file");
                hash.update(Sha256::digest(&bytes));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => hash.update(b"missing"),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(Worktree {
        observed_commit: commit,
        fingerprint: format!("{:x}", hash.finalize()),
        scoped_paths: paths,
    })
}
fn manifest(store: &Store) -> Result<crate::provenance::StoreManifest> {
    let manifest =
        read_manifest(store)?.context("Upgrade project store before using checkpoints")?;
    ensure!(
        manifest.schema_version == 2
            && manifest.min_writer_version <= crate::provenance::WRITER_VERSION,
        "INCOMPATIBLE_SCHEMA"
    );
    Ok(manifest)
}
fn read_unlocked(store: &Store, id: &str) -> Result<Checkpoint> {
    let path = record_path(store, id)?;
    let record: Checkpoint = serde_json::from_str(&crate::input::read(fs::File::open(path)?)?)?;
    ensure!(
        record.schema_version == 1 && record.id == id,
        "Unsupported checkpoint record"
    );
    let manifest = manifest(store)?;
    ensure!(
        record.store_id == manifest.store_id,
        "Checkpoint store identity mismatch"
    );
    validate(&record.data)?;
    Ok(record)
}
pub fn create(store: &Store, data: CheckpointInput, clock: &impl Clock) -> Result<Checkpoint> {
    validate(&data)?;
    let _lock = lock_store(store)?;
    let manifest = manifest(store)?;
    let now = clock.now();
    ensure!(
        DateTime::parse_from_rfc3339(&data.expires)?.with_timezone(&Utc) > now,
        "New checkpoint must expire in the future"
    );
    let record = Checkpoint {
        schema_version: 1,
        id: uuid::Uuid::new_v4().to_string(),
        store_id: manifest.store_id,
        revision: 1,
        state: "active".into(),
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
        worktree: observe(store, &data.scoped_paths)?,
        evidence_origin: "reported".into(),
        data,
    };
    directory(store, true)?;
    atomic_json(&record_path(store, &record.id)?, &record)?;
    Ok(record)
}
pub fn update(
    store: &Store,
    id: &str,
    revision: u64,
    data: CheckpointInput,
    clock: &impl Clock,
) -> Result<Checkpoint> {
    validate(&data)?;
    let _lock = lock_store(store)?;
    let mut record = read_unlocked(store, id)?;
    ensure!(record.revision == revision, "REVISION_CONFLICT");
    ensure!(record.state == "active", "Checkpoint is closed");
    ensure!(
        DateTime::parse_from_rfc3339(&record.data.expires)?.with_timezone(&Utc) > clock.now(),
        "Checkpoint is expired"
    );
    ensure!(
        record.data.task_id == data.task_id,
        "Task identity cannot change"
    );
    ensure!(
        DateTime::parse_from_rfc3339(&data.expires)?.with_timezone(&Utc) > clock.now(),
        "Checkpoint must expire in the future"
    );
    record.revision = record
        .revision
        .checked_add(1)
        .context("Revision overflow")?;
    record.worktree = observe(store, &data.scoped_paths)?;
    record.data = data;
    record.updated_at = clock.now().to_rfc3339();
    atomic_json(&record_path(store, id)?, &record)?;
    Ok(record)
}
pub fn close(store: &Store, id: &str, revision: u64, clock: &impl Clock) -> Result<Checkpoint> {
    let _lock = lock_store(store)?;
    let mut record = read_unlocked(store, id)?;
    ensure!(record.revision == revision, "REVISION_CONFLICT");
    if record.state != "closed" {
        record.state = "closed".into();
        record.revision = record
            .revision
            .checked_add(1)
            .context("Revision overflow")?;
        record.updated_at = clock.now().to_rfc3339();
        atomic_json(&record_path(store, id)?, &record)?;
    }
    Ok(record)
}
fn view(store: &Store, record: Checkpoint, clock: &impl Clock) -> Result<Value> {
    let current = observe(store, &record.worktree.scoped_paths)?;
    let matches = current.fingerprint == record.worktree.fingerprint;
    let tests:Vec<_>=record.data.tests.iter().map(|test|{
        let status=match (test.executed_at.as_ref(),test.exit_code){(_,None)=>"not_executed_or_unknown",(Some(_),Some(0))=>"reported_pass",(Some(_),Some(_))=>"reported_fail",_=>"unknown"};
        json!({"command":test.command,"status":status,"origin":"reported","needs_revalidation":test.worktree_fingerprint.is_empty() || test.worktree_fingerprint!=current.fingerprint})
    }).collect();
    let expired =
        DateTime::parse_from_rfc3339(&record.data.expires)?.with_timezone(&Utc) <= clock.now();
    Ok(
        json!({"checkpoint":record,"worktree_matches":matches,"current_worktree":current,"expired":expired,"test_results":tests,"verification":"reported_only"}),
    )
}
pub fn load(store: &Store, id: &str, clock: &impl Clock) -> Result<Value> {
    let _lock = lock_store_read_only(store)?;
    view(store, read_unlocked(store, id)?, clock)
}
/// Explicit task selection prevents unrelated task state leaking into normal recall.
pub fn active_context(
    stores: &[Store],
    task_id: Option<&str>,
    clock: &impl Clock,
) -> Result<Vec<Value>> {
    let Some(task) = task_id.filter(|s| !s.is_empty()) else {
        return Ok(vec![]);
    };
    let mut items = Vec::new();
    let mut skipped = 0;
    for store in stores.iter().filter(|s| s.scope == "project") {
        let dir = directory(store, false)?;
        if !dir.exists() {
            continue;
        }
        let _lock = lock_store_read_only(store)?;
        let mut paths = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let p = entry?.path();
            if p.extension().is_some_and(|e| e == "json") {
                paths.push(p);
            }
            ensure!(paths.len() <= 1000, "Checkpoint scan limit exceeded");
        }
        paths.sort();
        for path in paths {
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                skipped += 1;
                continue;
            };
            let Ok(record) = read_unlocked(store, id) else {
                skipped += 1;
                continue;
            };
            if record.state != "active"
                || record.data.task_id != task
                || DateTime::parse_from_rfc3339(&record.data.expires)?.with_timezone(&Utc)
                    <= clock.now()
            {
                continue;
            }
            let Ok(v) = view(store, record, clock) else {
                skipped += 1;
                continue;
            };
            let r = &v["checkpoint"];
            let failures = v["test_results"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|t| t["status"] == "reported_fail")
                .count();
            let revalidate = v["test_results"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["needs_revalidation"] == true)
                || v["worktree_matches"] == false;
            items.push(json!({"kind":"checkpoint","id":id,"source":r["data"]["source_agent"],"summary":format!("Task {}: {}. Reported failures: {}. Verification: reported only; needs revalidation: {}. Unresolved: {}. Next: {}",task,r["data"]["goal"].as_str().unwrap_or(""),failures,revalidate,r["data"]["unresolved"],r["data"]["next_action"].as_str().unwrap_or("")),"warnings":["reported_only"],"updated_at":r["updated_at"]}));
        }
    }
    items.sort_by(|a, b| b["updated_at"].as_str().cmp(&a["updated_at"].as_str()));
    items.truncate(3);
    if skipped > 0 {
        items.push(json!({"kind":"checkpoint_warning","id":"checkpoint-read-warning","summary":format!("{} checkpoint records could not be read; handoff context may be incomplete.", skipped),"warnings":["checkpoint_read_failed"]}));
    }
    Ok(items)
}

/// Versioned MCP writes share the same validation/CAS paths as the CLI.
pub fn dispatch(
    store: &Store,
    operation: &str,
    request: &Value,
    clock: &impl Clock,
) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Change {
        id: String,
        expected_revision: u64,
        data: CheckpointInput,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Close {
        id: String,
        expected_revision: u64,
    }
    match operation {
        "checkpoint_new" => Ok(serde_json::to_value(create(
            store,
            serde_json::from_value(request.clone())?,
            clock,
        )?)?),
        "checkpoint_update" => {
            let r: Change = serde_json::from_value(request.clone())?;
            Ok(serde_json::to_value(update(
                store,
                &r.id,
                r.expected_revision,
                r.data,
                clock,
            )?)?)
        }
        "checkpoint_close" => {
            let r: Close = serde_json::from_value(request.clone())?;
            Ok(serde_json::to_value(close(
                store,
                &r.id,
                r.expected_revision,
                clock,
            )?)?)
        }
        _ => anyhow::bail!("Unsupported checkpoint operation"),
    }
}
