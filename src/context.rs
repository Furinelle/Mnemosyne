//! Host-neutral, estimated-budget context assembly and session injection.
use crate::{
    api::search_entries,
    schema::{is_expired, parse_memory},
    store::*,
};
use anyhow::{Result, bail, ensure};
use chrono::{DateTime, Local, Utc};
use fs2::FileExt;
use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

pub fn approx_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let mut cost = 0.0_f64;
    for c in text.chars() {
        cost += if matches!(c, '\u{4e00}'..='\u{9fff}' | '\u{3040}'..='\u{30ff}' | '\u{ac00}'..='\u{d7a3}')
        {
            0.6
        } else {
            0.25
        };
    }
    (cost.round() as usize).max(1)
}

fn text<'a>(item: &'a Value, key: &str) -> &'a str {
    item[key].as_str().unwrap_or("")
}
fn inline(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn budget(config: &Value) -> Result<usize> {
    let value = &config["injection"]["max_tokens"];
    if value.is_null() {
        return Ok(2000);
    }
    value
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| anyhow::anyhow!("injection.max_tokens must be a nonnegative integer"))
}

#[derive(Clone, Debug, Serialize)]
pub struct ContextBundle {
    pub version: u8,
    pub context: String,
    pub items: Vec<Value>,
    pub estimated_tokens: usize,
    pub budget_mode: &'static str,
    pub selected: Vec<Value>,
    pub omitted: Vec<Value>,
}

struct Assembly {
    bundle: ContextBundle,
    delivered: Vec<Value>,
}

fn item_reason(item: &Value, reason: &str) -> Value {
    json!({"id":text(item,"id"),"scope":text(item,"scope"),"path":text(item,"path"),"kind":item["kind"].as_str().unwrap_or("memory"),"reason":reason,"match_reason":text(item,"match_reason")})
}

fn show_hint(config: &Value, channel: &str) -> String {
    let custom = config["injection"]["show_command_template"]
        .as_str()
        .unwrap_or("");
    if !custom.is_empty() {
        return custom.to_owned();
    }
    match channel {
        "none" => String::new(),
        "mcp" => "Call the mnemosyne_show tool with the id for full detail.".into(),
        _ => "Run `mnemosyne show <id>` for full detail.".into(),
    }
}

fn exposed(stores: &[Store], config: &Value, channel: &str) -> Vec<Store> {
    stores
        .iter()
        .filter(|s| {
            channel != "mcp"
                || config["mcp"][format!("expose_{}", s.scope)]
                    .as_bool()
                    .unwrap_or(true)
        })
        .cloned()
        .collect()
}

fn core(stores: &[Store]) -> Result<String> {
    let mut parts = Vec::new();
    for store in stores {
        let content = read_core(store)?;
        if !content.trim().is_empty() {
            let label = if store.scope == "global" {
                "Global Core"
            } else {
                "Project Core"
            };
            parts.push(format!("### {label}\n\n{}", content.trim()));
        }
    }
    Ok(parts.join("\n\n"))
}

fn identity(item: &Value) -> String {
    // Search supplies store-resolved paths. Equal ids in different stores must not collide.
    let path = Path::new(text(item, "path"));
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    json!([text(item, "scope"), path, text(item, "id")]).to_string()
}

fn semantic_identity(item: &Value) -> String {
    let semantic = if let Some(revision) = item.get("semantic_rev").and_then(Value::as_str) {
        json!([revision])
    } else {
        let path = Path::new(text(item, "path"));
        // The canonical file catches same-id corrections even when the injection summary stays equal.
        let memory = fs::read_to_string(path)
            .ok()
            .and_then(|contents| parse_memory(&contents).ok());
        if let Some(memory) = memory {
            json!([
                memory.memory_type,
                memory.source,
                memory.created,
                memory.tags,
                memory.links,
                memory.canonical_summary,
                memory.injection_summary,
                memory.status,
                memory.expires,
                memory.body,
                memory.extra
            ])
        } else {
            json!([
                item["type"],
                item["title"],
                item["source"],
                item["created"],
                item["tags"],
                item["links"],
                item["summary"],
                item["status"],
                item["expires"],
                item["evidence"],
                item["invalidated_by"],
                item["related_paths"],
                item["warnings"]
            ])
        }
    };
    // Worktree applicability can change while the stored memory stays identical.
    let digest = Sha256::digest(
        serde_json::to_vec(&json!([semantic, item["applicability"]])).unwrap_or_default(),
    );
    json!([identity(item), format!("{digest:x}")]).to_string()
}

