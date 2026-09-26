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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked: Vec<Value>,
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
        if path.extension().is_some_and(|value| value == extension) {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "Unsafe sleep input"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                ensure!(metadata.nlink() == 1, "Hardlink sleep input");
            }
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
    let _lock = lock_store_read_only(store)?;
    crate::relations::recover_pending(store)?;
    export_locked(store, cursor, expected, limit)
}

fn export_locked(
    store: &Store,
    cursor: usize,
    expected: Option<&str>,
    limit: usize,
) -> Result<Batch> {
    ensure!((1..=100).contains(&limit), "Limit must be 1..100");
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
    let mut blocked = Vec::new();
    let mut end = cursor;
    for (index, path) in paths.iter().enumerate().skip(cursor).take(limit) {
        let size = fs::symlink_metadata(path)?.len();
        if size > MAX_RECORD_BYTES as u64 {
            blocked.push(
                json!({"index": index, "file": path.file_name(), "reason": "record_too_large"}),
            );
            end = index + 1;
            continue;
        }
        if total.saturating_add(size) > MAX_SCAN_BYTES {
            break;
        }
        end = index + 1;
        let prior_issues = issues.len();
        let raw = read_bounded(path, &mut total)?;
        if index < checkpoint_count {
            let parsed = serde_json::from_slice::<Checkpoint>(&raw);
            let checkpoint = match parsed {
                Ok(checkpoint)
                    if checkpoint.schema_version == 1
                        && checkpoint.store_id == manifest.store_id
                        && path.file_stem().and_then(|value| value.to_str())
                            == Some(checkpoint.id.as_str())
                        && uuid::Uuid::parse_str(&checkpoint.id)
                            .is_ok_and(|value| value.to_string() == checkpoint.id) =>
                {
                    checkpoint
                }
                _ => {
                    blocked.push(json!({"index": index, "file": path.file_name(), "reason": "invalid_checkpoint"}));
                    continue;
                }
            };
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
            let parsed = String::from_utf8(raw)
                .map_err(anyhow::Error::from)
                .and_then(|raw| parse_memory(&raw));
            let memory: Memory = match parsed {
                Ok(memory)
                    if path.file_stem().and_then(|id| id.to_str()) == Some(memory.id.as_str())
                        && crate::revisions::safe_id(&memory.id).is_ok() =>
                {
                    memory
                }
                _ => {
                    blocked.push(json!({"index": index, "file": path.file_name(), "reason": "invalid_memory"}));
                    continue;
                }
            };
            observed |=
                crate::revisions::observe_memory(store, path, &crate::provenance::SystemClock)?;
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
        // Leave room for receipt hashes and up to 20 proposal IDs below 64 KiB.
        if serde_json::to_vec(&(&inputs, &issues, &blocked))?.len() > 48 * 1024 {
            ensure!(index > cursor, "Sleep metadata record too large");
            inputs.pop();
            issues.truncate(prior_issues);
            end = index;
            break;
        }
    }
    ensure!(
        end > cursor || cursor == paths.len(),
        "Sleep page cannot make progress"
    );
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
        blocked,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputPage {
    pub index: usize,
    pub total: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    pub batch: Batch,
    pub proposals: Vec<crate::proposals::Request>,
    pub output: Option<OutputPage>,
}

pub fn import(store: &Store, request: Import, clock: &impl Clock) -> Result<Value> {
    finish_page(
        store,
        &request.batch,
        request.proposals,
        request.output.unwrap_or(OutputPage { index: 0, total: 1 }),
        clock,
    )
}

pub fn finish(
    store: &Store,
    batch: &Batch,
    requests: Vec<crate::proposals::Request>,
    clock: &impl Clock,
) -> Result<Value> {
    finish_page(
        store,
        batch,
        requests,
        OutputPage { index: 0, total: 1 },
        clock,
    )
}

pub fn finish_page(
    store: &Store,
    batch: &Batch,
    requests: Vec<crate::proposals::Request>,
    output: OutputPage,
    clock: &impl Clock,
) -> Result<Value> {
    finish_page_with_checkpoint(store, batch, requests, output, clock, |_| Ok(()))
}

fn read_report(store: &Store, key: &str) -> Result<Option<String>> {
    let path = report_path(store, key)?;
    match fs::symlink_metadata(&path) {
        Ok(meta) => {
            ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "Unsafe sleep report"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                ensure!(meta.nlink() == 1, "Hardlink sleep report");
            }
            Ok(Some(String::from_utf8(crate::input::read_bytes(
                File::open(path)?,
                MAX_REPORT_BYTES,
            )?)?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn save_report(
    plan: &mut crate::relations::MutationPlan,
    key: &str,
    before: Option<String>,
    value: &Value,
) -> Result<()> {
    let after = serde_json::to_string(value)?;
    ensure!(after.len() <= MAX_REPORT_BYTES, "Report budget exceeded");
    if before.as_deref() != Some(after.as_str()) {
        plan.changes.push(crate::relations::MutationChange {
            path: PathBuf::from("sleep").join(format!("{key}.json")),
            before,
            after: Some(after),
        });
    }
    Ok(())
}

fn finish_page_with_checkpoint(
    store: &Store,
    batch: &Batch,
    requests: Vec<crate::proposals::Request>,
    output: OutputPage,
    clock: &impl Clock,
    checkpoint: impl FnMut(usize) -> Result<()>,
) -> Result<Value> {
    ensure!(
        batch.version == 1
            && requests.len() <= MAX_PROPOSALS
            && serde_json::to_vec(&requests)?.len() <= MAX_REPORT_BYTES,
        "Sleep output limit exceeded"
    );
    ensure!(
        (1..=1000).contains(&output.total) && output.index < output.total,
        "Invalid output page"
    );
    let _lock = lock_store_read_only(store)?;
    crate::relations::recover_pending(store)?;
    let manifest = read_manifest(store)?.context("Upgrade required")?;
    let run_id = hash(batch)?;
    let receipt_id = hash(&json!([run_id, output.index]))?;
    let request_hash = hash(&json!([batch, requests, output]))?;
    if let Some(raw) = read_report(store, &receipt_id)? {
        let receipt: Value = serde_json::from_str(&raw)?;
        ensure!(
            receipt["store_id"] == manifest.store_id && receipt["request_hash"] == request_hash,
            "OUTPUT_CONFLICT: page content changed"
        );
        // A receipt records a past commit, even after its source or proposal
        // changes. Replaying it must not reapply proposals or alter progress.
        return Ok(receipt);
    }
    let current = export_locked(
        store,
        batch.cursor,
        Some(&batch.snapshot),
        batch
            .next_cursor
            .checked_sub(batch.cursor)
            .context("Invalid cursor range")?
            .max(1),
    )?;
    ensure!(hash(&current)? == run_id, "STALE_SLEEP_INPUT");
    let before = read_report(store, &run_id)?;
    if let Some(raw) = &before {
        let state: Value = serde_json::from_str(raw)?;
        if state["version"] == 1 {
            let ids = requests.iter().map(hash).collect::<Result<Vec<_>>>()?;
            ensure!(
                output.index == 0
                    && output.total == 1
                    && state["batch"] == serde_json::to_value(batch)?
                    && state["proposal_ids"] == serde_json::to_value(ids)?,
                "OUTPUT_CONFLICT: completed legacy batch"
            );
            // v1 wrote report and cursor separately. Repair an interrupted cursor
            // write while preserving the original report and never rewinding.
            let cursor_before = read_report(store, "cursor")?;
            let progress: Value = cursor_before
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?
                .unwrap_or(json!({"next_cursor":0}));
            ensure!(
                progress["active_run"].is_null(),
                "INPUT_CURSOR_CONFLICT: output run pending"
            );
            let previous = if progress["snapshot"] == batch.snapshot {
                progress["next_cursor"]
                    .as_u64()
                    .context("Invalid saved cursor")? as usize
            } else {
                0
            };
            if previous < batch.next_cursor {
                ensure!(
                    previous == batch.cursor,
                    "INPUT_CURSOR_CONFLICT: finish preceding input outputs first"
                );
                let mut plan = crate::relations::MutationPlan { changes: vec![] };
                save_report(
                    &mut plan,
                    "cursor",
                    cursor_before,
                    &json!({"snapshot":batch.snapshot,
                    "next_cursor":batch.next_cursor, "status":if batch.partial {"partial"} else {"complete"}}),
                )?;
                crate::relations::execute_mutation(store, plan)?;
            }
            return Ok(state);
        }
        ensure!(
            state["version"] == 2
                && state["total"] == output.total
                && state["next_output"] == output.index,
            "OUTPUT_CONFLICT: expected next output page"
        );
    } else {
        ensure!(output.index == 0, "Output must start at zero");
    }
    let cursor_before = read_report(store, "cursor")?;
    let saved: Value = cursor_before
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or(json!({"next_cursor":0}));
    let same_snapshot = saved["snapshot"] == batch.snapshot;
    let previous = if same_snapshot {
        saved["next_cursor"]
            .as_u64()
            .context("Invalid saved cursor")? as usize
    } else {
        0
    };
    ensure!(
        batch.cursor == previous,
        "INPUT_CURSOR_CONFLICT: finish preceding input outputs first"
    );
    if same_snapshot && let Some(active) = saved["active_run"].as_str() {
        ensure!(
            active == run_id,
            "INPUT_CURSOR_CONFLICT: output run pending"
        );
    }
    let prior_blocked = if same_snapshot {
        saved["blocked_count"].as_u64().unwrap_or(0)
    } else {
        0
    };
    let blocked_count = prior_blocked + batch.blocked.len() as u64;
    let mut plan = crate::relations::MutationPlan { changes: vec![] };
    let mut proposal_ids = Vec::new();
    for request in requests {
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
        proposal_ids.push(crate::proposals::plan_proposal(store, request, clock, &mut plan)?.id);
    }
    let complete = output.index + 1 == output.total;
    let status = if !complete {
        "pending_output"
    } else if blocked_count > 0 || batch.partial {
        "partial"
    } else {
        "complete"
    };
    let receipt = json!({"version":2, "store_id":manifest.store_id, "run_id":run_id, "receipt_id":receipt_id,
        "request_hash":request_hash, "output":output, "next_output":output.index + 1,
        "input_committed":complete, "status":status, "batch":batch, "proposal_ids":proposal_ids,
        "mode":"offline_or_host", "facts_modified":false});
    save_report(&mut plan, &receipt_id, None, &receipt)?;
    save_report(
        &mut plan,
        &run_id,
        before,
        &json!({"version":2, "total":output.total,
        "next_output":output.index + 1, "status":status, "last_receipt":receipt_id}),
    )?;
    let mut progress = json!({"snapshot":batch.snapshot,
        "next_cursor": if complete { batch.next_cursor } else { previous },
        "blocked_count": if complete { blocked_count } else { prior_blocked }, "status":status});
    if !complete {
        progress["active_run"] = json!(run_id);
    }
    save_report(&mut plan, "cursor", cursor_before, &progress)?;
    crate::relations::execute_mutation_with_checkpoint(store, plan, checkpoint)?;
    Ok(receipt)
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
    let _lock = lock_store_read_only(store)?;
    crate::relations::recover_pending(store)?;
    let Some(raw) = read_report(store, "cursor")? else {
        return Ok(json!({"next_cursor": 0}));
    };
    let value: Value = serde_json::from_str(&raw)?;
    ensure!(
        value["next_cursor"].is_u64() && value["snapshot"].as_str().is_some_and(|v| v.len() == 64),
        "invalid sleep cursor"
    );
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abrupt_commit_child() -> Result<()> {
        let Ok(root) = std::env::var("MNEMOSYNE_TEST_ABORT_STORE") else {
            return Ok(());
        };
        let store = Store {
            scope: "project".into(),
            root: root.into(),
        };
        let import: Import =
            serde_json::from_slice(&fs::read(store.root.parent().unwrap().join("import.json"))?)?;
        let stage: usize = std::env::var("MNEMOSYNE_TEST_ABORT_STAGE")?.parse()?;
        if let Ok(after) = std::env::var("MNEMOSYNE_TEST_ABORT_RECOVERY_AFTER") {
            let after: usize = after.parse()?;
            ensure!(after > 0, "invalid recovery abort point");
            crate::relations::ABORT_RECOVERY_AFTER.with(|remaining| remaining.set(after));
        }
        finish_page_with_checkpoint(
            &store,
            &import.batch,
            import.proposals,
            import.output.unwrap(),
            &crate::provenance::SystemClock,
            |point| {
                if point == stage {
                    std::process::abort();
                }
                Ok(())
            },
        )?;
        anyhow::bail!("abort checkpoint was not reached")
    }

    #[test]
    fn abrupt_process_exit_retries_committed_output_without_response() -> Result<()> {
        for stage in 1..=4 {
            let temp = tempfile::tempdir()?;
            let store = Store {
                scope: "project".into(),
                root: temp.path().join("store"),
            };
            crate::provenance::upgrade_store(&store)?;
            let clock = crate::provenance::SystemClock;
            crate::provenance::write_v2(
                &store,
                &crate::provenance::WriteRequestV2 {
                    memory_type: "codebase".into(),
                    title: "abrupt".into(),
                    content: "before".into(),
                    origin: "test".into(),
                    source_session_id: "s".into(),
                    source_event_id: "e".into(),
                    finding_key: "f".into(),
                    source_kind: "tool_output".into(),
                    verification_state: "unverified".into(),
                    ..Default::default()
                },
                &clock,
            )?;
            let batch = export(&store, 0, None, 1)?;
            let input = &batch.inputs[0];
            let requests = vec![crate::proposals::Request {
                decision: "REFINE".into(),
                reason: "abort recovery".into(),
                evidence: vec![],
                targets: vec![crate::proposals::Target {
                    memory_ref: input.memory_ref.clone(),
                    expected_rev: input.revision,
                    expected_hash: input.semantic_hash.clone(),
                    body: Some("after".into()),
                    status: None,
                }],
            }];
            let page = OutputPage { index: 0, total: 1 };
            fs::write(
                temp.path().join("import.json"),
                serde_json::to_vec(&json!({
                    "batch": batch, "proposals": requests, "output": page
                }))?,
            )?;
            let abort_child = |recovery_after: Option<usize>| -> Result<()> {
                let mut child = std::process::Command::new(std::env::current_exe()?);
                child
                    .args(["--exact", "sleep::tests::abrupt_commit_child"])
                    .env_clear()
                    .env("HOME", temp.path())
                    .env("MNEMOSYNE_HOME", temp.path().join("global"))
                    .env("MNEMOSYNE_TEST_ABORT_STORE", &store.root)
                    .env("MNEMOSYNE_TEST_ABORT_STAGE", stage.to_string())
                    .current_dir(temp.path());
                if let Some(after) = recovery_after {
                    child.env("MNEMOSYNE_TEST_ABORT_RECOVERY_AFTER", after.to_string());
                }
                let result = child.output()?;
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    assert_eq!(
                        result.status.signal(),
                        Some(6),
                        "child must abort, not return an error"
                    );
                }
                Ok(())
            };
            abort_child(None)?;
            let mut committed_changes = Vec::new();
            if stage == 3 {
                let journal_path = store.root.join(".relations-operation.json");
                let original_journal = fs::read(&journal_path)?;
                let journal: Value = serde_json::from_slice(&original_journal)?;
                assert_eq!(journal["committed"], true);
                committed_changes = journal["changes"].as_array().unwrap().clone();
                assert_eq!(committed_changes.len(), 4);
                let mut materialized = 0;
                // Die in recovery twice: after the first object, then after two
                // more. Each restart must accept the durable after-image prefix.
                for after in [1, 2] {
                    abort_child(Some(after))?;
                    materialized += after;
                    assert_eq!(fs::read(&journal_path)?, original_journal);
                    assert_eq!(
                        committed_changes
                            .iter()
                            .filter(|change| {
                                fs::read_to_string(
                                    store.root.join(change["path"].as_str().unwrap()),
                                )
                                .is_ok_and(|raw| Some(raw.as_str()) == change["after"].as_str())
                            })
                            .count(),
                        materialized
                    );
                }
            }
            let receipt = finish_page(&store, &batch, requests.clone(), page.clone(), &clock)?;
            assert_eq!(
                receipt,
                finish_page(&store, &batch, requests, page, &clock)?
            );
            assert_eq!(cursor(&store)?["next_cursor"], 1);
            let id = receipt["proposal_ids"][0].as_str().unwrap();
            assert_eq!(crate::proposals::show(&store, id)?.state, "pending");
            assert_eq!(fs::read_dir(store.root.join("proposals"))?.count(), 1);
            assert!(!store.root.join(".relations-operation.json").exists());
            for change in committed_changes {
                assert_eq!(
                    fs::read_to_string(store.root.join(change["path"].as_str().unwrap()))?,
                    change["after"].as_str().unwrap()
                );
            }
        }
        Ok(())
    }

    #[test]
    fn output_transaction_recovers_every_boundary_and_materialization_prefix() -> Result<()> {
        // A page publishes a proposal, receipt, run state and input cursor in one journal.
        for stage in 1..=4 {
            for materialized in 0..=4 {
                let temp = tempfile::tempdir()?;
                let store = Store {
                    scope: "project".into(),
                    root: temp.path().join("store"),
                };
                crate::provenance::upgrade_store(&store)?;
                let clock = crate::provenance::SystemClock;
                crate::provenance::write_v2(
                    &store,
                    &crate::provenance::WriteRequestV2 {
                        memory_type: "codebase".into(),
                        title: "example".into(),
                        content: "before".into(),
                        origin: "test".into(),
                        source_session_id: "session".into(),
                        source_event_id: "event".into(),
                        finding_key: "key".into(),
                        source_kind: "tool_output".into(),
                        verification_state: "verified".into(),
                        ..Default::default()
                    },
                    &clock,
                )?;
                let batch = export(&store, 0, None, 1)?;
                let input = &batch.inputs[0];
                let requests = vec![crate::proposals::Request {
                    decision: "REFINE".into(),
                    reason: "review".into(),
                    evidence: vec![],
                    targets: vec![crate::proposals::Target {
                        memory_ref: input.memory_ref.clone(),
                        expected_rev: input.revision,
                        expected_hash: input.semantic_hash.clone(),
                        body: Some("after".into()),
                        status: None,
                    }],
                }];
                let page = OutputPage { index: 0, total: 1 };
                let result = finish_page_with_checkpoint(
                    &store,
                    &batch,
                    requests.clone(),
                    page.clone(),
                    &clock,
                    |point| {
                        if point == stage {
                            if point == 3 {
                                let journal: Value = serde_json::from_slice(&fs::read(
                                    store.root.join(".relations-operation.json"),
                                )?)?;
                                for change in journal["changes"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .take(materialized)
                                {
                                    let path = store.root.join(change["path"].as_str().unwrap());
                                    fs::create_dir_all(path.parent().unwrap())?;
                                    fs::write(path, change["after"].as_str().unwrap())?;
                                }
                            }
                            anyhow::bail!("simulated interruption");
                        }
                        Ok(())
                    },
                );
                assert!(result.is_err());
                let receipt = finish_page(&store, &batch, requests.clone(), page.clone(), &clock)?;
                assert_eq!(
                    receipt,
                    finish_page(&store, &batch, requests, page, &clock)?
                );
                assert_eq!(cursor(&store)?["next_cursor"], 1);
                assert_eq!(fs::read_dir(store.root.join("proposals"))?.count(), 1);
                assert!(!store.root.join(".relations-operation.json").exists());
                let proposal =
                    crate::proposals::show(&store, receipt["proposal_ids"][0].as_str().unwrap())?;
                assert_eq!(proposal.state, "pending");
            }
        }
        Ok(())
    }
}
