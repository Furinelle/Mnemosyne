use anyhow::Result;
use chrono::{DateTime, Utc};
use mnemosyne::{
    provenance::{self, Clock, WriteRequestV2},
    revisions::{self, HistoryManifest},
    schema::{parse_memory, serialize_memory},
    store::{self, Store},
};
use std::{fs, path::Path};

struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        "2026-09-20T00:00:00Z".parse().unwrap()
    }
}

fn store() -> Result<(tempfile::TempDir, Store)> {
    let temp = tempfile::tempdir()?;
    let store = Store {
        scope: "project".into(),
        root: temp.path().join(".mnemosyne"),
    };
    store::ensure_store(&store)?;
    provenance::upgrade_store(&store)?;
    Ok((temp, store))
}

fn manifest(store: &Store, id: &str) -> Result<HistoryManifest> {
    Ok(serde_json::from_slice(&fs::read(
        store.root.join("history").join(id).join("manifest.json"),
    )?)?)
}

#[test]
fn revisions_ignore_heat_and_capture_semantic_edits() -> Result<()> {
    let (_temp, store) = store()?;
    let path = store.working_dir().join("fact.md");
    let mut memory = parse_memory("---\nid: fact\ntype: codebase\nsource: agent\n---\n# First\n")?;
    {
        let _lock = store::lock_store(&store)?;
        revisions::write_locked(&store, &path, &memory, None, "created", &FixedClock)?;
    }
    assert_eq!(manifest(&store, "fact")?.entries.len(), 1);
    let first = fs::read_to_string(store.root.join("history/fact/1.md"))?;
    let snap = revisions::snapshot(&store, &path)?;
    memory.strength += 10;
    memory.access_count += 1;
    memory.last_accessed = "2026-09-20".into();
    {
        let _lock = store::lock_store(&store)?;
        revisions::write_locked(&store, &path, &memory, Some(snap), "heat", &FixedClock)?;
    }
    assert_eq!(manifest(&store, "fact")?.entries.len(), 1);
    let snap = revisions::snapshot(&store, &path)?;
    memory.body = "# Corrected".into();
    {
        let _lock = store::lock_store(&store)?;
        revisions::write_locked(
            &store,
            &path,
            &memory,
            Some(snap),
            "correction",
            &FixedClock,
        )?;
    }
    let history = manifest(&store, "fact")?;
    assert_eq!(history.entries.len(), 2);
    assert_eq!(history.entries[1].reason, "correction");
    assert_eq!(
        fs::read_to_string(store.root.join("history/fact/1.md"))?,
        first
    );
    assert_eq!(
        parse_memory(&fs::read_to_string(store.root.join("history/fact/2.md"))?)?.body,
        "# Corrected"
    );
    Ok(())
}

#[test]
fn legacy_adoption_preserves_raw_and_external_edit_has_gap() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let store = Store {
        scope: "project".into(),
        root: temp.path().join(".mnemosyne"),
    };
    store::ensure_store(&store)?;
    let path = store.working_dir().join("fact.md");
    let original = "---\nid: fact\ntype: codebase\ncustom: [one, two]\n---\n# First\n";
    fs::write(&path, original)?;
    provenance::upgrade_store(&store)?;
    {
        let _lock = store::lock_store(&store)?;
    }
    assert_eq!(fs::read_to_string(&path)?, original);
    let history = manifest(&store, "fact")?;
    assert_eq!(history.entries.len(), 1);
    assert_eq!(history.entries[0].reason, "adopted");
    assert_eq!(
        fs::read_to_string(store.root.join("history/fact/1.md"))?,
        original
    );
    let changed = original.replace("# First", "# Hand edited");
    fs::write(&path, &changed)?;
    let _lock = store::lock_store(&store)?;
    assert_eq!(fs::read_to_string(&path)?, changed);
    let history = manifest(&store, "fact")?;
    assert_eq!(history.entries.len(), 2);
    assert_eq!(history.entries[1].reason, "external_edit");
    assert!(history.entries[1].unknown_gap);
    assert_eq!(
        fs::read_to_string(store.root.join("history/fact/2.md"))?,
        changed
    );
    Ok(())
}

