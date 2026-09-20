use chrono::{DateTime, Utc};
use mnemosyne::{
    api,
    provenance::{self, Clock, MemoryRef, WriteRequestV2},
    schema::{Memory, parse_memory, serialize_memory},
    store::{self, Store},
};
use std::{fs, sync::Arc};

struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        "2026-09-20T01:02:03Z".parse().unwrap()
    }
}
fn store(root: &std::path::Path) -> Store {
    Store {
        scope: "project".into(),
        root: root.join(".mnemosyne"),
    }
}
fn request(event: &str, finding: &str) -> WriteRequestV2 {
    WriteRequestV2 {
        memory_type: "codebase".into(),
        title: "标题".into(),
        content: "中文, \"quote\" \\path".into(),
        importance: 70,
        origin: "original-tool".into(),
        source_session_id: "session".into(),
        source_event_id: event.into(),
        finding_key: finding.into(),
        source_kind: "tool_output".into(),
        verification_state: "verified".into(),
        fact_key: "fact".into(),
        ..Default::default()
    }
}

#[test]
fn explicit_upgrade_and_source_ledger() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let store = store(dir.path());
    assert_eq!(
        provenance::preview_upgrade(&store)?.status,
        "upgrade_available"
    );
    assert!(!store.root.exists());
    assert!(provenance::write_v2(&store, &request("e1", "f1"), &FixedClock).is_err());
    let manifest = provenance::upgrade_store(&store)?;
    assert_eq!(manifest.schema_version, 2);
    assert_eq!(
        manifest.min_writer_version,
        mnemosyne::provenance::WRITER_VERSION
    );
    assert_eq!(provenance::preview_upgrade(&store)?.status, "current");
    let first = provenance::write_v2(&store, &request("e1", "f1"), &FixedClock)?;
    assert_eq!(first.status, "created");
    assert_eq!(first.evidence_count, 1);
    assert_eq!(first.source_event.verification_state, "unverified");
    let md = store
        .working_dir()
        .join(format!("{}.md", first.memory_ref.memory_id));
    let text = fs::read_to_string(&md)?;
    let parsed = parse_memory(&text)?;
    assert_eq!(parsed.extra["recorded_at"], "2026-09-20T01:02:03Z");
    assert_eq!(parsed.extra["source_kind"], "tool_output");
    assert_eq!(parsed.body, "## 标题\n\n中文, \"quote\" \\path");
    let same = provenance::write_v2(&store, &request("e1", "f1"), &FixedClock)?;
    assert_eq!(same.status, "duplicate");
    assert_eq!(same.memory_ref, first.memory_ref);
    let next_finding = provenance::write_v2(&store, &request("e1", "f2"), &FixedClock)?;
    assert_eq!(next_finding.status, "supported");
    assert_eq!(next_finding.evidence_count, 1);
    let another_event = provenance::write_v2(&store, &request("e2", "f1"), &FixedClock)?;
    assert_eq!(another_event.status, "supported");
    assert_eq!(another_event.evidence_count, 2);
    let view = provenance::read_provenance(&store, &first.memory_ref.memory_id)?;
    assert_eq!(view.evidence_count, 2);
    assert_eq!(view.source_events.len(), 3);
    let mut conflict = request("e1", "f1");
    conflict.content = "different".into();
    assert!(
        provenance::write_v2(&store, &conflict, &FixedClock)
            .unwrap_err()
            .to_string()
            .contains("IDENTITY_CONFLICT")
    );
    assert!(
        api::write_entry(
            &store,
            &api::WriteRequest {
                memory_type: "codebase".into(),
                content: "old writer".into(),
                ..Default::default()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("INCOMPATIBLE_SCHEMA")
    );
    assert_eq!(store::load_memories(&store, false)?.len(), 1);
    let evidence = store
        .root
        .join("evidence")
        .join(format!("{}.json", first.memory_ref.memory_id));
    let mut pending: serde_json::Value = serde_json::from_slice(&fs::read(&evidence)?)?;
    pending["markdown_published"] = false.into();
    fs::write(&evidence, serde_json::to_vec(&pending)?)?;
    fs::remove_file(&md)?;
    assert_eq!(provenance::preview_upgrade(&store)?.status, "current");
    assert!(!md.exists(), "preview must not replay pending writes");
    assert_eq!(
        provenance::write_v2(&store, &request("e1", "f1"), &FixedClock)?.status,
        "duplicate"
    );
    assert!(md.exists());
    Ok(())
}

#[test]
fn concurrent_replay_and_stable_refs() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let a = store(&dir.path().join("a"));
    let b = store(&dir.path().join("b"));
    provenance::upgrade_store(&a)?;
    provenance::upgrade_store(&b)?;
    let a = Arc::new(a);
    let joins: Vec<_> = (0..8)
        .map(|i| {
            let store = a.clone();
            std::thread::spawn(move || {
                provenance::write_v2(
                    &store,
                    &request("e1", if i % 2 == 0 { "f1" } else { "f2" }),
                    &FixedClock,
                )
                .unwrap()
            })
        })
        .collect();
    let outcomes: Vec<_> = joins.into_iter().map(|j| j.join().unwrap()).collect();
    assert_eq!(outcomes.iter().filter(|o| o.status == "created").count(), 1);
    assert_eq!(
        outcomes.iter().filter(|o| o.status == "supported").count(),
        1
    );
    assert_eq!(store::load_memories(&a, false)?.len(), 1);
    let mut distinct = request("e1", "f3");
    distinct.fact_key.clear();
    distinct.content = "another finding".into();
    let second_memory = provenance::write_v2(&a, &distinct, &FixedClock)?;
    assert_eq!(second_memory.status, "created");
    assert_ne!(
        second_memory.memory_ref.memory_id,
        outcomes[0].memory_ref.memory_id
    );
    let first = outcomes[0].memory_ref.clone();
    let original = store::load_memories(&a, false)?
        .into_iter()
        .find(|(_, m)| m.id == first.memory_id)
        .unwrap()
        .1;
    store::write_memory(&store::working_path(&b, &original)?, &original)?;
    let old = store::find_memory(&first.memory_id, &[b.clone(), (*a).clone()], false)?.unwrap();
    assert_eq!(old.0.root, b.root); // v1 first match is preserved.
    let resolved = provenance::resolve_ref(&[b, (*a).clone()], &first)?.unwrap();
    assert_eq!(resolved.0.root, a.root);
    assert!(
        provenance::resolve_ref(
            &[(*a).clone()],
            &MemoryRef {
                store_id: uuid::Uuid::new_v4().to_string(),
                memory_id: first.memory_id
            }
        )?
        .is_none()
    );
    Ok(())
}

