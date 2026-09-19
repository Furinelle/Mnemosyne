use crate::{
    schema::{Memory, is_expired},
    search,
    store::*,
};
use anyhow::{Result, bail, ensure};
use chrono::Local;
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
fn update_markdown_index(store: &Store, added: Option<&Memory>) -> Result<()> {
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
fn duplicate_entry(
    store: &Store,
    request: &WriteRequest,
    body: &str,
) -> Result<Option<WriteResult>> {
    if !request.allow_duplicate {
        for (_, old) in load_memories_unlocked(store, false)? {
            if old.status != "superseded"
                && !is_expired(&old.expires)
                && old.memory_type == request.memory_type
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
        source: if request.source.is_empty() {
            "agent".into()
        } else {
            request.source.clone()
        },
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
        output.push(json!({"id":m.id,"scope":result.store.scope,"type":m.memory_type,"source":m.source,"created":m.created,"status":m.status,"expires":m.expires,"expired":is_expired(&m.expires),"evidence":m.extra.get("evidence").cloned().unwrap_or(json!("")),"invalidated_by":m.extra.get("invalidated_by").cloned().unwrap_or(json!("")),"score":(result.score*10000.0).round()/10000.0,"strength":m.strength,"tags":m.tags,"links":m.links,"summary":m.injection_summary,"path":result.path,"why_matched":result.why_matched,"score_breakdown":result.score_breakdown}));
    }
    Ok(output)
}

pub fn maintain(stores: &[Store], dry_run: bool) -> Result<Value> {
    let mut counts =
        json!({"processed":0,"decayed":0,"deprecated":0,"archived":0,"core_candidates":[]});
    for store in stores.iter().filter(|s| s.root.exists()) {
        let _guard = lock_store(store)?;
        let config = load_config(Some(store))?;
        let t = &config["thresholds"];
        for (path, mut m) in load_memories_unlocked(store, false)? {
            counts["processed"] = json!(counts["processed"].as_u64().unwrap() + 1);
            let expired = is_expired(&m.expires);
            if !expired {
                m.strength -= t["decay_per_run"].as_i64().unwrap_or(1);
            }
            if !expired && m.strength < t["deprecated_strength"].as_i64().unwrap_or(5) {
                m.status = "deprecated".into();
            }
            let action = if expired || m.strength < t["archive_strength"].as_i64().unwrap_or(30) {
                "archived"
            } else if m.strength >= t["core_strength"].as_i64().unwrap_or(80)
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
                write_memory(&path, &m)?;
                if action == "archived" {
                    let month = Local::now().format("%Y-%m").to_string();
                    let target = store.root.join("archive").join(month).join(
                        path.file_name()
                            .ok_or_else(|| anyhow::anyhow!("Invalid archive path"))?,
                    );
                    ensure!(!target.exists(), "Archive destination already exists");
                    std::fs::create_dir_all(target.parent().unwrap())?;
                    std::fs::rename(&path, target)?;
                }
            }
        }
        if !dry_run {
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
