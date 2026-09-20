use crate::{
    schema::{Memory, is_expired},
    search,
    store::*,
};
use anyhow::{Result, bail, ensure};
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct WriteRequest {
    #[serde(rename = "type")]
    pub memory_type: String,
    pub importance: i64,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub source: String,
    pub expires: String,
    pub evidence: String,
    pub allow_duplicate: bool,
}

#[derive(Debug, Serialize)]
pub struct WriteResult {
    pub status: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<String>,
}

// MEMORY.md is a derived directory for file-only clients, not the knowledge source.
// Caller holds the store lock, like the canonical write that precedes this update.
pub(crate) fn update_markdown_index(store: &Store, added: Option<&Memory>) -> Result<()> {
    use std::io::Write;
    let path = crate::store::cache_path(store, "MEMORY.md")?;
    let header = include_str!("../assets/templates/MEMORY.md");
    let line = |m: &Memory| {
        format!(
            "\n- `{}` ({}, strength {}): {}\n",
            m.id, m.memory_type, m.strength, m.injection_summary
        )
    };
    let text = if let Some(memory) = added {
        let mut text = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => header.to_owned(),
            Err(e) => return Err(e.into()),
        };
        text.push_str(&line(memory));
        text
    } else {
        let mut memories = load_memories_unlocked(store, false)?;
        memories.sort_by_key(|entry| std::cmp::Reverse(entry.1.strength));
        let mut text = header.to_owned();
        for (_, memory) in memories {
            text.push_str(&line(&memory));
        }
        text
    };
    let mut temporary = tempfile::NamedTempFile::new_in(&store.root)?;
    if let Ok(metadata) = std::fs::metadata(&path) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())?;
    }
    temporary.write_all(text.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

fn request_body(request: &WriteRequest) -> (String, String) {
    let title = if request.title.trim().is_empty() {
        request
            .content
            .lines()
            .find(|s| !s.trim().is_empty())
            .unwrap_or("")
            .trim()
            .trim_start_matches('#')
            .trim()
            .chars()
            .take(80)
            .collect::<String>()
    } else {
        request.title.trim().to_owned()
    };
    let body = format!(
        "## {title}\n\n{}",
        request.content.trim().replace("\r\n", "\n")
    );
    (title, body)
}
fn effective_source(source: &str) -> String {
    let source = source.trim();
    if source.is_empty() {
        "agent".into()
    } else {
        source.to_lowercase()
    }
}

fn duplicate_entry(
    store: &Store,
    request: &WriteRequest,
    body: &str,
) -> Result<Option<WriteResult>> {
    if !request.allow_duplicate {
        let source = effective_source(&request.source);
        for (_, old) in load_memories_unlocked(store, false)? {
            if old.status != "superseded"
                && !is_expired(&old.expires)
                && old.memory_type == request.memory_type
                && effective_source(&old.source) == source
                && old.body == body
                && old.expires == request.expires
                && old.tags == request.tags
                && old
                    .extra
                    .get("evidence")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    == request
                        .evidence
                        .trim()
                        .chars()
                        .take(200)
                        .collect::<String>()
            {
                return Ok(Some(WriteResult {
                    status: "duplicate".into(),
                    id: old.id.clone(),
                    duplicate_of: Some(old.id),
                    path: None,
                }));
            }
        }
    }
    Ok(None)
}
pub fn classify_entry(store: &Store, request: &WriteRequest) -> Result<WriteResult> {
    ensure!(!request.content.trim().is_empty(), "No content provided.");
    let _guard = if store.root.exists() {
        Some(lock_store(store)?)
    } else {
        None
    };
    let (_, body) = request_body(request);
    Ok(
        duplicate_entry(store, request, &body)?.unwrap_or(WriteResult {
            status: "new".into(),
            id: String::new(),
            path: None,
            duplicate_of: None,
        }),
    )
}

pub fn write_entry(store: &Store, request: &WriteRequest) -> Result<WriteResult> {
    ensure!(!request.content.trim().is_empty(), "No content provided.");
    ensure!(
        !request.memory_type.is_empty()
            && request
                .memory_type
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        "Invalid memory type"
    );
    ensure_store(store)?;
    let _guard = lock_store(store)?;
    crate::provenance::ensure_v1_writer_compatible(store)?;
    let (title, body) = request_body(request);
    if let Some(duplicate) = duplicate_entry(store, request, &body)? {
        return Ok(duplicate);
    }
    let today = Local::now().format("%Y-%m-%d").to_string();
    let summary = format!(
        "{title}: {}",
        request
            .content
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    )
    .chars()
    .take(220)
    .collect::<String>();
    let mut memory = Memory {
        id: format!(
            "{}-{today}-{}",
            request.memory_type,
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ),
        memory_type: request.memory_type.clone(),
        source: effective_source(&request.source),
        strength: request.importance.clamp(0, 100),
        created: today.clone(),
        last_accessed: today,
        tags: request.tags.clone(),
        canonical_summary: summary.clone(),
        injection_summary: summary,
        status: "active".into(),
        body,
        expires: request.expires.clone(),
        ..Default::default()
    };
    if !request.evidence.trim().is_empty() {
        memory.extra.insert(
            "evidence".into(),
            json!(
                request
                    .evidence
                    .trim()
                    .chars()
                    .take(200)
                    .collect::<String>()
            ),
        );
    }
    let path = working_path(store, &memory)?;
    write_memory(&path, &memory)?;
    if let Err(error) = update_markdown_index(store, Some(&memory)) {
        eprintln!("mnemosyne: memory saved, but derived MEMORY.md update failed: {error}");
    }
    Ok(WriteResult {
        status: "created".into(),
        id: memory.id,
        path: Some(path.display().to_string()),
        duplicate_of: None,
    })
}

pub fn search_entries(
    stores: &[Store],
    query: &str,
    limit: usize,
    kind: &str,
    archive: bool,
    superseded: bool,
    update_access: bool,
) -> Result<Vec<Value>> {
    let config = load_config(stores.last())?;
    let results = search::search(stores, query, limit, kind, archive, superseded, &config)?;
    let mut output = Vec::new();
    for result in results {
        let mut m = result.memory;
        if update_access {
            let bonus = config["thresholds"][if result
                .path
                .starts_with(result.store.root.join("archive"))
            {
                "bonus_recall"
            } else {
                "bonus_access"
            }]
            .as_i64()
            .unwrap_or(5);
            if let Ok(_guard) = try_lock_store(&result.store)
                && let Ok(text) = std::fs::read_to_string(&result.path)
                && let Ok(mut current) = crate::schema::parse_memory(&text)
            {
                if current.id != m.id
                    || current.body != m.body
                    || current.status != m.status
                    || current.expires != m.expires
                {
                    continue;
                }
                current.strength = (current.strength + bonus).min(100);
                current.access_count += 1;
                current.last_accessed = Local::now().format("%Y-%m-%d").to_string();
                if write_memory(&result.path, &current).is_ok() {
                    m = current;
                }
            }
        }
        let applicability = crate::applicability::evaluate(&result.store, &m)?;
        output.push(json!({"applicability":applicability,"id":m.id,"scope":result.store.scope,"type":m.memory_type,"title":m.title(),"source":m.source,"created":m.created,"status":m.status,"expires":m.expires,"expired":is_expired(&m.expires),"evidence":m.extra.get("evidence").cloned().unwrap_or(json!("")),"invalidated_by":m.extra.get("invalidated_by").cloned().unwrap_or(json!("")),"related_paths":m.extra.get("related_paths").cloned().unwrap_or(json!([])),"warnings":m.extra.get("warnings").cloned().unwrap_or(json!([])),"score":(result.score*10000.0).round()/10000.0,"strength":m.strength,"tags":m.tags,"links":m.links,"summary":m.injection_summary,"path":result.path,"why_matched":result.why_matched,"score_breakdown":result.score_breakdown}));
    }
    Ok(output)
}

pub fn maintain(stores: &[Store], dry_run: bool) -> Result<Value> {
    maintain_at(stores, dry_run, &crate::provenance::SystemClock)
}

fn maintenance_day(clock: &impl crate::provenance::Clock) -> NaiveDate {
    clock.now().date_naive()
}

fn expired_on(memory: &Memory, day: NaiveDate) -> bool {
    NaiveDate::parse_from_str(memory.expires.trim(), "%Y-%m-%d").is_ok_and(|expiry| expiry < day)
}

fn decay_mode(thresholds: &Value) -> Result<bool> {
    match thresholds["decay_mode"].as_str().unwrap_or("per_run") {
        "per_run" => Ok(false),
        "per_day" => Ok(true),
        mode => bail!("thresholds.decay_mode must be per_run or per_day, got {mode}"),
    }
}

/// `per_day` records its first observed maintenance day without charging old records.
/// The caller supplies the clock so day boundaries and clock rollback stay testable.
pub fn maintain_at(
    stores: &[Store],
    dry_run: bool,
    clock: &impl crate::provenance::Clock,
) -> Result<Value> {
    let mut counts =
        json!({"processed":0,"decayed":0,"deprecated":0,"archived":0,"core_candidates":[]});
    for store in stores.iter().filter(|s| s.root.exists()) {
        let _guard = if dry_run {
            let path = store.root.join(".lock");
            if path.exists() {
                Some(crate::store::lock_store_existing_read_only(store)?)
            } else {
                None
            }
        } else {
            Some(lock_store(store)?)
        };
        let config = load_config(Some(store))?;
        let t = &config["thresholds"];
        let per_day = decay_mode(t)?;
        let decay = t["decay_per_run"].as_i64().unwrap_or(1).max(0);
        let day = maintenance_day(clock);
        let day_text = day.to_string();
        let mut updated = false;
        let manifest = crate::provenance::read_manifest(store)?;
        if !dry_run && let Some(manifest) = &manifest {
            ensure!(
                manifest.min_writer_version == crate::provenance::WRITER_VERSION,
                "UPGRADE_REQUIRED: maintenance requires store-upgrade --commit"
            );
        }
        let versioned = manifest.is_some();
        for (path, mut m) in load_memories_unlocked(store, false)? {
            let expected = if versioned && !dry_run {
                let snapshot = crate::revisions::snapshot(store, &path)?;
                ensure!(
                    snapshot.semantic_hash == crate::revisions::semantic_digest(&m),
                    "REVISION_CONFLICT: maintenance input changed"
                );
                Some(snapshot)
            } else {
                None
            };
            counts["processed"] = json!(counts["processed"].as_u64().unwrap() + 1);
            let expired = expired_on(&m, day);
            let mut changed = false;
            if !expired && per_day {
                let last = m
                    .extra
                    .get("last_maintained_at")
                    .and_then(Value::as_str)
                    .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok());
                if let Some(last) = last {
                    let elapsed = (day - last).num_days().max(0);
                    if elapsed > 0 {
                        m.strength = m.strength.saturating_sub(decay.saturating_mul(elapsed));
                        m.extra.insert("last_maintained_at".into(), json!(day_text));
                        changed = true;
                    }
                } else {
                    m.extra.insert("last_maintained_at".into(), json!(day_text));
                    changed = true;
                }
            } else if !expired {
                m.strength = m.strength.saturating_sub(decay);
                changed = true;
            }
            if !per_day
                && !expired
                && m.status != "superseded"
                && m.strength < t["deprecated_strength"].as_i64().unwrap_or(5)
            {
                m.status = "deprecated".into();
                changed = true;
            }
            let pinned = m.extra.get("pinned").and_then(Value::as_bool) == Some(true);
            let action = if expired
                || (m.status == "superseded"
                    && m.strength < t["archive_strength"].as_i64().unwrap_or(30))
                || (!pinned && m.strength < t["archive_strength"].as_i64().unwrap_or(30))
            {
                "archived"
            } else if m.status == "active"
                && m.strength >= t["core_strength"].as_i64().unwrap_or(80)
                && m.access_count >= t["core_access_count"].as_i64().unwrap_or(3)
            {
                "core_candidate"
            } else if m.status == "deprecated" {
                "deprecated"
            } else {
                "decayed"
            };
            if action == "core_candidate" {
                counts["core_candidates"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"id":m.id,"summary":m.injection_summary}));
            } else {
                counts[action] = json!(counts[action].as_u64().unwrap() + 1);
            }
            if !dry_run {
                let archive_target = if action == "archived" {
                    let month = day.format("%Y-%m").to_string();
                    Some(
                        store.root.join("archive").join(month).join(
                            path.file_name()
                                .ok_or_else(|| anyhow::anyhow!("Invalid archive path"))?,
                        ),
                    )
                } else {
                    None
                };
                if versioned && (changed || archive_target.is_some()) {
                    if archive_target.is_some() {
                        m.extra
                            .insert("archived_at".into(), json!(clock.now().to_rfc3339()));
                    }
                    let mut plan = crate::revisions::plan_updates(
                        store,
                        &[crate::revisions::RevisionUpdate {
                            path: path.clone(),
                            memory: m,
                            expected,
                        }],
                        "maintenance",
                        clock,
                    )?;
                    if let Some(target) = archive_target {
                        let relative = path.strip_prefix(&store.root)?.to_path_buf();
                        let (before, after) = if let Some(index) = plan
                            .changes
                            .iter()
                            .position(|change| change.path == relative)
                        {
                            let change = plan.changes.remove(index);
                            (change.before, change.after)
                        } else {
                            let text = std::fs::read_to_string(&path)?;
                            (Some(text.clone()), Some(text))
                        };
                        plan.changes.push(crate::relations::MutationChange {
                            path: target.strip_prefix(&store.root)?.to_path_buf(),
                            before: None,
                            after,
                        });
                        plan.changes.push(crate::relations::MutationChange {
                            path: relative,
                            before,
                            after: None,
                        });
                    }
                    if !plan.changes.is_empty() {
                        crate::relations::execute_mutation(store, plan)?;
                        updated = true;
                    }
                } else if !versioned && (changed || archive_target.is_some()) {
                    write_memory(&path, &m)?;
                    if let Some(target) = archive_target {
                        ensure!(!target.exists(), "Archive destination already exists");
                        std::fs::create_dir_all(target.parent().unwrap())?;
                        std::fs::rename(&path, target)?;
                    }
                    updated = true;
                }
            }
        }
        if !dry_run && updated {
            update_markdown_index(store, None)?;
        }
    }
    Ok(counts)
}