#[test]
fn unknown_flat_fields_round_trip_without_read_migration() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let store = store(dir.path());
    store::ensure_store(&store)?;
    let path = store.working_dir().join("legacy.md");
    let input = "---\nid: legacy\ncustom: \"中文, \\\"quote\\\" \\\\path\"\n---\n# Body\n";
    fs::write(&path, input)?;
    assert_eq!(store::load_memories(&store, false)?.len(), 1);
    assert_eq!(fs::read_to_string(&path)?, input);
    let unknown = provenance::read_provenance(&store, "legacy")?;
    assert_eq!(unknown.source_kind, "unknown");
    assert_eq!(unknown.verification_state, "unknown");
    let mut memory = parse_memory(input)?;
    memory.extra.insert("literal".into(), "true".into());
    memory.extra.insert("spaced".into(), "  中文  ".into());
    let round = parse_memory(&serialize_memory(&memory))?;
    assert_eq!(round.extra["custom"], memory.extra["custom"]);
    assert_eq!(round.extra["literal"], "true");
    assert_eq!(round.extra["spaced"], "  中文  ");
    let _ = Memory::default();
    Ok(())
}

#[test]
fn request_schema_and_payload_identity_are_strict() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let store = store(dir.path());
    provenance::upgrade_store(&store)?;
    assert!(
        serde_json::from_value::<WriteRequestV2>(serde_json::json!({
            "type":"codebase","content":"x","invented":"ignored"
        }))
        .is_err()
    );
    fs::write(store.config_path(), "[memory]\ntypes = ['codebase']\n")?;
    let mut invalid = request("e1", "f1");
    invalid.memory_type = "pitfall".into();
    assert!(
        provenance::write_v2(&store, &invalid, &FixedClock)
            .unwrap_err()
            .to_string()
            .contains("Invalid memory type")
    );
    let original = request("e1", "f1");
    provenance::write_v2(&store, &original, &FixedClock)?;
    assert_eq!(
        provenance::write_v2(
            &store,
            &WriteRequestV2 {
                source: "another retelling agent".into(),
                ..request("e1", "f1")
            },
            &FixedClock
        )?
        .status,
        "duplicate"
    );
    for changed in [
        WriteRequestV2 {
            verification_state: "evidence_attached".into(),
            ..request("e1", "f1")
        },
        WriteRequestV2 {
            source_summary: "redacted summary".into(),
            ..request("e1", "f1")
        },
    ] {
        assert!(
            provenance::write_v2(&store, &changed, &FixedClock)
                .unwrap_err()
                .to_string()
                .contains("IDENTITY_CONFLICT")
        );
    }
    Ok(())
}

