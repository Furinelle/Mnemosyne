use anyhow::Result;
use chrono::{DateTime, Utc};
use mnemosyne::{
    api,
    provenance::{self, Clock},
    revisions::HistoryManifest,
    schema::Memory,
    store::{self, Store},
};
use serde_json::json;
use std::fs;

struct FixedClock(&'static str);
impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0.parse().unwrap()
    }
}

fn store(mode: &str) -> Result<(tempfile::TempDir, Store)> {
    let temp = tempfile::tempdir()?;
    let store = Store {
        scope: "project".into(),
        root: temp.path().join(".mnemosyne"),
    };
    store::ensure_store(&store)?;
    fs::write(
        store.config_path(),
        format!(
            "[thresholds]\ndecay_mode = '{mode}'\ndecay_per_run = 2\narchive_strength = 5\ndeprecated_strength = 3\n"
        ),
    )?;
    Ok((temp, store))
}

fn memory(id: &str, strength: i64) -> Memory {
    Memory {
        id: id.into(),
        memory_type: "codebase".into(),
        source: "fixture".into(),
        strength,
        created: "2000-01-01".into(),
        last_accessed: "2000-01-01".into(),
        status: "active".into(),
        body: format!("## {id}\n\nfixture"),
        injection_summary: id.into(),
        ..Default::default()
    }
}

fn put(store: &Store, memory: &Memory) -> Result<()> {
    store::write_memory(&store::working_path(store, memory)?, memory)
}

fn get(store: &Store, id: &str) -> Result<Memory> {
    Ok(store::load_memories_unlocked(store, true)?
        .into_iter()
        .find(|(_, memory)| memory.id == id)
        .unwrap()
        .1)
}

#[test]
fn per_day_initializes_once_and_per_run_remains_per_invocation() -> Result<()> {
    let (_temp, per_day) = store("per_day")?;
    put(&per_day, &memory("day", 10))?;
    let first = FixedClock("2026-09-20T10:00:00Z");
    api::maintain_at(std::slice::from_ref(&per_day), false, &first)?;
    let initialized = get(&per_day, "day")?;
    assert_eq!(initialized.strength, 10, "old records start at activation");
    assert_eq!(initialized.extra["last_maintained_at"], "2026-09-20");
    let raw = fs::read(per_day.working_dir().join("day.md"))?;
    for _ in 0..9 {
        api::maintain_at(std::slice::from_ref(&per_day), false, &first)?;
    }
    assert_eq!(fs::read(per_day.working_dir().join("day.md"))?, raw);
    api::maintain_at(
        std::slice::from_ref(&per_day),
        false,
        &FixedClock("2026-09-22T00:00:00Z"),
    )?;
    assert_eq!(get(&per_day, "day")?.strength, 6);

    let (_temp, per_run) = store("per_run")?;
    put(&per_run, &memory("run", 10))?;
    api::maintain_at(std::slice::from_ref(&per_run), false, &first)?;
    api::maintain_at(std::slice::from_ref(&per_run), false, &first)?;
    assert_eq!(get(&per_run, "run")?.strength, 6);
    Ok(())
}

