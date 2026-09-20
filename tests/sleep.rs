use anyhow::Result;
use mnemosyne::{
    api,
    checkpoint::{self, CheckpointInput},
    proposals::{Request, Target},
    provenance::{self, SystemClock, WriteRequestV2},
    sleep,
    store::Store,
};
use serde_json::json;
use std::fs;

fn store() -> Result<(tempfile::TempDir, Store)> {
    let temp = tempfile::tempdir()?;
    let store = Store {
        scope: "project".into(),
        root: temp.path().join(".mnemosyne"),
    };
    provenance::upgrade_store(&store)?;
    Ok((temp, store))
}

fn memory(store: &Store, key: &str, content: &str) -> Result<Target> {
    let outcome = provenance::write_v2(
        store,
        &WriteRequestV2 {
            memory_type: "codebase".into(),
            title: key.into(),
            content: content.into(),
            importance: 70,
            origin: "test".into(),
            source_session_id: "session".into(),
            source_event_id: key.into(),
            finding_key: key.into(),
            source_kind: "tool_output".into(),
            verification_state: "verified".into(),
            fact_key: key.into(),
            ..Default::default()
        },
        &SystemClock,
    )?;
    let shown = api::show_v2(
        std::slice::from_ref(store),
        &outcome.memory_ref.memory_id,
        None,
    )?;
    Ok(Target {
        memory_ref: outcome.memory_ref,
        expected_rev: shown["revision"]["semantic_rev"].as_u64().unwrap(),
        expected_hash: shown["revision"]["semantic_hash"].as_str().unwrap().into(),
        body: Some(format!("## {key}\n\nrevised")),
        status: None,
    })
}

fn request(target: Target) -> Request {
    Request {
        decision: "REFINE".into(),
        reason: "explicit host suggestion".into(),
        evidence: vec!["test evidence".into()],
        targets: vec![target],
    }
}

#[test]
fn rules_are_offline_metadata_only_and_redact_source_bodies() -> Result<()> {
    let (_temp, store) = store()?;
    let target = memory(&store, "secret", "api_key=not-for-report")?;
    let core = fs::read_to_string(store.core_path())?;
    let report = sleep::rules(&store, 0, None, 10, &SystemClock)?;
    let report_path = fs::read_dir(store.root.join("sleep"))?
        .map(|entry| Ok(entry?.path()))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .find(|path| path.file_name().is_some_and(|name| name != "cursor.json"))
        .unwrap();
    let raw = fs::read_to_string(report_path)?;
    assert_eq!(report["facts_modified"], false);
    assert!(report["proposal_ids"].as_array().unwrap().is_empty());
    assert!(!raw.contains("api_key") && !raw.contains("not-for-report"));
    assert_eq!(fs::read_to_string(store.core_path())?, core);
    assert_eq!(
        api::show_v2(
            std::slice::from_ref(&store),
            &target.memory_ref.memory_id,
            None
        )?["memory"]["body"],
        "## secret\n\napi_key=not-for-report"
    );
    Ok(())
}

#[test]
fn replay_is_idempotent_and_changed_source_is_stale() -> Result<()> {
    let (_temp, store) = store()?;
    let target = memory(&store, "routing", "port 8080")?;
    let batch = sleep::export(&store, 0, None, 10)?;
    let first = sleep::finish(&store, &batch, vec![request(target.clone())], &SystemClock)?;
    let again = sleep::finish(&store, &batch, vec![request(target.clone())], &SystemClock)?;
    assert_eq!(first, again);
    assert_eq!(first["proposal_ids"].as_array().unwrap().len(), 1);
    api::revise_v2(
        &store,
        &serde_json::from_value(json!({
            "memory_ref": target.memory_ref,
            "expected_rev": target.expected_rev,
            "expected_hash": target.expected_hash,
            "changes": {"body": "## routing\n\nport 9090"},
        }))?,
        &SystemClock,
    )?;
    assert!(sleep::finish(&store, &batch, vec![], &SystemClock).is_err());
    Ok(())
}

#[test]
fn failed_or_partial_proposal_import_does_not_advance_cursor() -> Result<()> {
    let (_temp, store) = store()?;
    let target = memory(&store, "database", "postgres")?;
    let batch = sleep::export(&store, 0, None, 10)?;
    let invalid = Request {
        decision: "REFINE".into(),
        reason: "invalid second request".into(),
        evidence: vec![],
        targets: vec![Target {
            body: None,
            ..target.clone()
        }],
    };
    assert!(sleep::finish(&store, &batch, vec![request(target), invalid], &SystemClock).is_err());
    assert_eq!(sleep::cursor(&store)?, json!({"next_cursor": 0}));
    Ok(())
}

#[test]
fn relation_import_remains_pending_and_never_edits_facts() -> Result<()> {
    let (_temp, store) = store()?;
    let left = memory(&store, "cause", "a")?;
    let right = memory(&store, "effect", "b")?;
    let batch = sleep::export(&store, 0, None, 10)?;
    let relation = Request {
        decision: "CAUSED_BY".into(),
        reason: "host reports causal relation".into(),
        evidence: vec!["reported only".into()],
        targets: vec![
            Target {
                body: None,
                ..left.clone()
            },
            Target {
                body: None,
                ..right.clone()
            },
        ],
    };
    let report = sleep::finish(&store, &batch, vec![relation], &SystemClock)?;
    let id = report["proposal_ids"][0].as_str().unwrap();
    assert_eq!(mnemosyne::proposals::show(&store, id)?.state, "pending");
    for target in [left, right] {
        assert!(
            api::show_v2(
                std::slice::from_ref(&store),
                &target.memory_ref.memory_id,
                None
            )?["memory"]["links"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    Ok(())
}

#[test]
fn pages_are_bounded_and_include_checkpoint_metadata_without_checkpoint_content() -> Result<()> {
    let (temp, store) = store()?;
    memory(&store, "one", "one")?;
    memory(&store, "two", "two")?;
    fs::write(temp.path().join("task.txt"), "work")?;
    checkpoint::create(
        &store,
        CheckpointInput {
            task_id: "task".into(),
            goal: "checkpoint-secret-goal".into(),
            scoped_paths: vec!["task.txt".into()],
            expires: "2099-01-01T00:00:00Z".into(),
            ..Default::default()
        },
        &SystemClock,
    )?;
    let first = sleep::export(&store, 0, None, 1)?;
    assert!(first.partial && first.next_cursor == 1);
    let full = sleep::export(&store, 0, None, 10)?;
    assert!(full.inputs.iter().any(|input| input.kind == "checkpoint"));
    assert!(sleep::export(&store, 1, Some("wrong"), 1).is_err());
    assert!(sleep::export(&store, 0, None, 101).is_err());
    let report = sleep::finish(&store, &full, vec![], &SystemClock)?;
    assert!(!serde_json::to_string(&report)?.contains("checkpoint-secret-goal"));
    Ok(())
}

#[test]
fn oversized_input_or_output_is_rejected_without_a_cursor_write() -> Result<()> {
    let (_temp, store) = store()?;
    let path = store.working_dir().join("oversized.md");
    fs::write(&path, "x".repeat(128 * 1024 + 1))?;
    assert!(sleep::export(&store, 0, None, 1).is_err());
    assert_eq!(sleep::cursor(&store)?, json!({"next_cursor": 0}));
    Ok(())
}
