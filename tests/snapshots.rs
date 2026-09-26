use anyhow::Result;
use mnemosyne::{
    proposals::{Request, Target},
    provenance::read_manifest,
    snapshot,
    store::{Store, ensure_store},
};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn fixture(root: &Path) -> Result<Store> {
    let store = Store {
        scope: "project".into(),
        root: root.into(),
    };
    ensure_store(&store)?;
    mnemosyne::provenance::upgrade_store(&store)?;
    let id = read_manifest(&store)?.unwrap().store_id;
    mnemosyne::proposals::propose(
        &store,
        Request {
            decision: "REFINE".into(),
            reason: "fixture".into(),
            evidence: vec![],
            targets: vec![Target {
                memory_ref: mnemosyne::provenance::MemoryRef {
                    store_id: id,
                    memory_id: "a".into(),
                },
                expected_rev: 1,
                expected_hash: "0".repeat(64),
                body: Some("new wording".into()),
                status: None,
            }],
        },
        &mnemosyne::provenance::SystemClock,
    )?;
    fs::write(store.root.join("config.toml"), "api_key = 'secret'\n")?;
    fs::write(store.root.join("transcript.jsonl"), "secret transcript")?;
    fs::write(store.root.join("working/legacy.md.lock"), "")?;
    Ok(store)
}

#[test]
fn snapshot_restore_and_fork_keep_distinct_identity() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let original = fixture(&tmp.path().join("original"))?;
    fs::write(
        original.working_dir().join("restore-fact.md"),
        "---\nid: restore-fact\ntype: codebase\nsource: agent\n---\n# Restored invariant\n",
    )?;
    let package = tmp.path().join("package");
    let manifest = snapshot::create(&original, &package)?;
    let proposal_path = manifest
        .files
        .iter()
        .find(|entry| entry.path.starts_with("proposals/"))
        .unwrap()
        .path
        .clone();
    assert!(
        manifest
            .files
            .iter()
            .any(|entry| entry.path == proposal_path)
    );
    assert!(
        !manifest
            .files
            .iter()
            .any(|entry| entry.path == "config.toml")
    );
    assert!(!package.join("files/config.toml").exists());
    assert!(!package.join("files/transcript.jsonl").exists());
    assert!(!package.join("files/working/legacy.md.lock").exists());

    let restored = tmp.path().join("restored");
    snapshot::restore(&package, &restored, false)?;
    assert!(fs::read_to_string(restored.join("MEMORY.md"))?.contains("`restore-fact`"));
    let restored_store = Store {
        scope: "project".into(),
        root: restored.clone(),
    };
    assert_eq!(
        read_manifest(&restored_store)?.unwrap().store_id,
        manifest.store_id
    );
    assert_eq!(
        fs::read(restored.join(&proposal_path))?,
        fs::read(original.root.join(&proposal_path))?
    );
    assert!(
        mnemosyne::provenance::resolve_ref(
            &[original.clone(), restored_store.clone()],
            &mnemosyne::provenance::MemoryRef {
                store_id: manifest.store_id.clone(),
                memory_id: "a".into()
            }
        )
        .is_err()
    );

    let fork = tmp.path().join("fork");
    let fork_manifest = snapshot::restore(&package, &fork, true)?;
    assert!(fs::read_to_string(fork.join("MEMORY.md"))?.contains("`restore-fact`"));
    let fork_store = Store {
        scope: "project".into(),
        root: fork.clone(),
    };
    let fork_id = read_manifest(&fork_store)?.unwrap().store_id;
    assert_ne!(fork_id, manifest.store_id);
    let fork_proposal_path = &fork_manifest
        .files
        .iter()
        .find(|entry| entry.path.starts_with("proposals/"))
        .unwrap()
        .path;
    assert_ne!(fork_proposal_path, &proposal_path);
    let proposal: Value = serde_json::from_slice(&fs::read(fork.join(fork_proposal_path))?)?;
    assert_eq!(proposal["store_id"], fork_id);
    assert_eq!(proposal["state"], "stale");
    let original_proposal: Value =
        serde_json::from_slice(&fs::read(original.root.join(&proposal_path))?)?;
    assert_eq!(
        proposal["fork_origin_summary_hash"],
        original_proposal["summary_hash"]
    );
    assert_eq!(
        proposal["request"]["targets"][0]["memory_ref"]["store_id"],
        fork_id
    );
    assert!(!fork.join("config.toml").exists());
    Ok(())
}