#[test]
fn support_rechecks_live_markdown_and_inactive_state() -> anyhow::Result<()> {
    for case in [
        "corrected",
        "superseded",
        "deprecated",
        "expired",
        "invalidated",
        "archived",
    ] {
        let dir = tempfile::tempdir()?;
        let store = store(dir.path());
        provenance::upgrade_store(&store)?;
        let first = provenance::write_v2(&store, &request("e1", "f1"), &FixedClock)?;
        let path = store
            .working_dir()
            .join(format!("{}.md", first.memory_ref.memory_id));
        if case == "archived" {
            let archive = store.archive_dir().join("2026-09");
            fs::create_dir_all(&archive)?;
            fs::rename(&path, archive.join(path.file_name().unwrap()))?;
        } else {
            let mut memory = parse_memory(&fs::read_to_string(&path)?)?;
            match case {
                "corrected" => memory.body.push_str("\nCorrection"),
                "superseded" => memory.status = "superseded".into(),
                "deprecated" => memory.status = "deprecated".into(),
                "expired" => memory.expires = "2000-01-01".into(),
                "invalidated" => {
                    memory.extra.insert("invalidated_by".into(), "newer".into());
                }
                _ => unreachable!(),
            }
            store::write_memory(&path, &memory)?;
        }
        assert!(
            provenance::write_v2(&store, &request("e2", "f1"), &FixedClock).is_err(),
            "{case}"
        );
        assert_eq!(
            provenance::read_provenance(&store, &first.memory_ref.memory_id)?.evidence_count,
            1,
            "{case}"
        );
    }
    Ok(())
}

#[test]
fn sidecar_identity_and_read_size_are_checked() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let store = store(dir.path());
    provenance::upgrade_store(&store)?;
    let first = provenance::write_v2(&store, &request("e1", "f1"), &FixedClock)?;
    let source = store
        .root
        .join("evidence")
        .join(format!("{}.json", first.memory_ref.memory_id));
    let wrong = store.root.join("evidence").join("wrong.json");
    fs::copy(&source, &wrong)?;
    assert!(
        provenance::read_provenance(&store, &first.memory_ref.memory_id)
            .unwrap_err()
            .to_string()
            .contains("filename differs")
    );
    fs::remove_file(&wrong)?;
    let huge = fs::File::create(&wrong)?;
    huge.set_len(8 * 1024 * 1024 + 1)?;
    assert!(
        provenance::read_provenance(&store, &first.memory_ref.memory_id)
            .unwrap_err()
            .to_string()
            .contains("INPUT_TOO_LARGE")
    );
    Ok(())
}