#[cfg(test)]
fn assemble(
    base: &str,
    suffix: &str,
    results: &[Value],
    config: &Value,
    channel: &str,
    prefix: &str,
) -> Result<(String, Vec<Value>)> {
    let output = assemble_bundle(base, suffix, results, config, channel, prefix, None)?;
    Ok((output.bundle.context, output.delivered))
}

fn assemble_bundle(
    base: &str,
    suffix: &str,
    results: &[Value],
    config: &Value,
    channel: &str,
    prefix: &str,
    budget_override: Option<usize>,
) -> Result<Assembly> {
    let max_tokens = match budget_override {
        Some(value) => value,
        None => budget(config)?,
    };
    let join = |entries: &[String]| {
        let mut parts = Vec::new();
        if !base.is_empty() {
            parts.push(base.to_owned());
        }
        if !entries.is_empty() {
            if !prefix.is_empty() {
                parts.push(prefix.to_owned());
            }
            parts.push(format!(
                "## Relevant memories from Mnemosyne\n\n{}",
                entries.join("\n")
            ));
            let hint = show_hint(config, channel);
            if !hint.is_empty() {
                parts.push(hint);
            }
        }
        if !suffix.is_empty() {
            parts.push(suffix.to_owned());
        }
        parts.join("\n\n")
    };
    ensure!(
        approx_tokens(&join(&[])) <= max_tokens,
        "BUDGET_TOO_SMALL: mandatory context exceeds estimated budget"
    );
    let summary_chars = config["injection"]["summary_chars"].as_u64().unwrap_or(120) as usize;
    let mut candidates: Vec<_> = results.iter().collect();
    candidates.sort_by(|a, b| {
        b["score"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&a["score"].as_f64().unwrap_or(0.0))
            .then_with(|| {
                b["strength"]
                    .as_i64()
                    .unwrap_or(0)
                    .cmp(&a["strength"].as_i64().unwrap_or(0))
            })
    });
    let mut entries = Vec::new();
    let mut selected = Vec::new();
    let mut delivered = Vec::new();
    let mut omitted = Vec::new();
    let mut items = Vec::new();
    for item in candidates {
        let source = inline(text(item, "source"));
        let created = inline(text(item, "created"));
        let mut provenance = Vec::new();
        if !source.is_empty() {
            provenance.push(source);
        }
        if !created.is_empty() {
            provenance.push(format!("recorded {created}"));
        }
        let label = if provenance.is_empty() {
            String::new()
        } else {
            format!(" [{}]", provenance.join("; "))
        };
        let mut warnings = item["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(inline)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        if !text(item, "invalidated_by").is_empty() {
            warnings.push(format!(
                "invalidated by {}",
                inline(text(item, "invalidated_by"))
            ));
        }
        if item["applicability"]["applies_to"].is_string()
            && item["applicability"]["status"] != "applicable"
        {
            warnings.push(format!(
                "code applicability: {} ({})",
                item["applicability"]["status"]
                    .as_str()
                    .unwrap_or("unknown"),
                item["applicability"]["reason"]
                    .as_str()
                    .unwrap_or("unknown")
            ));
        }
        let warning = if warnings.is_empty() {
            String::new()
        } else {
            format!(" [Warning: {}]", warnings.join("; "))
        };
        let fixed = format!(
            "- ({}/{}) {}{}{label}{warning}",
            inline(text(item, "scope")),
            inline(if text(item, "kind") == "checkpoint" {
                "checkpoint"
            } else {
                text(item, "type")
            }),
            inline(text(item, "id")),
            if text(item, "title").is_empty() || text(item, "title") == text(item, "id") {
                String::new()
            } else {
                format!(" — {}", inline(text(item, "title")))
            }
        );
        let tags = item["tags"]
            .as_array()
            .map(|tags| {
                tags.iter()
                    .filter_map(Value::as_str)
                    .map(inline)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let summary = inline(text(item, "summary"));
        let summary =
            if text(item, "kind") != "checkpoint" && summary.chars().count() > summary_chars {
                format!(
                    "{}...",
                    summary
                        .chars()
                        .take(summary_chars.saturating_sub(3))
                        .collect::<String>()
                )
            } else {
                summary
            };
        let mut detail = format!(
            "{}: {summary}",
            if tags.is_empty() {
                String::new()
            } else {
                format!(" [{tags}]")
            }
        );
        let mut line = format!("{fixed}{detail}");
        entries.push(line.clone());
        if approx_tokens(&join(&entries)) > max_tokens {
            entries.pop();
            if !entries.is_empty() {
                omitted.push(item_reason(item, "budget"));
                continue;
            }
            loop {
                line = format!("{fixed}{}...", detail.trim_end());
                if approx_tokens(&join(&[line.clone()])) <= max_tokens {
                    break;
                }
                if detail.pop().is_none() {
                    break;
                }
            }
            if approx_tokens(&join(&[line.clone()])) > max_tokens {
                omitted.push(item_reason(item, "budget"));
                continue;
            }
            entries.push(line);
        }
        selected.push(item_reason(
            item,
            if text(item, "match_reason").is_empty() {
                "ranked_relevance"
            } else {
                text(item, "match_reason")
            },
        ));
        delivered.push(item.clone());
        items.push(json!({"kind":item["kind"].as_str().unwrap_or("memory"),"id":text(item,"id"),"scope":text(item,"scope"),"path":text(item,"path"),"text":entries.last().cloned().unwrap_or_default()}));
    }
    let context = join(&entries);
    let estimated_tokens = approx_tokens(&context);
    Ok(Assembly {
        bundle: ContextBundle {
            version: 1,
            context,
            items,
            estimated_tokens,
            budget_mode: "estimated",
            selected,
            omitted,
        },
        delivered,
    })
}

const FINDINGS: &str = "### Reporting new findings\n\nWhen you finish, if you discovered something worth persisting, append a block in this exact format at the END of your reply:\n\n**新发现:**\n- type: pitfall|arch_decision|codebase|handoff\n- importance: 50-90\n- title: <=80 chars\n- tags: tag1, tag2\n- content: |\n    <multiline content here, 4-space indent>\n\nMultiple findings: repeat the block. Skip if there is nothing to record.";

pub fn prep(stores: &[Store], task: &str, limit: usize, channel: &str) -> Result<String> {
    Ok(prep_bundle(stores, task, limit, channel, None, &[])?.context)
}

pub fn prep_bundle(
    stores: &[Store],
    task: &str,
    limit: usize,
    channel: &str,
    budget_override: Option<usize>,
    extra_items: &[Value],
) -> Result<ContextBundle> {
    let config = load_config(stores.last())?;
    let stores = exposed(stores, &config, channel);
    let core = core(&stores)?;
    let base = format!(
        "## Project memory (via Mnemosyne){}",
        if core.is_empty() {
            String::new()
        } else {
            format!("\n\n{core}")
        }
    );
    let tools = match channel {
        "cli" => {
            "### Mnemosyne CLI available\n\nIf you need more context mid-task:\n    mnemosyne search \"<keywords>\" --format json --limit 3\n\n"
        }
        "mcp" => {
            "### Mnemosyne memory tools available\n\nIf you need more context mid-task, call the mnemosyne_search tool with your keywords.\n\n"
        }
        _ => "",
    };
    let results = if task.trim().is_empty() || stores.is_empty() {
        vec![]
    } else {
        search_entries(&stores, task, limit, "", false, false, false)?
    };
    let mut all = extra_items.to_vec();
    for item in &mut all {
        if item.get("score").is_none() {
            item["score"] = json!(10.0);
        }
    }
    all.extend(results);
    Ok(assemble_bundle(
        &base,
        &format!("{tools}{FINDINGS}"),
        &all,
        &config,
        channel,
        "### Relevant prior memories",
        budget_override,
    )?
    .bundle)
}

fn result(bundle: ContextBundle, selected: &[Value]) -> Value {
    json!({"approx_tokens":bundle.estimated_tokens,"context":bundle.context,"memory_ids":selected.iter().filter(|v| text(v,"kind") != "checkpoint").map(|v| text(v,"id")).collect::<Vec<_>>(),"budget_mode":"estimated","context_bundle":bundle})
}

struct SessionLock(File);
impl Drop for SessionLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => ensure!(
            !meta.file_type().is_symlink(),
            "Session state cannot use a symlink: {}",
            path.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

fn session_lock(root: &Path) -> Result<SessionLock> {
    reject_symlink(root)?;
    fs::create_dir_all(root)?;
    let path = root.join(".session_injected.lock");
    reject_symlink(&path)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(SessionLock(file)),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(e) => return Err(e.into()),
        }
    }
}

fn load_sessions(path: &Path) -> Result<Map<String, Value>> {
    reject_symlink(path)?;
    let contents = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(e.into()),
    };
    let data: Value = serde_json::from_str(&contents).unwrap_or(Value::Null);
    let cutoff = Utc::now() - chrono::Duration::hours(48);
    Ok(data["sessions"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, v)| {
            v["ids"].is_array()
                && v["ts"]
                    .as_str()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .is_some_and(|stamp| stamp >= cutoff)
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect())
}

fn save_sessions(path: &Path, sessions: Map<String, Value>) -> Result<()> {
    reject_symlink(path)?;
    let tmp = path.with_file_name(format!(".session-{}.tmp", uuid::Uuid::new_v4()));
    let written = (|| -> Result<()> {
        let mut file = OpenOptions::new().create_new(true).write(true).open(&tmp)?;
        file.write_all(
            serde_json::to_string(&json!({"version":3,"sessions":sessions}))?.as_bytes(),
        )?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if written.is_err() {
        let _ = fs::remove_file(tmp);
    }
    written
}

#[cfg(test)]
fn select_for_session(
    stores: &[Store],
    candidates: &[Value],
    session: &str,
    host: &str,
    channel: &str,
    config: &Value,
    prefix: &str,
) -> Result<(String, Vec<Value>)> {
    let output = select_for_session_bundle(
        stores, candidates, session, host, channel, config, prefix, None, "",
    )?;
    Ok((output.bundle.context, output.delivered))
}

#[allow(clippy::too_many_arguments)]
fn select_for_session_bundle(
    stores: &[Store],
    candidates: &[Value],
    session: &str,
    host: &str,
    channel: &str,
    config: &Value,
    prefix: &str,
    budget_override: Option<usize>,
    context_epoch: &str,
) -> Result<Assembly> {
    if session.is_empty() || candidates.is_empty() || stores.is_empty() {
        return assemble_bundle("", "", candidates, config, channel, prefix, budget_override);
    }
    let root = &stores.last().unwrap().root;
    // No search/model work occurs while the state lock is held.
    let _lock = match session_lock(root) {
        Ok(lock) => lock,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::WouldBlock) =>
        {
            // Session dedup is best effort; a busy state lock must not suppress recall.
            return assemble_bundle("", "", candidates, config, channel, prefix, budget_override);
        }
        Err(error) => return Err(error),
    };
    let path = root.join(".session_injected.json");
    let mut sessions = load_sessions(&path)?;
    let key = json!([host, channel, session, context_epoch]).to_string();
    let mut seen: HashSet<String> = sessions
        .get(&key)
        .and_then(|s| s["ids"].as_array())
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let fresh: Vec<_> = candidates
        .iter()
        .filter(|v| !seen.contains(&semantic_identity(v)))
        .cloned()
        .collect();
    let mut output = assemble_bundle("", "", &fresh, config, channel, prefix, budget_override)?;
    for item in candidates
        .iter()
        .filter(|v| seen.contains(&semantic_identity(v)))
    {
        output
            .bundle
            .omitted
            .push(item_reason(item, "already_delivered"));
    }
    if !output.delivered.is_empty() {
        seen.extend(output.delivered.iter().map(semantic_identity));
        let mut ids: Vec<_> = seen.into_iter().collect();
        ids.sort();
        sessions.insert(key, json!({"ts":Utc::now().to_rfc3339(),"ids":ids}));
        save_sessions(&path, sessions)?;
    }
    Ok(output)
}

fn bump_selected(stores: &[Store], selected: &[Value], config: &Value) {
    for item in selected {
        if text(item, "kind") == "checkpoint" {
            continue;
        }
        let path = Path::new(text(item, "path"));
        let Some(store) = stores
            .iter()
            .find(|s| s.scope == text(item, "scope") && path.starts_with(&s.root))
        else {
            continue;
        };
        // Access accounting is best effort and cannot turn delivered context into an error.
        let update = || -> Result<()> {
            let _lock = try_lock_store(store)?;
            ensure!(
                !fs::symlink_metadata(path)?.file_type().is_symlink(),
                "Symlink memory"
            );
            let mut memory = parse_memory(&fs::read_to_string(path)?)?;
            ensure!(
                memory.id == text(item, "id")
                    && memory.status != "superseded"
                    && !is_expired(&memory.expires),
                "Memory changed after search"
            );
            memory.access_count = memory.access_count.saturating_add(1);
            memory.strength = memory
                .strength
                .saturating_add(config["thresholds"]["bonus_access"].as_i64().unwrap_or(5))
                .min(100);
            memory.last_accessed = Local::now().format("%Y-%m-%d").to_string();
            write_memory(path, &memory)
        };
        let _ = update();
    }
}

fn repo_relative(path: &str, store: &Store) -> Option<String> {
    let path = Path::new(path);
    let relative = if path.is_absolute() {
        path.strip_prefix(store.root.parent()?).ok()?
    } else {
        path
    };
    let mut parts = Vec::new();
    for part in relative.components() {
        match part {
            std::path::Component::Normal(value) => parts.push(value.to_str()?.to_owned()),
            std::path::Component::CurDir => (),
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn file_candidates(stores: &[Store], files: &[Value]) -> Result<(Vec<Value>, Vec<String>)> {
    let mut candidates = Vec::new();
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    let mut seen_files = HashSet::new();
    for file in files {
        let file = file
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("file_touch.files must contain strings"))?;
        let Some(name) = Path::new(file).file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        if name.is_empty() || !seen_files.insert(file.to_owned()) {
            continue;
        }
        names.push(
            stores
                .iter()
                .find(|s| s.scope == "project")
                .and_then(|s| repo_relative(file, s))
                .unwrap_or_else(|| file.to_owned()),
        );
        let mut path_matches = Vec::new();
        let mut directory_matches = Vec::new();
        for store in stores.iter().filter(|s| s.scope == "project") {
            let Some(touched) = repo_relative(file, store) else {
                continue;
            };
            for (path, memory) in load_memories(store, false)? {
                if memory.status == "superseded" || is_expired(&memory.expires) {
                    continue;
                }
                let Some(paths) = memory.extra.get("related_paths").and_then(Value::as_array)
                else {
                    continue;
                };
                let related = paths
                    .iter()
                    .filter_map(Value::as_str)
                    .filter_map(|s| repo_relative(s, store))
                    .collect::<Vec<_>>();
                let reason = if related.iter().any(|s| s == &touched) {
                    "path_exact"
                } else if related.iter().any(|s| {
                    Path::new(s)
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .is_some_and(|p| Some(p) == Path::new(&touched).parent())
                }) {
                    "path_directory"
                } else {
                    continue;
                };
                let applicability = crate::applicability::evaluate(store, &memory)?;
                let candidate = json!({"applicability":applicability,"id":memory.id,"scope":store.scope,"type":memory.memory_type,"title":memory.title(),"source":memory.source,"created":memory.created,"score":if reason == "path_exact" {2.0} else {1.5},"strength":memory.strength,"tags":memory.tags,"summary":memory.injection_summary,"path":path,"match_reason":reason,"related_paths":paths,"warnings":memory.extra.get("warnings").cloned().unwrap_or(json!([]))});
                if reason == "path_exact" {
                    path_matches.push(candidate);
                } else {
                    directory_matches.push(candidate);
                }
            }
        }
        if path_matches.is_empty() {
            path_matches = directory_matches;
        }
        if path_matches.is_empty() {
            for store in stores.iter().filter(|s| s.scope == "project") {
                let Some(relative) = repo_relative(file, store) else {
                    continue;
                };
                let Some(parent) = Path::new(&relative)
                    .parent()
                    .and_then(Path::to_str)
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                for mut item in search_entries(
                    std::slice::from_ref(store),
                    parent,
                    2,
                    "",
                    false,
                    false,
                    false,
                )? {
                    item["match_reason"] = json!("directory_fallback");
                    path_matches.push(item);
                }
            }
        }
        if path_matches.is_empty() {
            for mut item in search_entries(stores, name, 2, "", false, false, false)? {
                item["match_reason"] = json!("basename_fallback");
                path_matches.push(item);
            }
        }
        for item in path_matches {
            if seen.insert(identity(&item)) {
                candidates.push(item);
            }
        }
    }
    Ok((candidates, names))
}

pub fn inject(event: &str, payload: &Value, session: &str, channel: &str) -> Result<Value> {
    inject_with_options(event, payload, session, channel, None, "", &[])
}

pub fn inject_with_options(
    event: &str,
    payload: &Value,
    session: &str,
    channel: &str,
    budget_override: Option<usize>,
    context_epoch: &str,
    extra_items: &[Value],
) -> Result<Value> {
    ensure!(
        matches!(
            event,
            "session_start" | "turn_start" | "file_touch" | "session_end"
        ),
        "Unknown event: {event}"
    );
    ensure!(payload.is_object(), "Event payload must be an object");
    let stores = stores_for_scope("all")?;
    let config = load_config(stores.last())?;
    let stores = exposed(&stores, &config, channel);
    if event == "session_end" {
        let target = find_project_store().unwrap_or_else(global_store);
        ensure!(
            !config["distill"]["enabled"].as_bool().unwrap_or(false)
                || stores
                    .iter()
                    .any(|s| s.scope == target.scope && s.root == target.root),
            "Session distillation destination is not exposed"
        );
        return crate::ingest::session_end(payload);
    }
    if event == "session_start" {
        let core = core(&stores)?;
        let base = if core.is_empty() {
            core
        } else {
            format!("## Mnemosyne Memory\n\n{core}")
        };
        let bundle = assemble_bundle(
            &base,
            "",
            extra_items,
            &config,
            channel,
            "",
            budget_override,
        )?
        .bundle;
        return Ok(result(bundle, extra_items));
    }
    let mut candidates = Vec::new();
    let mut prefix = String::new();
    if event == "turn_start" {
        let prompt = text(payload, "prompt").trim();
        if prompt.chars().count() < 10 {
            let bundle =
                assemble_bundle("", "", extra_items, &config, channel, "", budget_override)?.bundle;
            return Ok(result(bundle, extra_items));
        }
        if !stores.is_empty() {
            candidates = search_entries(&stores, prompt, 3, "", false, false, false)?;
        }
    } else {
        let files = match payload.get("files") {
            None => &[][..],
            Some(Value::Array(files)) => files.as_slice(),
            _ => bail!("file_touch.files must be an array of paths"),
        };
        let (found, names) = file_candidates(&stores, files)?;
        candidates = found;
        prefix = format!(
            "## Memories relevant to {}",
            names
                .iter()
                .map(|v| inline(v))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let mut all = extra_items.to_vec();
    for item in &mut all {
        if item.get("score").is_none() {
            item["score"] = json!(10.0);
        }
    }
    all.extend(candidates);
    let output = select_for_session_bundle(
        &stores,
        &all,
        session,
        text(payload, "host"),
        channel,
        &config,
        &prefix,
        budget_override,
        context_epoch,
    )?;
    let update = payload
        .get("update_access")
        .and_then(Value::as_bool)
        .unwrap_or(event == "turn_start");
    if update {
        bump_selected(&stores, &output.delivered, &config);
    }
    Ok(result(output.bundle, &output.delivered))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(id: &str, path: &Path) -> Value {
        json!({"id":id,"path":path,"scope":"project","type":"pitfall","summary":"并发检索".repeat(80),"score":1,"strength":50,"tags":[]})
    }
    #[test]
    fn budget_covers_mandatory_content_and_only_selects_rendered_items() -> Result<()> {
        let config = json!({"injection":{"max_tokens":100,"summary_chars":1000}});
        let items = [
            item("one", Path::new("/one")),
            item("two", Path::new("/two")),
        ];
        let (context, selected) = assemble(
            "### Core\n\nAlways preserve data.",
            "Final instructions",
            &items,
            &config,
            "cli",
            "",
        )?;
        assert!(approx_tokens(&context) <= 100);
        assert_eq!(selected.len(), 1);
        assert!(context.contains("one") && !context.contains("two"));
        assert!(context.ends_with("Final instructions"));
        assert!(
            assemble(
                "required",
                "",
                &[],
                &json!({"injection":{"max_tokens":0}}),
                "cli",
                ""
            )
            .is_err()
        );
        Ok(())
    }
    #[test]
    fn session_dedup_is_locked_and_distinguishes_stores() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let stores = vec![Store {
            scope: "project".into(),
            root: temp.path().to_path_buf(),
        }];
        let config = json!({"injection":{"max_tokens":2000}});
        let candidates = vec![
            item("same", &temp.path().join("first/same.md")),
            item("same", &temp.path().join("second/same.md")),
        ];
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let stores = stores.clone();
                let config = config.clone();
                let candidates = candidates.clone();
                std::thread::spawn(move || {
                    select_for_session(&stores, &candidates, "session", "agent", "cli", &config, "")
                        .unwrap()
                        .1
                        .len()
                })
            })
            .collect();
        assert_eq!(
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .sum::<usize>(),
            2
        );
        assert_eq!(
            select_for_session(
                &stores,
                &candidates,
                "session",
                "other-agent",
                "cli",
                &config,
                ""
            )?
            .1
            .len(),
            2
        );
        let path = temp.path().join(".session_injected.json");
        fs::write(&path,json!({"sessions":{"old":{"ts":(Utc::now()-chrono::Duration::hours(49)).to_rfc3339(),"ids":["x"]}}}).to_string())?;
        assert!(load_sessions(&path)?.is_empty());
        Ok(())
    }
    #[test]
    fn busy_session_state_does_not_drop_recall() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let store = Store {
            scope: "project".into(),
            root: temp.path().to_path_buf(),
        };
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(store.root.join(".session_injected.lock"))?;
        file.lock_exclusive()?;
        let items = [item("one", &store.root.join("one.md"))];
        let (context, selected) = select_for_session(
            &[store],
            &items,
            "s",
            "codex",
            "cli",
            &json!({"injection":{"max_tokens":2000}}),
            "",
        )?;
        assert!(context.contains("one"));
        assert_eq!(selected.len(), 1);
        Ok(())
    }
    #[test]
    fn mcp_exposure_filters_explicit_stores() {
        let stores = [
            Store {
                scope: "global".into(),
                root: "/global".into(),
            },
            Store {
                scope: "project".into(),
                root: "/project".into(),
            },
        ];
        let config = json!({"mcp":{"expose_global":false,"expose_project":true}});
        assert_eq!(exposed(&stores, &config, "mcp").len(), 1);
        assert_eq!(exposed(&[], &config, "mcp").len(), 0);
        assert_eq!(exposed(&stores, &config, "cli").len(), 2);
    }
    #[test]
    fn only_rendered_memories_receive_access_credit() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let store = Store {
            scope: "project".into(),
            root: temp.path().join(".mnemosyne"),
        };
        ensure_store(&store)?;
        let mut candidates = Vec::new();
        let mut paths = Vec::new();
        for id in ["first", "second"] {
            let memory = crate::schema::Memory {
                id: id.into(),
                memory_type: "pitfall".into(),
                status: "active".into(),
                strength: 98,
                body: "test".into(),
                ..Default::default()
            };
            let path = working_path(&store, &memory)?;
            write_memory(&path, &memory)?;
            candidates.push(item(id, &path));
            paths.push(path);
        }
        let config = json!({"injection":{"max_tokens":80,"summary_chars":1000},"thresholds":{"bonus_access":5}});
        let (_, selected) = assemble("", "", &candidates, &config, "cli", "")?;
        assert_eq!(selected.len(), 1);
        bump_selected(std::slice::from_ref(&store), &selected, &config);
        let held_lock = lock_store(&store)?;
        let started = Instant::now();
        bump_selected(std::slice::from_ref(&store), &selected, &config);
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(held_lock);
        assert_eq!(
            parse_memory(&fs::read_to_string(&paths[0])?)?.access_count,
            1
        );
        assert_eq!(parse_memory(&fs::read_to_string(&paths[0])?)?.strength, 100);
        assert_eq!(
            parse_memory(&fs::read_to_string(&paths[1])?)?.access_count,
            0
        );
        Ok(())
    }

    #[test]
    fn revision_epoch_and_heat_have_correct_session_identity() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let store = Store {
            scope: "project".into(),
            root: temp.path().join(".mnemosyne"),
        };
        ensure_store(&store)?;
        let mut memory = crate::schema::Memory {
            id: "same".into(),
            memory_type: "pitfall".into(),
            status: "active".into(),
            body: "## Original\n\nBody".into(),
            injection_summary: "Same summary".into(),
            ..Default::default()
        };
        let path = working_path(&store, &memory)?;
        write_memory(&path, &memory)?;
        let candidate = item("same", &path);
        let config = json!({"injection":{"max_tokens":2000}});
        let select = |epoch: &str| {
            select_for_session_bundle(
                std::slice::from_ref(&store),
                std::slice::from_ref(&candidate),
                "s",
                "host",
                "cli",
                &config,
                "",
                None,
                epoch,
            )
            .map(|v| v.delivered.len())
        };
        assert_eq!(select("one")?, 1);
        assert_eq!(select("one")?, 0);
        memory.access_count = 3;
        memory.strength = 80;
        memory.last_accessed = "2026-09-20".into();
        write_memory(&path, &memory)?;
        assert_eq!(select("one")?, 0);
        memory.body.push_str(" corrected");
        write_memory(&path, &memory)?;
        assert_eq!(select("one")?, 1);
        assert_eq!(select("two")?, 1);
        assert_eq!(select("two")?, 0);
        assert_eq!(
            select_for_session_bundle(
                std::slice::from_ref(&store),
                std::slice::from_ref(&candidate),
                "s",
                "other",
                "cli",
                &config,
                "",
                None,
                "two"
            )?
            .delivered
            .len(),
            1
        );
        assert_eq!(
            select_for_session_bundle(
                std::slice::from_ref(&store),
                std::slice::from_ref(&candidate),
                "s",
                "host",
                "mcp",
                &config,
                "",
                None,
                "two"
            )?
            .delivered
            .len(),
            1
        );
        let other_store = Store {
            scope: "project".into(),
            root: temp.path().join("other/.mnemosyne"),
        };
        ensure_store(&other_store)?;
        let other_path = working_path(&other_store, &memory)?;
        write_memory(&other_path, &memory)?;
        assert_eq!(
            select_for_session_bundle(
                std::slice::from_ref(&other_store),
                &[item("same", &other_path)],
                "s",
                "host",
                "cli",
                &config,
                "",
                None,
                "two"
            )?
            .delivered
            .len(),
            1
        );
        Ok(())
    }

    #[test]
    fn file_touch_prefers_repo_paths_before_basename() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let store = Store {
            scope: "project".into(),
            root: temp.path().join(".mnemosyne"),
        };
        ensure_store(&store)?;
        for (id, related) in [
            ("backend", "backend/config.rs"),
            ("frontend", "frontend/config.rs"),
        ] {
            let mut memory = crate::schema::Memory {
                id: id.into(),
                memory_type: "codebase".into(),
                status: "active".into(),
                body: format!("## {id}\n\nconfig.rs"),
                injection_summary: id.into(),
                ..Default::default()
            };
            memory
                .extra
                .insert("related_paths".into(), json!([related]));
            let path = working_path(&store, &memory)?;
            write_memory(&path, &memory)?;
        }
        let (exact, _) =
            file_candidates(std::slice::from_ref(&store), &[json!("backend/config.rs")])?;
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0]["id"], "backend");
        assert_eq!(exact[0]["match_reason"], "path_exact");
        let (both, labels) = file_candidates(
            std::slice::from_ref(&store),
            &[json!("backend/config.rs"), json!("frontend/config.rs")],
        )?;
        assert_eq!(both.len(), 2);
        assert_eq!(labels, ["backend/config.rs", "frontend/config.rs"]);
        let (dir, _) =
            file_candidates(std::slice::from_ref(&store), &[json!("frontend/other.rs")])?;
        assert_eq!(dir[0]["id"], "frontend");
        assert_eq!(dir[0]["match_reason"], "path_directory");
        let (fallback, _) = file_candidates(std::slice::from_ref(&store), &[json!("config.rs")])?;
        assert!(!fallback.is_empty());
        assert_eq!(fallback[0]["match_reason"], "basename_fallback");
        Ok(())
    }
    #[test]
    fn applicability_changes_refresh_delivered_context() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let store = Store {
            scope: "project".into(),
            root: temp.path().join(".mnemosyne"),
        };
        ensure_store(&store)?;
        let path = store.working_dir().join("fact.md");
        let memory = crate::schema::Memory {
            id: "fact".into(),
            memory_type: "codebase".into(),
            body: "## Fact".into(),
            ..Default::default()
        };
        write_memory(&path, &memory)?;
        let mut candidate = item("fact", &path);
        candidate["applicability"] =
            json!({"applies_to":"commit","status":"applicable","reason":"exact_match"});
        let config = json!({"injection":{"max_tokens":2000}});
        let select = |candidate: &Value| {
            select_for_session_bundle(
                std::slice::from_ref(&store),
                std::slice::from_ref(candidate),
                "r08-session",
                "host",
                "cli",
                &config,
                "",
                None,
                "",
            )
        };
        assert_eq!(select(&candidate)?.delivered.len(), 1);
        assert!(select(&candidate)?.delivered.is_empty());
        candidate["applicability"]["status"] = json!("unknown");
        candidate["applicability"]["reason"] = json!("dirty_worktree");
        let refreshed = select(&candidate)?;
        assert_eq!(refreshed.delivered.len(), 1);
        assert!(
            refreshed
                .bundle
                .context
                .contains("code applicability: unknown")
        );
        assert!(refreshed.bundle.estimated_tokens <= 2000);
        Ok(())
    }
    #[test]
    fn file_paths_never_deliver_expired_or_superseded_memories() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let store = Store {
            scope: "project".into(),
            root: temp.path().join(".mnemosyne"),
        };
        ensure_store(&store)?;
        for (id, status, expires) in [
            ("current", "active", ""),
            ("replaced", "superseded", ""),
            ("expired", "active", "2000-01-01"),
        ] {
            let mut memory = crate::schema::Memory {
                id: id.into(),
                memory_type: "codebase".into(),
                body: format!("## {id}\n\nOnly for routing config"),
                status: status.into(),
                expires: expires.into(),
                ..Default::default()
            };
            memory
                .extra
                .insert("related_paths".into(), json!(["src/config.rs"]));
            write_memory(&store.working_dir().join(format!("{id}.md")), &memory)?;
        }
        for path in ["src/config.rs", "src/other.rs"] {
            let (items, _) = file_candidates(std::slice::from_ref(&store), &[json!(path)])?;
            assert_eq!(items.len(), 1);
            assert_eq!(items[0]["id"], "current");
        }
        Ok(())
    }
}
