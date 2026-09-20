use anyhow::Result;
use mnemosyne::{
    context::{approx_tokens, prep_bundle},
    store::{Store, ensure_store},
};
use serde_json::json;

#[test]
fn bundle_budget_includes_core_checkpoint_warning_and_hint() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let store = Store {
        scope: "global".into(),
        root: temp.path().join("store"),
    };
    ensure_store(&store)?;
    std::fs::write(store.core_path(), "Required core")?;
    let checkpoint = json!({"kind":"checkpoint","id":"task-1","title":"Handoff","scope":"task","summary":"Resume at step two","source":"local checkpoint","warnings":["unverified result"]});
    let bundle = prep_bundle(
        std::slice::from_ref(&store),
        "",
        3,
        "cli",
        Some(1000),
        std::slice::from_ref(&checkpoint),
    )?;
    assert_eq!(bundle.version, 1);
    assert_eq!(bundle.budget_mode, "estimated");
    assert_eq!(bundle.estimated_tokens, approx_tokens(&bundle.context));
    assert!(bundle.context.contains("Required core"));
    assert!(bundle.context.contains("local checkpoint"));
    assert!(bundle.context.contains("Handoff"));
    assert!(bundle.context.contains("Warning: unverified result"));
    assert!(bundle.context.contains("Reporting new findings"));
    assert_eq!(bundle.items[0]["kind"], "checkpoint");
    assert_eq!(bundle.selected[0]["reason"], "ranked_relevance");
    assert!(bundle.estimated_tokens <= 1000);
    assert!(
        prep_bundle(&[store], "", 3, "cli", Some(1), &[checkpoint])
            .unwrap_err()
            .to_string()
            .contains("BUDGET_TOO_SMALL")
    );
    Ok(())
}

#[test]
fn checkpoint_next_action_is_never_lost_to_memory_summary_truncation() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let store = Store {
        scope: "project".into(),
        root: temp.path().join(".mnemosyne"),
    };
    ensure_store(&store)?;
    let summary = format!(
        "Task Quartz-J9: {}. Next: Run migration verification for Quartz-J9",
        "Long reviewed handoff detail. ".repeat(12)
    );
    let checkpoint =
        json!({"kind":"checkpoint","id":"task-1","summary":summary,"warnings":["reported_only"]});
    let bundle = prep_bundle(
        std::slice::from_ref(&store),
        "",
        3,
        "cli",
        Some(2000),
        std::slice::from_ref(&checkpoint),
    )?;
    assert!(
        bundle
            .context
            .contains("Next: Run migration verification for Quartz-J9")
    );
    assert!(bundle.estimated_tokens <= 2000);
    let tiny = prep_bundle(&[store], "", 3, "cli", Some(120), &[checkpoint]);
    if let Ok(bundle) = tiny {
        assert!(
            bundle.items.is_empty(),
            "omit the whole checkpoint instead of a misleading partial handoff"
        );
        assert!(!bundle.context.contains("Task Quartz-J9"));
    }
    Ok(())
}
