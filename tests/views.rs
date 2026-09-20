use anyhow::Result;
use chrono::{DateTime, Utc};
use mnemosyne::{
    provenance::{self, Clock, WriteRequestV2},
    revisions,
    schema::parse_memory,
    store::{self, Store},
    views::{self, ViewRequest, ViewSourceRef},
};
use std::fs;

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
    provenance::upgrade_store(&store)?;
    Ok((temp, store))
}

fn memory(store: &Store, key: &str) -> Result<ViewSourceRef> {
    let outcome = provenance::write_v2(
        store,
        &WriteRequestV2 {
            memory_type: "codebase".into(),
            title: key.into(),
            content: format!("fact {key}"),
            importance: 70,
            origin: "test".into(),
            source_session_id: "s".into(),
            source_event_id: key.into(),
            finding_key: key.into(),
            source_kind: "tool_output".into(),
            verification_state: "verified".into(),
            fact_key: key.into(),
            ..Default::default()
        },
        &FixedClock,
    )?;
    let path = store
        .working_dir()
        .join(format!("{}.md", outcome.memory_ref.memory_id));
    Ok(ViewSourceRef {
        memory_ref: outcome.memory_ref,
        revision: revisions::snapshot(store, &path)?.semantic_rev,
    })
}

#[test]
fn generated_index_is_traceable_not_evidence_and_stales_on_source_change() -> Result<()> {
    let (_temp, store) = store()?;
    let source = memory(&store, "routing")?;
    let core_before = fs::read_to_string(store.core_path())?;
    let outcome = views::generate(
        &store,
        &ViewRequest {
            name: "architecture".into(),
            references: vec![source.clone()],
            allow_partial: false,
        },
        &FixedClock,
    )?;
    assert_eq!(outcome.status, "current");
    assert_eq!(outcome.evidence_count, 0);
    let text = fs::read_to_string(&outcome.path)?;
    assert!(text.contains("offline from explicit current source revisions"));
    assert!(!text.contains("fact routing"));
    assert_eq!(fs::read_to_string(store.core_path())?, core_before);
    let path = store
        .working_dir()
        .join(format!("{}.md", source.memory_ref.memory_id));
    let mut changed = parse_memory(&fs::read_to_string(&path)?)?;
    changed.body.push_str("\nchanged");
    let snapshot = revisions::snapshot(&store, &path)?;
    let _lock = store::lock_store(&store)?;
    revisions::write_locked(
        &store,
        &path,
        &changed,
        Some(snapshot),
        "correction",
        &FixedClock,
    )?;
    drop(_lock);
    assert!(views::inspect(&store, "architecture")?.stale);
    Ok(())
}

#[test]
fn missing_refs_are_explicitly_partial_and_managed_edits_conflict() -> Result<()> {
    let (_temp, store) = store()?;
    let source = memory(&store, "testing")?;
    let missing = ViewSourceRef {
        memory_ref: source.memory_ref.clone(),
        revision: source.revision + 1,
    };
    assert!(
        views::generate(
            &store,
            &ViewRequest {
                name: "testing".into(),
                references: vec![missing.clone()],
                allow_partial: false
            },
            &FixedClock
        )
        .is_err()
    );
    let partial = views::generate(
        &store,
        &ViewRequest {
            name: "testing".into(),
            references: vec![missing],
            allow_partial: true,
        },
        &FixedClock,
    )?;
    assert_eq!(partial.status, "partial");
    assert!(!partial.metadata.known_gaps.is_empty());
    fs::write(&partial.path, "hand edit")?;
    assert!(
        views::generate(
            &store,
            &ViewRequest {
                name: "testing".into(),
                references: vec![source],
                allow_partial: false
            },
            &FixedClock
        )
        .unwrap_err()
        .to_string()
        .contains("VIEW_UNMANAGED")
    );
    Ok(())
}

#[test]
fn unsafe_names_and_changed_managed_views_are_rejected() -> Result<()> {
    let (_temp, store) = store()?;
    let source = memory(&store, "pitfall")?;
    let request = ViewRequest {
        name: "pitfalls".into(),
        references: vec![source],
        allow_partial: false,
    };
    let view = views::generate(&store, &request, &FixedClock)?;
    let changed = fs::read_to_string(&view.path)?.replace("Sources", "Edited sources");
    fs::write(&view.path, changed)?;
    assert!(
        views::generate(&store, &request, &FixedClock)
            .unwrap_err()
            .to_string()
            .contains("VIEW_CONFLICT")
    );
    assert!(views::inspect(&store, "../core").is_err());
    Ok(())
}

#[test]
fn checked_view_can_be_explicitly_unmanaged() -> Result<()> {
    let (_temp, store) = store()?;
    let source = memory(&store, "manual")?;
    let request = ViewRequest {
        name: "manual".into(),
        references: vec![source],
        allow_partial: false,
    };
    views::generate(&store, &request, &FixedClock)?;
    assert_eq!(views::unmanage(&store, "manual")?.status, "unmanaged");
    assert!(views::inspect(&store, "manual")?.stale);
    assert!(
        views::generate(&store, &request, &FixedClock)
            .unwrap_err()
            .to_string()
            .contains("VIEW_UNMANAGED")
    );
    Ok(())
}
