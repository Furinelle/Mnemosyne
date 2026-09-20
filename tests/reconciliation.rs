use anyhow::Result;
use mnemosyne::{
    provenance::{self, SystemClock},
    reconcile::{self, Request},
    store::Store,
};
use serde_json::json;
fn req(event: &str, value: &str, body: &str) -> Result<Request> {
    Ok(serde_json::from_value(
        json!({"fact":{"subject":"service","environment":"prod","attribute":"database","multivalued":false},"value":value,"write":{"type":"codebase","title":"database","content":body,"importance":70,"origin":"fixture","source_session_id":"s","source_event_id":event,"finding_key":"f","source_kind":"tool_output","verification_state":"unverified"}}),
    )?)
}
#[test]
fn independent_support_replay_and_conflicting_append() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let store = Store {
        scope: "project".into(),
        root: temp.path().join(".mnemosyne"),
    };
    provenance::upgrade_store(&store)?;
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