#[test]
fn per_day_clamps_clock_rollback_and_keeps_pinned_validity_policy() -> Result<()> {
    let (_temp, store) = store("per_day")?;
    put(&store, &memory("old", 10))?;
    api::maintain_at(
        std::slice::from_ref(&store),
        false,
        &FixedClock("2026-09-20T00:00:00Z"),
    )?;
    api::maintain_at(
        std::slice::from_ref(&store),
        false,
        &FixedClock("2026-09-19T00:00:00Z"),
    )?;
    assert_eq!(get(&store, "old")?.strength, 10);
    api::maintain_at(
        std::slice::from_ref(&store),
        false,
        &FixedClock("2026-09-21T00:00:00Z"),
    )?;
    assert_eq!(get(&store, "old")?.strength, 8);

    let mut pinned = memory("pinned", 1);
    pinned.extra.insert("pinned".into(), json!(true));
    pinned
        .extra
        .insert("verification_state".into(), json!("unverified"));
    put(&store, &pinned)?;
    api::maintain_at(
        std::slice::from_ref(&store),
        false,
        &FixedClock("2026-09-21T00:00:00Z"),
    )?;
    let pinned = get(&store, "pinned")?;
    assert_eq!(pinned.status, "active");
    assert_eq!(pinned.extra["verification_state"], "unverified");
    assert!(store.working_dir().join("pinned.md").exists());

    let mut expired = memory("expired", 1);
    expired.expires = "2026-09-20".into();
    expired.extra.insert("pinned".into(), json!(true));
    put(&store, &expired)?;
    api::maintain_at(
        std::slice::from_ref(&store),
        false,
        &FixedClock("2026-09-21T00:00:00Z"),
    )?;
    assert!(!store.working_dir().join("expired.md").exists());

    let mut superseded = memory("superseded", 1);
    superseded.status = "superseded".into();
    superseded.extra.insert("pinned".into(), json!(true));
    put(&store, &superseded)?;
    api::maintain_at(
        std::slice::from_ref(&store),
        false,
        &FixedClock("2026-09-21T00:00:00Z"),
    )?;
    assert!(!store.working_dir().join("superseded.md").exists());
    assert_eq!(get(&store, "superseded")?.status, "superseded");

    let mut deprecated = memory("deprecated", 90);
    deprecated.status = "deprecated".into();
    deprecated.access_count = 10;
    put(&store, &deprecated)?;
    let report = api::maintain_at(
        std::slice::from_ref(&store),
        false,
        &FixedClock("2026-09-21T00:00:00Z"),
    )?;
    assert!(report["core_candidates"].as_array().unwrap().is_empty());
    Ok(())
}

#[test]
fn decay_configuration_never_grants_heat_or_overflows() -> Result<()> {
    let (_temp, negative) = store("per_day")?;
    fs::write(
        negative.config_path(),
        "[thresholds]\ndecay_mode='per_day'\ndecay_per_run=-2\narchive_strength=-100\n",
    )?;
    put(&negative, &memory("negative", 10))?;
    api::maintain_at(
        std::slice::from_ref(&negative),
        false,
        &FixedClock("2026-09-20T00:00:00Z"),
    )?;
    api::maintain_at(
        std::slice::from_ref(&negative),
        false,
        &FixedClock("2026-09-21T00:00:00Z"),
    )?;
    assert_eq!(get(&negative, "negative")?.strength, 10);

    let (_temp, huge) = store("per_day")?;
    fs::write(
        huge.config_path(),
        "[thresholds]\ndecay_mode='per_day'\ndecay_per_run=9223372036854775807\narchive_strength=-100\n",
    )?;
    put(&huge, &memory("huge", 10))?;
    api::maintain_at(
        std::slice::from_ref(&huge),
        false,
        &FixedClock("2026-09-20T00:00:00Z"),
    )?;
    api::maintain_at(
        std::slice::from_ref(&huge),
        false,
        &FixedClock("2026-09-22T00:00:00Z"),
    )?;
    assert_eq!(get(&huge, "huge")?.strength, 10i64.saturating_sub(i64::MAX));
    Ok(())
}

#[test]
fn dry_run_leaves_history_unchanged_and_commit_adds_one_revision() -> Result<()> {
    let (_temp, store) = store("per_day")?;
    put(&store, &memory("history", 1))?;
    provenance::upgrade_store(&store)?;
    drop(store::lock_store(&store)?);
    let manifest = |store: &Store| -> Result<HistoryManifest> {
        Ok(serde_json::from_slice(&fs::read(
            store.root.join("history/history/manifest.json"),
        )?)?)
    };
    let before_memory = fs::read(store.working_dir().join("history.md"))?;
    let before_history = manifest(&store)?;
    api::maintain_at(
        std::slice::from_ref(&store),
        true,
        &FixedClock("2026-09-20T00:00:00Z"),
    )?;
    assert_eq!(
        fs::read(store.working_dir().join("history.md"))?,
        before_memory
    );
    assert_eq!(
        manifest(&store)?.entries.len(),
        before_history.entries.len()
    );

    api::maintain_at(
        std::slice::from_ref(&store),
        false,
        &FixedClock("2026-09-20T00:00:00Z"),
    )?;
    assert_eq!(
        manifest(&store)?.entries.len(),
        before_history.entries.len() + 1
    );
    assert!(!store.working_dir().join("history.md").exists());
    Ok(())
}
