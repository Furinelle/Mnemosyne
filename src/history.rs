//! Read-only system-time snapshots. No current retrieval lane is consulted.
use crate::{
    provenance::{SourceEvent, read_manifest},
    revisions::{self, HistoryManifest},
    schema::Memory,
    store::{Store, load_memories_unlocked, lock_store_existing_read_only},
};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::Path};

fn time(value: &str) -> Result<DateTime<FixedOffset>> {
    if value.len() == 10 {
        let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")?;
        Ok(date.and_hms_opt(0, 0, 0).unwrap().and_utc().fixed_offset())
    } else {
        Ok(DateTime::parse_from_rfc3339(value)
            .context("Expected RFC3339 with offset, or YYYY-MM-DD in UTC")?)
    }
}
fn safe_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "Invalid memory ID"
    );
    Ok(())
}
fn checked(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) => ensure!(!m.file_type().is_symlink(), "Symlink history target"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    };
    Ok(())
}
fn ready(store: &Store) -> Result<bool> {
    checked(&store.root)?;
    if !store.root.exists() {
        return Ok(false);
    }
    // Never create a lock file, repair a journal or adopt current Markdown on reads.
    let lock = store.root.join(".lock");
    checked(&lock)?;
    Ok(lock.is_file())
}
fn require_settled(store: &Store) -> Result<()> {
    checked(&store.root.join(".relations-operation.json"))?;
    ensure!(
        !store.root.join(".relations-operation.json").exists(),
        "PENDING_RECOVERY: historical read will not recover a mutation"
    );
    if let Some(m) = read_manifest(store)? {
        ensure!(
            m.schema_version == 2 && m.min_writer_version <= crate::provenance::WRITER_VERSION,
            "INCOMPATIBLE_SCHEMA"
        );
    }
    Ok(())
}
fn ids(store: &Store) -> Result<BTreeSet<String>> {
    let mut ids = BTreeSet::new();
    let root = store.root.join("history");
    checked(&root)?;
    if root.exists() {
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            checked(&entry.path())?;
            if entry.file_type()?.is_dir() {
                let id = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("Invalid history filename"))?;
                safe_id(&id)?;
                ids.insert(id);
            }
        }
    }
    for (_, memory) in load_memories_unlocked(store, true)? {
        ids.insert(memory.id);
    }
    Ok(ids)
}
fn window(
    manifest: &HistoryManifest,
    index: usize,
) -> Result<(DateTime<FixedOffset>, Option<DateTime<FixedOffset>>)> {
    let from = time(&manifest.entries[index].recorded_at)?;
    let until = manifest
        .entries
        .get(index + 1)
        .map(|e| time(&e.recorded_at))
        .transpose()?;
    if let Some(until) = until {
        ensure!(
            until >= from,
            "NON_MONOTONIC_HISTORY: system clock moved backwards"
        );
    }
    Ok((from, until))
}
fn source(store: &Store, memory: &Memory) -> Result<Value> {
    let mut result = json!({"source_kind":memory.extra.get("source_kind").cloned().unwrap_or(json!("unknown")),"verification_state":memory.extra.get("verification_state").cloned().unwrap_or(json!("unknown")),"trusted_verification":false,"source_events":[],"coverage":"unknown"});
    if let (Some(count), Some(expected)) = (
        memory
            .extra
            .get("source_event_count")
            .and_then(Value::as_u64),
        memory
            .extra
            .get("source_event_ledger_hash")
            .and_then(Value::as_str),
    ) {
        safe_id(&memory.id)?;
        let dir = store.root.join("evidence");
        checked(&dir)?;
        let path = dir.join(format!("{}.json", memory.id));
        checked(&path)?;
        let bytes = crate::input::read_bytes(fs::File::open(path)?, 8 * 1024 * 1024)?;
        let ledger: Value = serde_json::from_slice(&bytes)?;
        ensure!(
            ledger["schema_version"] == 2 && ledger["memory"]["id"] == memory.id,
            "Invalid source ledger identity"
        );
        let events: Vec<SourceEvent> = serde_json::from_value(ledger["events"].clone())?;
        let prefix = events
            .get(..usize::try_from(count)?)
            .context("Historical source ledger is incomplete")?;
        ensure!(
            format!("{:x}", Sha256::digest(serde_json::to_vec(prefix)?)) == expected,
            "Historical source ledger hash mismatch"
        );
        result["source_events"] = serde_json::to_value(prefix)?;
        result["coverage"] = json!("versioned_prefix");
    }
    Ok(result)
}
fn item(store: &Store, id: &str, manifest: &HistoryManifest, index: usize) -> Result<Value> {
    let entry = &manifest.entries[index];
    let memory = revisions::history_image(store, id, entry)?;
    let (from, until) = window(manifest, index)?;
    Ok(
        json!({"memory_ref":{"store_id":read_manifest(store)?.map(|m|m.store_id),"memory_id":id},"revision":entry.revision,"system_from":from.with_timezone(&Utc).to_rfc3339(),"system_until":until.map(|t|t.with_timezone(&Utc).to_rfc3339()),"reason":entry.reason,"unknown_gap":entry.unknown_gap,"provenance":source(store,&memory)?,"applicability":crate::applicability::evaluate(store,&memory)?,"applicability_scope":"current_worktree","memory":memory}),
    )
}
fn locate(
    stores: &[Store],
    id: &str,
    store_id: Option<&str>,
    revision: Option<u64>,
) -> Result<Value> {
    safe_id(id)?;
    let mut found = None;
    for store in stores {
        if !ready(store)? {
            continue;
        }
        let _lock = lock_store_existing_read_only(store)?;
        require_settled(store)?;
        let identity = read_manifest(store)?.map(|m| m.store_id);
        if store_id.is_some_and(|wanted| identity.as_deref() != Some(wanted)) {
            continue;
        }
        let (_, manifest) = revisions::read_history(store, id)?;
        if let Some(manifest) = manifest {
            ensure!(found.is_none(), "AMBIGUOUS_MEMORY_ID");
            found = Some(if let Some(revision) = revision {
                let index = manifest
                    .entries
                    .iter()
                    .position(|e| e.revision == revision)
                    .context("Unknown revision")?;
                item(store, id, &manifest, index)?
            } else {
                let entries=manifest.entries.iter().enumerate().map(|(i,e)|{let (from,until)=window(&manifest,i)?;Ok(json!({"revision":e.revision,"system_from":from.to_rfc3339(),"system_until":until.map(|t|t.to_rfc3339()),"reason":e.reason,"unknown_gap":e.unknown_gap,"semantic_hash":e.semantic_hash}))}).collect::<Result<Vec<Value>>>()?;
                json!({"memory_ref":{"store_id":identity,"memory_id":id},"coverage_start":manifest.coverage_start,"coverage":"recorded_history","entries":entries})
            });
        } else if load_memories_unlocked(store, true)?
            .iter()
            .any(|(_, m)| m.id == id)
        {
            ensure!(found.is_none(), "AMBIGUOUS_MEMORY_ID");
            found = Some(
                json!({"memory_ref":{"store_id":identity,"memory_id":id},"coverage":"unknown","entries":[],"reason":"No recorded history; current Markdown is not a historical fallback"}),
            );
        }
    }
    Ok(found.unwrap_or_else(||json!({"coverage":"unknown","memory_id":id,"entries":[],"reason":"No recorded history; current Markdown is not a historical fallback"})))
}
pub fn list(stores: &[Store], id: &str, store_id: Option<&str>) -> Result<Value> {
    locate(stores, id, store_id, None)
}
pub fn show(stores: &[Store], id: &str, store_id: Option<&str>, revision: u64) -> Result<Value> {
    locate(stores, id, store_id, Some(revision))
}

