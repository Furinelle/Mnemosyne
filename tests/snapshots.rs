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