#[test]
fn restore_rejects_tampering_without_publishing() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let original = fixture(&tmp.path().join("original"))?;
    let package = tmp.path().join("package");
    snapshot::create(&original, &package)?;
    let manifest_path = package.join("manifest.json");
    let baseline = fs::read(&manifest_path)?;
    for path in [
        "../escape",
        "/absolute",
        "working/../escape",
        "working//escape",
        "config.toml",
    ] {
        let mut value: Value = serde_json::from_slice(&baseline)?;
        value["files"][0]["path"] = json!(path);
        fs::write(&manifest_path, serde_json::to_vec(&value)?)?;
        let target = tmp.path().join("target");
        assert!(
            snapshot::restore(&package, &target, false).is_err(),
            "accepted {path}"
        );
        assert!(!target.exists());
    }
    let mut value: Value = serde_json::from_slice(&baseline)?;
    let duplicate = value["files"][0].clone();
    value["files"].as_array_mut().unwrap().push(duplicate);
    fs::write(&manifest_path, serde_json::to_vec(&value)?)?;
    assert!(snapshot::restore(&package, &tmp.path().join("target"), false).is_err());
    value = serde_json::from_slice(&baseline)?;
    value["files"][0]["size"] = json!(u64::MAX);
    fs::write(&manifest_path, serde_json::to_vec(&value)?)?;
    assert!(snapshot::restore(&package, &tmp.path().join("target"), false).is_err());
    fs::write(&manifest_path, &baseline)?;
    fs::write(package.join("files/core.md"), "corrupted")?;
    assert!(snapshot::restore(&package, &tmp.path().join("target"), false).is_err());
    assert!(!tmp.path().join("target").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn snapshot_rejects_links_and_pending_journal() -> Result<()> {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir()?;
    let original = fixture(&tmp.path().join("original"))?;
    symlink(
        original.root.join("core.md"),
        original.root.join("working/linked.md"),
    )?;
    assert!(snapshot::create(&original, &tmp.path().join("one")).is_err());
    fs::remove_file(original.root.join("working/linked.md"))?;
    fs::hard_link(
        original.root.join("core.md"),
        original.root.join("working/linked.md"),
    )?;
    assert!(snapshot::create(&original, &tmp.path().join("two")).is_err());
    fs::remove_file(original.root.join("working/linked.md"))?;
    fs::write(
        original.root.join(".relations-operation.json"),
        "unresolved",
    )?;
    assert!(snapshot::create(&original, &tmp.path().join("three")).is_err());
    assert!(original.root.join(".relations-operation.json").exists());
    Ok(())
}

fn sleep_store(root: &Path, count: usize) -> Result<Store> {
    let store = Store {
        scope: "project".into(),
        root: root.into(),
    };
    mnemosyne::provenance::upgrade_store(&store)?;
    for index in 0..count {
        mnemosyne::provenance::write_v2(
            &store,
            &mnemosyne::provenance::WriteRequestV2 {
                memory_type: "codebase".into(),
                title: format!("fact {index}"),
                content: "original content".into(),
                origin: "test".into(),
                source_session_id: "snapshot".into(),
                source_event_id: index.to_string(),
                finding_key: index.to_string(),
                source_kind: "tool_output".into(),
                verification_state: "verified".into(),
                ..Default::default()
            },
            &mnemosyne::provenance::SystemClock,
        )?;
    }
    Ok(store)
}

#[test]
fn sleep_partial_output_refused_completed_receipts_restore_and_fork_refused() -> Result<()> {
    use mnemosyne::{
        provenance::SystemClock,
        sleep::{self, OutputPage},
    };
    let temp = tempfile::tempdir()?;
    let original = sleep_store(&temp.path().join("original"), 1)?;
    let batch = sleep::export(&original, 0, None, 10)?;
    let input = &batch.inputs[0];
    let requests = vec![Request {
        decision: "REFINE".into(),
        reason: "snapshot test".into(),
        evidence: vec![],
        targets: vec![Target {
            memory_ref: input.memory_ref.clone(),
            expected_rev: input.revision,
            expected_hash: input.semantic_hash.clone(),
            body: Some("revised content".into()),
            status: None,
        }],
    }];
    let first = OutputPage { index: 0, total: 2 };
    let receipt = sleep::finish_page(
        &original,
        &batch,
        requests.clone(),
        first.clone(),
        &SystemClock,
    )?;
    let package = temp.path().join("package");
    assert!(
        snapshot::create(&original, &package)
            .unwrap_err()
            .to_string()
            .contains("unfinished sleep")
    );
    assert!(!package.exists());
    assert_eq!(
        sleep::finish_page(
            &original,
            &batch,
            requests.clone(),
            first.clone(),
            &SystemClock
        )?,
        receipt
    );
    let last = OutputPage { index: 1, total: 2 };
    let final_receipt = sleep::finish_page(&original, &batch, vec![], last.clone(), &SystemClock)?;
    let manifest = snapshot::create(&original, &package)?;
    assert_eq!(
        manifest
            .files
            .iter()
            .filter(|entry| entry.path.starts_with("sleep/"))
            .count(),
        4
    );
    let restored = Store {
        scope: "project".into(),
        root: temp.path().join("restored"),
    };
    snapshot::restore(&package, &restored.root, false)?;
    for entry in manifest
        .files
        .iter()
        .filter(|entry| entry.path.starts_with("sleep/"))
    {
        assert_eq!(
            fs::read(original.root.join(&entry.path))?,
            fs::read(restored.root.join(&entry.path))?
        );
    }
    assert_eq!(
        sleep::finish_page(&restored, &batch, requests, first, &SystemClock)?,
        receipt
    );
    assert_eq!(
        sleep::finish_page(&restored, &batch, vec![], last, &SystemClock)?,
        final_receipt
    );
    assert_eq!(fs::read_dir(restored.root.join("proposals"))?.count(), 1);
    // Fresh work cannot reuse an inode/path-bound source inventory after restore.
    assert!(sleep::export(&restored, batch.next_cursor, Some(&batch.snapshot), 10).is_err());
    let fresh = sleep::export(&restored, 0, None, 10)?;
    assert_ne!(fresh.snapshot, batch.snapshot);
    assert_eq!(
        sleep::finish(&restored, &fresh, vec![], &SystemClock)?["status"],
        "complete"
    );
    let fork = temp.path().join("fork");
    assert!(
        snapshot::restore(&package, &fork, true)
            .unwrap_err()
            .to_string()
            .contains("sleep receipts")
    );
    assert!(!fork.exists());
    Ok(())
}

#[test]
fn sleep_partial_input_refused_until_complete() -> Result<()> {
    use mnemosyne::{provenance::SystemClock, sleep};
    let temp = tempfile::tempdir()?;
    let store = sleep_store(&temp.path().join("original"), 2)?;
    let first = sleep::export(&store, 0, None, 1)?;
    assert!(first.partial);
    sleep::finish(&store, &first, vec![], &SystemClock)?;
    let package = temp.path().join("package");
    assert!(
        snapshot::create(&store, &package)
            .unwrap_err()
            .to_string()
            .contains("unfinished sleep")
    );
    assert!(!package.exists());
    let last = sleep::export(&store, first.next_cursor, Some(&first.snapshot), 1)?;
    assert!(!last.partial);
    sleep::finish(&store, &last, vec![], &SystemClock)?;
    snapshot::create(&store, &package)?;
    Ok(())
}

#[test]
fn sleep_snapshot_rejects_malformed_records_without_publishing() -> Result<()> {
    use mnemosyne::{provenance::SystemClock, sleep};
    use sha2::{Digest, Sha256};
    let temp = tempfile::tempdir()?;
    let store = sleep_store(&temp.path().join("original"), 1)?;
    sleep::rules(&store, 0, None, 10, &SystemClock)?;
    let cursor = store.root.join("sleep/cursor.json");
    let baseline = fs::read(&cursor)?;
    for bad in [b"{".as_slice(), b"{}", b"null", b"[]"] {
        fs::write(&cursor, bad)?;
        let target = temp.path().join("bad");
        assert!(snapshot::create(&store, &target).is_err());
        assert!(!target.exists());
    }
    fs::write(&cursor, &baseline)?;
    let invalid = store.root.join("sleep/not-a-digest.json");
    fs::write(&invalid, "{}")?;
    assert!(snapshot::create(&store, &temp.path().join("bad")).is_err());
    fs::remove_file(invalid)?;
    fs::write(
        &cursor,
        serde_json::to_vec(&json!({"padding": "x".repeat(64 * 1024)}))?,
    )?;
    assert!(
        snapshot::create(&store, &temp.path().join("bad"))
            .unwrap_err()
            .to_string()
            .contains("sleep record too large")
    );
    fs::write(&cursor, baseline)?;
    let package = temp.path().join("package");
    snapshot::create(&store, &package)?;
    // Rehash the tampered JSON so schema validation, not only checksums, rejects it.
    fs::write(package.join("files/sleep/cursor.json"), "{}")?;
    let manifest_path = package.join("manifest.json");
    let mut manifest: snapshot::SnapshotManifest =
        serde_json::from_slice(&fs::read(&manifest_path)?)?;
    let entry = manifest
        .files
        .iter_mut()
        .find(|entry| entry.path == "sleep/cursor.json")
        .unwrap();
    entry.size = 2;
    entry.sha256 = format!("{:x}", Sha256::digest(b"{}"));
    fs::write(manifest_path, serde_json::to_vec(&manifest)?)?;
    let restored = temp.path().join("restored");
    assert!(snapshot::restore(&package, &restored, false).is_err());
    assert!(!restored.exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn sleep_snapshot_rejects_symlinks_and_hardlinks() -> Result<()> {
    use mnemosyne::{provenance::SystemClock, sleep};
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir()?;
    let store = sleep_store(&temp.path().join("original"), 1)?;
    sleep::rules(&store, 0, None, 10, &SystemClock)?;
    let cursor = store.root.join("sleep/cursor.json");
    let saved = temp.path().join("cursor.json");
    fs::rename(&cursor, &saved)?;
    symlink(&saved, &cursor)?;
    assert!(snapshot::create(&store, &temp.path().join("bad")).is_err());
    fs::remove_file(&cursor)?;
    fs::hard_link(&saved, &cursor)?;
    assert!(snapshot::create(&store, &temp.path().join("bad")).is_err());
    fs::remove_file(&cursor)?;
    fs::rename(&saved, &cursor)?;
    let package = temp.path().join("package");
    snapshot::create(&store, &package)?;
    let packaged_cursor = package.join("files/sleep/cursor.json");
    fs::rename(&packaged_cursor, &saved)?;
    let restored = temp.path().join("restored");
    symlink(&saved, &packaged_cursor)?;
    assert!(snapshot::restore(&package, &restored, false).is_err());
    assert!(!restored.exists());
    fs::remove_file(&packaged_cursor)?;
    fs::hard_link(&saved, &packaged_cursor)?;
    assert!(snapshot::restore(&package, &restored, false).is_err());
    assert!(!restored.exists());
    let state = temp.path().join("sleep");
    fs::rename(store.root.join("sleep"), &state)?;
    symlink(&state, store.root.join("sleep"))?;
    assert!(snapshot::create(&store, &temp.path().join("bad")).is_err());
    Ok(())
}

#[test]
fn completed_legacy_sleep_report_is_preserved() -> Result<()> {
    use mnemosyne::sleep;
    use sha2::{Digest, Sha256};
    let temp = tempfile::tempdir()?;
    let store = sleep_store(&temp.path().join("original"), 1)?;
    let batch = sleep::export(&store, 0, None, 10)?;
    let run_id = format!("{:x}", Sha256::digest(serde_json::to_vec(&batch)?));
    fs::create_dir(store.root.join("sleep"))?;
    let report = json!({"version": 1, "batch": batch, "proposal_ids": [],
        "mode": "offline_or_host", "facts_modified": false});
    let relative = format!("sleep/{run_id}.json");
    fs::write(store.root.join(&relative), serde_json::to_vec(&report)?)?;
    fs::write(
        store.root.join("sleep/cursor.json"),
        serde_json::to_vec(&json!({
            "snapshot": batch.snapshot, "next_cursor": batch.next_cursor
        }))?,
    )?;
    let package = temp.path().join("package");
    snapshot::create(&store, &package)?;
    let restored = temp.path().join("restored");
    snapshot::restore(&package, &restored, false)?;
    assert_eq!(
        fs::read(store.root.join(&relative))?,
        fs::read(restored.join(&relative))?
    );
    Ok(())
}

#[test]
fn abandoned_old_sleep_output_preserves_receipt_after_new_inventory_completes() -> Result<()> {
    use mnemosyne::{
        provenance::SystemClock,
        sleep::{self, OutputPage},
    };
    let temp = tempfile::tempdir()?;
    let original = sleep_store(&temp.path().join("original"), 1)?;
    let old = sleep::export(&original, 0, None, 10)?;
    let first = OutputPage { index: 0, total: 2 };
    let receipt = sleep::finish_page(&original, &old, vec![], first.clone(), &SystemClock)?;
    let path = original
        .working_dir()
        .join(format!("{}.md", old.inputs[0].memory_ref.memory_id));
    fs::write(
        &path,
        format!("{}\nupdated source\n", fs::read_to_string(&path)?),
    )?;
    let current = sleep::export(&original, 0, None, 10)?;
    assert_ne!(old.snapshot, current.snapshot);
    sleep::finish(&original, &current, vec![], &SystemClock)?;
    let package = temp.path().join("package");
    snapshot::create(&original, &package)?;
    let restored = Store {
        scope: "project".into(),
        root: temp.path().join("restored"),
    };
    snapshot::restore(&package, &restored.root, false)?;
    assert_eq!(
        sleep::finish_page(&restored, &old, vec![], first, &SystemClock)?,
        receipt
    );
    assert!(
        sleep::finish_page(
            &restored,
            &old,
            vec![],
            OutputPage { index: 1, total: 2 },
            &SystemClock
        )
        .is_err()
    );
    Ok(())
}