#[test]
fn stale_semantic_snapshot_conflicts_but_heat_does_not() -> Result<()> {
    let (_temp, store) = store()?;
    let path = store.working_dir().join("fact.md");
    let original = parse_memory("---\nid: fact\n---\n# First\n")?;
    {
        let _lock = store::lock_store(&store)?;
        revisions::write_locked(&store, &path, &original, None, "created", &FixedClock)?;
    }
    let snap = revisions::snapshot(&store, &path)?;
    let mut heated = parse_memory(&fs::read_to_string(&path)?)?;
    heated.access_count = 42;
    fs::write(&path, serialize_memory(&heated))?;
    let mut corrected = original.clone();
    corrected.body = "# Corrected".into();
    {
        let _lock = store::lock_store(&store)?;
        revisions::write_locked(
            &store,
            &path,
            &corrected,
            Some(snap.clone()),
            "correction",
            &FixedClock,
        )?;
    }
    assert_eq!(parse_memory(&fs::read_to_string(&path)?)?.access_count, 42);
    assert!(
        revisions::write_locked(
            &store,
            Path::new("working/fact.md"),
            &original,
            Some(snap),
            "stale",
            &FixedClock
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn future_writer_manifest_cannot_disable_history_silently() -> Result<()> {
    let (_temp, store) = store()?;
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(store.root.join("store.json"))?)?;
    manifest["min_writer_version"] = serde_json::json!(99);
    fs::write(store.root.join("store.json"), manifest.to_string())?;
    let memory = parse_memory("---\nid: fact\n---\n# First\n")?;
    let path = store.working_dir().join("fact.md");
    assert!(revisions::write_locked(&store, &path, &memory, None, "created", &FixedClock).is_err());
    assert!(!path.exists());
    Ok(())
}

#[test]
fn source_support_is_versioned_and_replay_is_idempotent() -> Result<()> {
    let (_temp, store) = store()?;
    let request = |event: &str, finding: &str| WriteRequestV2 {
        memory_type: "codebase".into(),
        title: "Fact".into(),
        content: "A supported fact".into(),
        importance: 70,
        origin: "test".into(),
        source_session_id: "session".into(),
        source_event_id: event.into(),
        finding_key: finding.into(),
        source_kind: "tool_output".into(),
        verification_state: "verified".into(),
        fact_key: "fact".into(),
        ..Default::default()
    };
    let created = provenance::write_v2(&store, &request("e1", "f1"), &FixedClock)?;
    assert_eq!(created.status, "created");
    let id = &created.memory_ref.memory_id;
    let path = store.working_dir().join(format!("{id}.md"));
    assert_eq!(manifest(&store, id)?.entries.len(), 1);

    let supported = provenance::write_v2(&store, &request("e1", "f2"), &FixedClock)?;
    assert_eq!(supported.status, "supported");
    assert_eq!(supported.evidence_count, 1);
    let history = manifest(&store, id)?;
    assert_eq!(history.entries.len(), 2);
    let parsed = parse_memory(&fs::read_to_string(&path)?)?;
    assert_eq!(parsed.extra["evidence_count"], 1);
    assert_eq!(parsed.extra["source_event_count"], 2);
    let ledger_hash = parsed.extra["source_event_ledger_hash"].as_str().unwrap();
    assert_eq!(ledger_hash.len(), 64);

    assert_eq!(
        provenance::write_v2(&store, &request("e1", "f2"), &FixedClock)?.status,
        "duplicate"
    );
    assert_eq!(manifest(&store, id)?.entries.len(), 2);
    assert_eq!(
        parse_memory(&fs::read_to_string(&path)?)?.extra["source_event_ledger_hash"],
        ledger_hash
    );

    let independent = provenance::write_v2(&store, &request("e2", "f1"), &FixedClock)?;
    assert_eq!(independent.status, "supported");
    assert_eq!(independent.evidence_count, 2);
    assert_eq!(manifest(&store, id)?.entries.len(), 3);
    let parsed = parse_memory(&fs::read_to_string(&path)?)?;
    assert_eq!(parsed.extra["evidence_count"], 2);
    assert_eq!(parsed.extra["source_event_count"], 3);
    assert_ne!(parsed.extra["source_event_ledger_hash"], ledger_hash);
    Ok(())
}

#[test]
fn concurrent_show_keeps_source_ledger_and_semantic_revision_together() -> Result<()> {
    let (_temp, store) = store()?;
    let request = |event: usize| WriteRequestV2 {
        memory_type: "codebase".into(),
        title: "Shared fact".into(),
        content: "same body".into(),
        importance: 70,
        origin: "test".into(),
        source_session_id: "s".into(),
        source_event_id: event.to_string(),
        finding_key: "f".into(),
        source_kind: "tool_output".into(),
        verification_state: "unverified".into(),
        fact_key: "shared".into(),
        ..Default::default()
    };
    let created = provenance::write_v2(&store, &request(0), &FixedClock)?;
    std::thread::scope(|scope| -> Result<()> {
        let writer = scope.spawn(|| -> Result<()> {
            for event in 1..12 {
                provenance::write_v2(&store, &request(event), &FixedClock)?;
            }
            Ok(())
        });
        for _ in 0..24 {
            let shown = mnemosyne::api::show_v2(
                std::slice::from_ref(&store),
                &created.memory_ref.memory_id,
                None,
            )?;
            let count = shown["provenance"]["source_events"]
                .as_array()
                .unwrap()
                .len() as u64;
            assert_eq!(shown["memory"]["extra"]["source_event_count"], count);
            assert_eq!(shown["revision"]["semantic_rev"], count);
        }
        writer.join().unwrap()?;
        Ok(())
    })?;
    Ok(())
}
