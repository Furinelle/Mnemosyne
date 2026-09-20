use anyhow::Result;
use mnemosyne::{
    provenance::{self, SystemClock},
    reconcile::{self, Request},
    store::Store,
};
use serde_json::json;
fn req(event: &str, value: &str, body: &str) -> Result<Request> {
    req_in_scope(event, "prod", false, value, body)
}
fn req_in_scope(
    event: &str,
    environment: &str,
    multivalued: bool,
    value: &str,
    body: &str,
) -> Result<Request> {
    Ok(serde_json::from_value(
        json!({"fact":{"subject":"service","environment":environment,"attribute":"database","multivalued":multivalued},"value":value,"write":{"type":"codebase","title":"database","content":body,"importance":70,"origin":"fixture","source_session_id":"s","source_event_id":event,"finding_key":"f","source_kind":"tool_output","verification_state":"unverified"}}),
    )?)
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
#[test]
fn independent_support_replay_and_conflicting_append() -> Result<()> {
    let (_temp, store) = store()?;
    let first = reconcile::write(
        &store,
        req("a", "PostgreSQL", "Database is PostgreSQL")?,
        &SystemClock,
    )?;
    let duplicate = reconcile::write(
        &store,
        req("a", "PostgreSQL", "Database is PostgreSQL")?,
        &SystemClock,
    )?;
    assert_eq!(duplicate["write"]["status"], "duplicate");
    assert_eq!(
        duplicate["classification"]["candidates"][0]["decision"],
        "SKIP"
    );
    let support = reconcile::write(
        &store,
        req("b", "PostgreSQL", "Database is PostgreSQL")?,
        &SystemClock,
    )?;
    assert_eq!(first["write"]["memory_ref"], support["write"]["memory_ref"]);
    assert_eq!(support["write"]["evidence_count"], 2);
    assert_eq!(
        support["classification"]["candidates"][0]["decision"],
        "SUPPORT"
    );
    let conflict = reconcile::write(
        &store,
        req("c", "SQLite", "Database is SQLite")?,
        &SystemClock,
    )?;
    assert_ne!(
        first["write"]["memory_ref"],
        conflict["write"]["memory_ref"]
    );
    assert_eq!(
        conflict["classification"]["candidates"][0]["decision"],
        "CONTRADICT"
    );
    let complement = reconcile::write(
        &store,
        req("d", "PostgreSQL", "PostgreSQL uses port 5432")?,
        &SystemClock,
    )?;
    assert_eq!(complement["write"]["status"], "created");
    assert!(
        mnemosyne::store::load_memories_unlocked(&store, false)?
            .iter()
            .all(|(_, m)| m.status == "active")
    );
    Ok(())
}

#[test]
fn classification_prioritizes_scope_cardinality_and_value() -> Result<()> {
    let cases = [
        (
            "same body, different value",
            "prod",
            false,
            "SQLite",
            "Database is PostgreSQL",
            "CONTRADICT",
        ),
        (
            "different body, same value",
            "prod",
            false,
            "PostgreSQL",
            "PostgreSQL uses port 5432",
            "REFINE",
        ),
        (
            "different environment",
            "dev",
            false,
            "SQLite",
            "Database is PostgreSQL",
            "CONTEXTUALIZE",
        ),
        (
            "multivalued attribute",
            "prod",
            true,
            "SQLite",
            "Database is PostgreSQL",
            "CREATE",
        ),
    ];
    for (i, (_, environment, multivalued, value, body, expected)) in cases.iter().enumerate() {
        let (_temp, store) = store()?;
        reconcile::write(
            &store,
            req_in_scope("a", "prod", false, "PostgreSQL", "Database is PostgreSQL")?,
            &SystemClock,
        )?;
        let report = reconcile::classify(
            &store,
            &req_in_scope(&(i + 1).to_string(), environment, *multivalued, value, body)?,
        )?;
        assert_eq!(report["candidates"][0]["decision"], *expected);
        assert_eq!(report["automatic_replacement"], false);
    }
    Ok(())
}
