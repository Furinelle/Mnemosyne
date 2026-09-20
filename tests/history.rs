use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use mnemosyne::{
    history,
    provenance::{self, Clock, MemoryRef, WriteRequestV2},
    revisions,
    schema::{Link, Memory, parse_memory, serialize_memory},
    search,
    store::{self, Store},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
struct FixedClock(DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

fn at(value: &str) -> FixedClock {
    FixedClock(value.parse().expect("valid UTC fixture timestamp"))
}

fn new_store() -> Result<(tempfile::TempDir, Store)> {
    let temp = tempfile::tempdir()?;
    let store = Store {
        scope: "project".into(),
        root: temp.path().join(".mnemosyne"),
    };
    store::ensure_store(&store)?;
    provenance::upgrade_store(&store)?;
    Ok((temp, store))
}

fn request(event: &str, fact_key: &str, content: &str, expires: &str) -> WriteRequestV2 {
    WriteRequestV2 {
        memory_type: "codebase".into(),
        title: "History fixture".into(),
        content: content.into(),
        importance: 70,
        expires: expires.into(),
        origin: "r08-fixture".into(),
        source_session_id: "r08-session".into(),
        source_event_id: event.into(),
        finding_key: event.into(),
        source_kind: "code_observation".into(),
        verification_state: "verified".into(),
        fact_key: fact_key.into(),
        ..Default::default()
    }
}

fn create_source(
    store: &Store,
    clock: &FixedClock,
    event: &str,
    fact_key: &str,
    content: &str,
    expires: &str,
) -> Result<MemoryRef> {
    let outcome = provenance::write_v2(store, &request(event, fact_key, content, expires), clock)?;
    assert_eq!(outcome.status, "created");
    Ok(outcome.memory_ref)
}

fn support_source(
    store: &Store,
    clock: &FixedClock,
    event: &str,
    fact_key: &str,
    content: &str,
    expires: &str,
) -> Result<()> {
    let outcome = provenance::write_v2(store, &request(event, fact_key, content, expires), clock)?;
    assert_eq!(outcome.status, "supported");
    Ok(())
}

fn path(store: &Store, id: &str) -> PathBuf {
    store.working_dir().join(format!("{id}.md"))
}

fn load(store: &Store, id: &str) -> Result<Memory> {
    parse_memory(&fs::read_to_string(path(store, id))?)
}

fn rewrite(store: &Store, memory: &Memory, clock: &FixedClock, reason: &str) -> Result<()> {
    let path = path(store, &memory.id);
    let expected = revisions::snapshot(store, &path)?;
    // Read-only locking avoids the legacy observer's SystemClock side effect;
    // this helper supplies every history timestamp explicitly.
    let _lock = store::lock_store_read_only(store)?;
    revisions::write_locked(store, &path, memory, Some(expected), reason, clock)
}

fn create_plain(store: &Store, memory: &Memory, clock: &FixedClock) -> Result<()> {
    let path = path(store, &memory.id);
    let _lock = store::lock_store_read_only(store)?;
    revisions::write_locked(store, &path, memory, None, "created", clock)
}

fn item<'a>(result: &'a Value, id: &str) -> &'a Value {
    result["items"]
        .as_array()
        .expect("history search items array")
        .iter()
        .find(|candidate| candidate["memory_ref"]["memory_id"] == id)
        .unwrap_or_else(|| panic!("missing history item {id}: {result}"))
}

fn assert_utc(value: &Value, expected: DateTime<Utc>) {
    let parsed = DateTime::parse_from_rfc3339(value.as_str().expect("RFC3339 timestamp"))
        .expect("parse history timestamp");
    assert_eq!(parsed.with_timezone(&Utc), expected);
    assert_eq!(parsed.offset().local_minus_utc(), 0);
}

fn snapshot_tree(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                bail!("fixture unexpectedly contains symlink: {}", path.display());
            }
            if metadata.is_dir() {
                visit(root, &path, files)?;
            } else if metadata.is_file() {
                files.insert(path.strip_prefix(root)?.to_path_buf(), fs::read(path)?);
            }
        }
        Ok(())
    }

    let mut files = BTreeMap::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