pub fn scope_store(scope: &str) -> Result<Store> {
    match scope {
        "global" => Ok(global_store()),
        "project" => Ok(project_store()),
        _ => bail!("Write scope must be global or project"),
    }
}

/// Conservative consolidation retains old records and provenance. Approximate
/// matches are review candidates, never permission to discard a different fact.
pub fn consolidate(stores: &[Store], threshold: f64, commit: bool) -> Result<Value> {
    use std::collections::HashSet;
    ensure!(
        threshold.is_finite() && (0.0..=1.0).contains(&threshold),
        "Threshold must be between 0 and 1"
    );
    let mut candidates = Vec::new();
    for store in stores {
        let mut memories = load_memories(store, false)?;
        memories.sort_by(|a, b| b.1.strength.cmp(&a.1.strength).then(a.1.id.cmp(&b.1.id)));
        let mut retired = HashSet::new();
        for i in 0..memories.len() {
            let a = &memories[i].1;
            if a.status == "superseded" || is_expired(&a.expires) || retired.contains(&a.id) {
                continue;
            }
            let first: HashSet<_> = search::tokenize(&a.body).into_iter().collect();
            for (_, b) in &memories[i + 1..] {
                if b.memory_type != a.memory_type
                    || b.status == "superseded"
                    || is_expired(&b.expires)
                    || retired.contains(&b.id)
                {
                    continue;
                }
                let second: HashSet<_> = search::tokenize(&b.body).into_iter().collect();
                let union = first.union(&second).count();
                let score = if union == 0 {
                    0.0
                } else {
                    first.intersection(&second).count() as f64 / union as f64
                };
                if score < threshold {
                    continue;
                }
                let exact = a.body == b.body
                    && a.tags == b.tags
                    && a.expires == b.expires
                    && a.source == b.source
                    && a.extra == b.extra;
                if commit && exact {
                    // Recheck under the mutation's lock using expected hashes.
                    crate::relations::link_entries_checked(
                        stores,
                        &a.id,
                        &b.id,
                        "supersedes",
                        false,
                        Some((a, b)),
                    )?;
                    retired.insert(b.id.clone());
                }
                candidates.push(json!({"keep":a.id,"candidate":b.id,"similarity":score,"exact":exact,"applied":commit&&exact,"action":if exact{"supersede duplicate; retain history"}else{"review required; unchanged"}}));
            }
        }
    }
    Ok(json!({"candidates":candidates,"commit":commit}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complementary_facts_and_changed_values_survive() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let store = Store {
            scope: "project".into(),
            root: tmp.path().join(".mnemosyne"),
        };
        let mut req = WriteRequest {
            memory_type: "codebase".into(),
            title: "Service config".into(),
            content: "port 8080".into(),
            importance: 70,
            ..Default::default()
        };
        assert_eq!(write_entry(&store, &req)?.status, "created");
        assert_eq!(write_entry(&store, &req)?.status, "duplicate");
        req.content = "healthcheck /health".into();
        assert_eq!(write_entry(&store, &req)?.status, "created");
        req.content = "port 9090".into();
        assert_eq!(write_entry(&store, &req)?.status, "created");
        req.expires = "2099-01-01".into();
        assert_eq!(write_entry(&store, &req)?.status, "created");
        assert!(
            load_memories(&store, false)?
                .iter()
                .all(|(_, m)| m.status == "active")
        );
        let index = std::fs::read_to_string(store.root.join("MEMORY.md"))?;
        for (_, memory) in load_memories(&store, false)? {
            assert!(index.contains(&memory.id));
        }
        Ok(())
    }
}

/// Corrections preserve identity and use semantic CAS; heat updates are not conflicts.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviseRequest {
    pub memory_ref: crate::provenance::MemoryRef,
    pub expected_rev: u64,
    pub expected_hash: String,
    pub changes: RevisionFields,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RevisionFields {
    pub body: Option<String>,
    #[serde(rename = "type")]
    pub memory_type: Option<String>,
    pub tags: Option<Vec<String>>,
    pub source: Option<String>,
    pub evidence: Option<String>,
    pub expires: Option<String>,
    pub observed_commit: Option<String>,
    pub branch: Option<String>,
    pub applies_to: Option<String>,
    pub applies_commit: Option<String>,
    pub applies_tree: Option<String>,
    pub related_paths: Option<Vec<String>>,
    pub related_path_hashes: Option<Vec<String>>,
}

pub fn revise_v2(
    store: &Store,
    request: &ReviseRequest,
    clock: &impl crate::provenance::Clock,
) -> Result<Value> {
    let _lock = lock_store(store)?;
    let manifest = crate::provenance::read_manifest(store)?
        .ok_or_else(|| anyhow::anyhow!("Explicit upgrade required"))?;
    ensure!(
        manifest.min_writer_version == crate::provenance::WRITER_VERSION,
        "UPGRADE_REQUIRED: corrections require store-upgrade --commit"
    );
    ensure!(
        manifest.store_id == request.memory_ref.store_id,
        "STORE_ID_MISMATCH"
    );
    let (path, mut memory) = load_memories_unlocked(store, true)?
        .into_iter()
        .find(|(_, m)| m.id == request.memory_ref.memory_id)
        .ok_or_else(|| anyhow::anyhow!("Memory not found"))?;
    let snapshot = crate::revisions::snapshot(store, &path)?;
    ensure!(
        snapshot.semantic_rev == request.expected_rev
            && snapshot.semantic_hash == request.expected_hash,
        "REVISION_CONFLICT: memory changed"
    );
    let changes = &request.changes;
    if let Some(body) = &changes.body {
        ensure!(!body.trim().is_empty(), "Empty body");
        memory.body = body.clone();
        let summary: String = body
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(220)
            .collect();
        memory.canonical_summary = summary.clone();
        memory.injection_summary = summary;
    }
    if let Some(kind) = &changes.memory_type {
        let config = load_config(Some(store))?;
        ensure!(
            config["memory"]["types"]
                .as_array()
                .is_some_and(|types| types.iter().any(|v| v.as_str() == Some(kind))),
            "Invalid memory type"
        );
        memory.memory_type = kind.clone();
    }
    if let Some(tags) = &changes.tags {
        memory.tags = tags.clone();
    }
    if let Some(source) = &changes.source {
        memory.source = effective_source(source);
    }
    if let Some(evidence) = &changes.evidence {
        ensure!(
            evidence.chars().count() <= 200,
            "Evidence exceeds 200 characters"
        );
        memory.extra.insert("evidence".into(), json!(evidence));
    }
    if let Some(expires) = &changes.expires {
        memory.expires = expires.clone();
    }
    for (key, value) in [
        ("observed_commit", &changes.observed_commit),
        ("branch", &changes.branch),
        ("applies_to", &changes.applies_to),
        ("applies_commit", &changes.applies_commit),
        ("applies_tree", &changes.applies_tree),
    ] {
        if let Some(value) = value {
            memory.extra.insert(key.into(), json!(value));
        }
    }
    for (key, value) in [
        ("related_paths", &changes.related_paths),
        ("related_path_hashes", &changes.related_path_hashes),
    ] {
        if let Some(value) = value {
            memory.extra.insert(key.into(), json!(value));
        }
    }
    crate::revisions::write_locked(store, &path, &memory, Some(snapshot), "correction", clock)?;
    update_markdown_index(store, None)?;
    Ok(
        json!({"version":2,"memory_ref":request.memory_ref,"revision":crate::revisions::snapshot(store, &path)?}),
    )
}

pub fn show_v2(stores: &[Store], id: &str, store_id: Option<&str>) -> Result<Value> {
    let (store, path, _) = crate::provenance::resolve_id_v2(stores, id, store_id)?
        .ok_or_else(|| anyhow::anyhow!("Memory not found"))?;
    let _lock = lock_store(&store)?;
    let provenance = crate::provenance::read_provenance_unlocked(&store, id)?;
    let memory = crate::schema::parse_memory(&std::fs::read_to_string(&path)?)?;
    let manifest = crate::provenance::read_manifest(&store)?
        .ok_or_else(|| anyhow::anyhow!("Missing store manifest"))?;
    let applicability = crate::applicability::evaluate(&store, &memory)?;
    Ok(
        json!({"version":2,"applicability":applicability,"memory_ref":{"store_id":manifest.store_id,"memory_id":id},"memory":memory,"provenance":provenance,"revision":crate::revisions::snapshot(&store,&path)?}),
    )
}