#[allow(clippy::too_many_arguments)]
pub fn search(
    stores: &[Store],
    query: &str,
    as_of: &str,
    limit: usize,
    kind: &str,
    include_archive: bool,
    include_superseded: bool,
) -> Result<Value> {
    let at = time(as_of)?;
    let query_day = at.date_naive();
    let terms: BTreeSet<_> = crate::search::tokenize(query).into_iter().collect();
    let mut items = Vec::new();
    let mut unknown = Vec::new();
    for store in stores {
        if !ready(store)? {
            unknown.push(json!({"scope":store.scope,"reason":"No readable history coverage"}));
            continue;
        }
        let _lock = lock_store_existing_read_only(store)?;
        require_settled(store)?;
        let current: std::collections::HashMap<_, _> = load_memories_unlocked(store, true)?
            .into_iter()
            .map(|(_, m)| (m.id.clone(), revisions::semantic_digest(&m)))
            .collect();
        for id in ids(store)? {
            let (_, manifest) = revisions::read_history(store, &id)?;
            let Some(manifest) = manifest else {
                unknown.push(
                    json!({"scope":store.scope,"memory_id":id,"reason":"No recorded history"}),
                );
                continue;
            };
            let mut selected = None;
            for (i, _) in manifest.entries.iter().enumerate() {
                let (from, until) = window(&manifest, i)?;
                if from <= at && until.is_none_or(|u| at < u) {
                    selected = Some(i)
                }
            }
            let Some(index) = selected else {
                if manifest
                    .entries
                    .first()
                    .is_some_and(|e| e.reason != "created")
                {
                    unknown.push(json!({"scope":store.scope,"memory_id":id,"reason":"Before coverage_start"}));
                }
                continue;
            };
            // An observed external edit cannot reconstruct the preceding interval.
            if manifest
                .entries
                .get(index + 1)
                .is_some_and(|e| e.unknown_gap)
            {
                unknown.push(json!({"scope":store.scope,"memory_id":id,"reason":"Unobserved edits within this interval"}));
            }
            if index + 1 == manifest.entries.len()
                && current.get(&id) != Some(&manifest.entries[index].semantic_hash)
            {
                unknown.push(json!({"scope":store.scope,"memory_id":id,"reason":"Current content changed or vanished after the last observation"}));
            }
            let memory = revisions::history_image(store, &id, &manifest.entries[index])?;
            let expires = NaiveDate::parse_from_str(&memory.expires, "%Y-%m-%d").ok();
            let expired = expires.is_some_and(|d| d < query_day);
            if (!kind.is_empty() && memory.memory_type != kind)
                || (!include_superseded && memory.status == "superseded")
                || (!include_archive && (expired || memory.extra.contains_key("archived_at")))
            {
                continue;
            }
            let words: BTreeSet<_> = crate::search::tokenize(&format!(
                "{} {} {}",
                memory.body,
                memory.tags.join(" "),
                memory.canonical_summary
            ))
            .into_iter()
            .collect();
            let score = terms.intersection(&words).count();
            if !terms.is_empty() && score == 0 {
                continue;
            }
            let mut row = item(store, &id, &manifest, index)?;
            row["score"] = json!(score);
            row["expired"] = json!(expired);
            row["expiry_calendar_date"] = json!(query_day.to_string());
            items.push(row);
        }
    }
    items.sort_by(|a, b| {
        b["score"].as_u64().cmp(&a["score"].as_u64()).then_with(|| {
            a["memory_ref"]
                .to_string()
                .cmp(&b["memory_ref"].to_string())
        })
    });
    items.truncate(limit.min(10000));
    Ok(
        json!({"version":1,"mode":"historical_lexical","as_of":at.with_timezone(&Utc).to_rfc3339(),"input":as_of,"expiry_calendar_date":query_day.to_string(),"expiry_offset":at.offset().to_string(),"interval":"[system_from,system_until)","coverage":if unknown.is_empty(){"complete_for_known_records"}else{"partial_history"},"unknown":unknown,"items":items,"ranking":"snapshot lexical term overlap; not historical ranking replay","disabled_lanes":["current_vectors","current_graph","access_updates","current_view_fallback"]}),
    )
}