#[test]
fn r08_a01_search_uses_half_open_revision_windows_and_list_show() -> Result<()> {
    let (_temp, store) = new_store()?;
    let t1 = at("2099-09-20T00:00:00Z");
    let t2 = at("2099-09-20T06:00:00Z");
    let original = Memory {
        id: "shared-r08".into(),
        memory_type: "codebase".into(),
        source: "fixture".into(),
        strength: 70,
        created: "2099-09-20".into(),
        last_accessed: "2099-09-20".into(),
        status: "active".into(),
        canonical_summary: "History fixture: The service uses PostgreSQL".into(),
        injection_summary: "History fixture: The service uses PostgreSQL".into(),
        body: "## History fixture\n\nThe service uses PostgreSQL".into(),
        expires: "2099-12-31".into(),
        ..Default::default()
    };
    create_plain(&store, &original, &t1)?;
    let id = original.id.as_str();
    let store_id = provenance::read_manifest(&store)?.unwrap().store_id;

    let mut sqlite = load(&store, id)?;
    sqlite.body = "## History fixture\n\nThe service uses SQLite".into();
    sqlite.canonical_summary = "History fixture: The service uses SQLite".into();
    sqlite.injection_summary = sqlite.canonical_summary.clone();
    rewrite(&store, &sqlite, &t2, "correction")?;

    let first = history::search(
        std::slice::from_ref(&store),
        "PostgreSQL",
        "2099-09-20T00:00:00Z",
        10,
        "codebase",
        false,
        false,
    )?;
    assert_eq!(first["coverage"], "complete_for_known_records");
    assert!(first["unknown"].as_array().unwrap().is_empty());
    assert_utc(&first["as_of"], t1.0);
    let old = item(&first, id);
    assert_eq!(old["revision"], 1);
    assert!(
        old["memory"]["body"]
            .as_str()
            .unwrap()
            .contains("PostgreSQL")
    );
    assert_eq!(old["system_until"], t2.0.to_rfc3339());
    assert!(
        old["provenance"]["source_events"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let between = history::search(
        std::slice::from_ref(&store),
        "PostgreSQL",
        "2099-09-20T05:59:59Z",
        10,
        "codebase",
        false,
        false,
    )?;
    assert_eq!(item(&between, id)["revision"], 1);

    let second = history::search(
        std::slice::from_ref(&store),
        "SQLite",
        "2099-09-20T06:00:00Z",
        10,
        "codebase",
        false,
        false,
    )?;
    let current = item(&second, id);
    assert_eq!(current["revision"], 2);
    assert!(
        current["memory"]["body"]
            .as_str()
            .unwrap()
            .contains("SQLite")
    );
    assert_utc(&current["system_from"], t2.0);

    let date_only = history::search(
        std::slice::from_ref(&store),
        "PostgreSQL",
        "2099-09-20",
        10,
        "codebase",
        false,
        false,
    )?;
    assert_utc(&date_only["as_of"], t1.0);
    assert_eq!(item(&date_only, id)["revision"], 1);

    let listed = history::list(std::slice::from_ref(&store), id, Some(store_id.as_str()))?;
    assert_eq!(listed["coverage"], "recorded_history");
    assert_eq!(listed["entries"].as_array().unwrap().len(), 2);
    assert_eq!(listed["entries"][0]["revision"], 1);
    assert_eq!(listed["entries"][1]["revision"], 2);

    let shown = history::show(std::slice::from_ref(&store), id, Some(store_id.as_str()), 1)?;
    assert_eq!(shown["revision"], 1);
    assert!(
        shown["memory"]["body"]
            .as_str()
            .unwrap()
            .contains("PostgreSQL")
    );
    Ok(())
}

#[test]
fn r08_a02_expiry_is_evaluated_on_the_query_calendar_date() -> Result<()> {
    let (_temp, store) = new_store()?;
    let clock = at("2099-09-18T12:00:00Z");
    let reference = create_source(
        &store,
        &clock,
        "expired",
        "expired-fact",
        "An expired fact was valid through its expiry date",
        "2099-09-19",
    )?;
    let id = reference.memory_id.as_str();

    let before = history::search(
        std::slice::from_ref(&store),
        "expired fact",
        "2099-09-19",
        10,
        "codebase",
        false,
        false,
    )?;
    assert_eq!(before["expiry_calendar_date"], "2099-09-19");
    assert_eq!(before["items"].as_array().unwrap().len(), 1);
    let before_item = item(&before, id);
    assert_eq!(before_item["expired"], false);
    assert_eq!(before_item["expiry_calendar_date"], "2099-09-19");
    assert_eq!(before_item["memory"]["expires"], "2099-09-19");

    // The UTC instant crosses into the next UTC date, while the explicit
    // -02:00 calendar date is still the inclusive expiry date.
    let offset_before = history::search(
        std::slice::from_ref(&store),
        "expired fact",
        "2099-09-19T23:30:00-02:00",
        10,
        "codebase",
        false,
        false,
    )?;
    assert_utc(&offset_before["as_of"], at("2099-09-20T01:30:00Z").0);
    assert_eq!(offset_before["expiry_calendar_date"], "2099-09-19");
    assert_eq!(offset_before["items"].as_array().unwrap().len(), 1);

    let offset_after = history::search(
        std::slice::from_ref(&store),
        "expired fact",
        "2099-09-20T00:30:00+02:00",
        10,
        "codebase",
        false,
        false,
    )?;
    assert_utc(&offset_after["as_of"], at("2099-09-19T22:30:00Z").0);
    assert_eq!(offset_after["expiry_calendar_date"], "2099-09-20");
    assert!(offset_after["items"].as_array().unwrap().is_empty());

    let included = history::search(
        std::slice::from_ref(&store),
        "expired fact",
        "2099-09-20T00:30:00+02:00",
        10,
        "codebase",
        true,
        false,
    )?;
    assert_eq!(included["items"].as_array().unwrap().len(), 1);
    assert_eq!(item(&included, id)["expired"], true);
    Ok(())
}

#[test]
fn r08_a03_before_legacy_adoption_is_partial_and_unknown() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let store = Store {
        scope: "project".into(),
        root: temp.path().join(".mnemosyne"),
    };
    store::ensure_store(&store)?;
    let legacy = Memory {
        id: "legacy-r08".into(),
        memory_type: "codebase".into(),
        source: "legacy".into(),
        created: "2000-01-01".into(),
        last_accessed: "2000-01-01".into(),
        status: "active".into(),
        body: "## Legacy bridge\n\nThe adopted legacy fact has no earlier snapshot".into(),
        ..Default::default()
    };
    fs::write(path(&store, &legacy.id), serialize_memory(&legacy))?;
    provenance::upgrade_store(&store)?;

    let adopted_at = at("2099-09-20T05:00:00Z");
    let _lock = store::lock_store_read_only(&store)?;
    revisions::observe_store(&store, &adopted_at)?;
    drop(_lock);

    let before = history::search(
        std::slice::from_ref(&store),
        "legacy bridge",
        "2000-01-01",
        10,
        "codebase",
        true,
        false,
    )?;
    assert_eq!(before["coverage"], "partial_history");
    assert!(before["items"].as_array().unwrap().is_empty());
    assert_eq!(before["unknown"].as_array().unwrap().len(), 1);
    assert_eq!(before["unknown"][0]["memory_id"], legacy.id);
    assert_eq!(before["unknown"][0]["reason"], "Before coverage_start");

    let at_adoption = history::search(
        std::slice::from_ref(&store),
        "legacy bridge",
        "2099-09-20T05:00:00Z",
        10,
        "codebase",
        true,
        false,
    )?;
    assert_eq!(at_adoption["coverage"], "complete_for_known_records");
    let adopted = item(&at_adoption, &legacy.id);
    assert_eq!(adopted["revision"], 1);
    assert_eq!(adopted["memory"]["body"], legacy.body);
    Ok(())
}

#[test]
fn r08_a04_historical_results_exclude_future_supersedes_vectors_and_events() -> Result<()> {
    let (_temp, store) = new_store()?;
    let t1 = at("2099-09-20T00:00:00Z");
    let t2 = at("2099-09-20T06:00:00Z");
    let t3 = at("2099-09-20T12:00:00Z");
    let t4 = at("2099-09-20T18:00:00Z");
    let reference = create_source(
        &store,
        &t1,
        "e1",
        "replaceable-fact",
        "The database engine is PostgreSQL",
        "2099-12-31",
    )?;
    support_source(
        &store,
        &t2,
        "e2",
        "replaceable-fact",
        "The database engine is PostgreSQL",
        "2099-12-31",
    )?;
    let future = create_source(
        &store,
        &t3,
        "future",
        "future-fact",
        "Future replacement body must stay out of old history",
        "2099-12-31",
    )?;

    let id = reference.memory_id.as_str();
    let mut superseded = load(&store, id)?;
    superseded.status = "superseded".into();
    superseded
        .extra
        .insert("invalidated_by".into(), json!(future.memory_id));
    superseded.links.push(Link {
        id: future.memory_id.clone(),
        rel: "superseded_by".into(),
    });
    rewrite(&store, &superseded, &t4, "relation")?;

    let mut future_current = load(&store, &future.memory_id)?;
    future_current.links.push(Link {
        id: reference.memory_id.clone(),
        rel: "supersedes".into(),
    });
    rewrite(
        &store,
        &future_current,
        &at("2099-09-20T19:00:00Z"),
        "relation",
    )?;

    search::reindex_store(&store, true)?;
    fs::write(
        store.root.join("vectors-rust.sqlite"),
        b"garbage vector result: PostgreSQL future replacement",
    )?;

    let old = history::search(
        std::slice::from_ref(&store),
        "PostgreSQL",
        "2099-09-20T05:00:00Z",
        10,
        "codebase",
        false,
        false,
    )?;
    let old_item = item(&old, id);
    assert_eq!(old_item["revision"], 1);
    assert_eq!(old_item["memory"]["status"], "active");
    assert_eq!(old_item["memory"]["extra"]["source_event_count"], 1);
    let events = old_item["provenance"]["source_events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["source_event_id"], "e1");
    assert_eq!(old_item["memory"]["extra"]["source_event_id"], "e1");
    assert!(
        !old_item["memory"]["body"]
            .as_str()
            .unwrap()
            .contains("Future replacement body")
    );
    assert!(old_item["memory"]["links"].as_array().unwrap().is_empty());
    assert_eq!(old_item["memory"]["extra"].get("invalidated_by"), None);

    let current = history::search(
        std::slice::from_ref(&store),
        "PostgreSQL",
        "2099-09-20T06:00:00Z",
        10,
        "codebase",
        false,
        false,
    )?;
    let current_item = item(&current, id);
    assert_eq!(current_item["revision"], 2);
    assert_eq!(current_item["memory"]["extra"]["source_event_count"], 2);
    assert_eq!(
        current_item["provenance"]["source_events"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        current_item["memory"]["links"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    Ok(())
}

#[test]
fn r08_a05_history_reads_do_not_change_canonical_usage_expiry_or_caches() -> Result<()> {
    let (_temp, store) = new_store()?;
    let clock = at("2099-09-20T00:00:00Z");
    let reference = create_source(
        &store,
        &clock,
        "stable",
        "stable-fact",
        "Canonical history fixture",
        "2099-12-31",
    )?;
    let id = reference.memory_id.as_str();
    let mut heated = load(&store, id)?;
    heated.strength = 41;
    heated.last_accessed = "2099-09-19".into();
    heated.access_count = 7;
    heated.canonical_summary = "canonical fixture must not be rewritten".into();
    heated.injection_summary = heated.canonical_summary.clone();
    rewrite(&store, &heated, &at("2099-09-20T01:00:00Z"), "heat")?;
    search::reindex_store(&store, true)?;
    fs::write(store.root.join("vectors-rust.sqlite"), b"cache fixture")?;

    let before = snapshot_tree(&store.root)?;
    let searched = history::search(
        std::slice::from_ref(&store),
        "Canonical history fixture",
        "2099-09-20T02:00:00Z",
        10,
        "codebase",
        false,
        false,
    )?;
    let _listed = history::list(
        std::slice::from_ref(&store),
        id,
        Some(reference.store_id.as_str()),
    )?;
    let _shown = history::show(
        std::slice::from_ref(&store),
        id,
        Some(reference.store_id.as_str()),
        1,
    )?;
    assert_eq!(searched["items"].as_array().unwrap().len(), 1);
    let after = snapshot_tree(&store.root)?;
    assert_eq!(after, before, "history reads changed a store file");

    let current = load(&store, id)?;
    assert_eq!(current.strength, 41);
    assert_eq!(current.last_accessed, "2099-09-19");
    assert_eq!(current.access_count, 7);
    assert_eq!(current.expires, "2099-12-31");
    assert_eq!(
        current.canonical_summary,
        "canonical fixture must not be rewritten"
    );
    assert_eq!(current.extra["source_event_count"], 1);
    Ok(())
}
