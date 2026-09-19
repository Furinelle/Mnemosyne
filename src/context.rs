//! Host-neutral, estimated-budget context assembly and session injection.
use crate::{
    api::search_entries,
    schema::{is_expired, parse_memory},
    store::*,
};
use anyhow::{Result, bail, ensure};
use chrono::{DateTime, Local, Utc};
use fs2::FileExt;
use serde_json::{Map, Value, json};
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

fn assemble(
    base: &str,
    suffix: &str,
    results: &[Value],
    config: &Value,
    channel: &str,
    prefix: &str,
) -> Result<(String, Vec<Value>)> {
    let max_tokens = budget(config)?;
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
        let fixed = format!(
            "- ({}/{}) {}{label}",
            inline(text(item, "scope")),
            inline(text(item, "type")),
            inline(text(item, "id"))
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
        let summary = if summary.chars().count() > summary_chars {
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
                continue;
            }
            entries.push(line);
        }
        selected.push(item.clone());
    }
    Ok((join(&entries), selected))
}

const FINDINGS: &str = "### Reporting new findings\n\nWhen you finish, if you discovered something worth persisting, append a block in this exact format at the END of your reply:\n\n**新发现:**\n- type: pitfall|arch_decision|codebase|handoff\n- importance: 50-90\n- title: <=80 chars\n- tags: tag1, tag2\n- content: |\n    <multiline content here, 4-space indent>\n\nMultiple findings: repeat the block. Skip if there is nothing to record.";

pub fn prep(stores: &[Store], task: &str, limit: usize, channel: &str) -> Result<String> {
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
    Ok(assemble(
        &base,
        &format!("{tools}{FINDINGS}"),
        &results,
        &config,
        channel,
        "### Relevant prior memories",
    )?
    .0)
}

fn result(context: String, selected: &[Value]) -> Value {
    json!({"approx_tokens":approx_tokens(&context),"context":context,"memory_ids":selected.iter().map(|v| text(v,"id")).collect::<Vec<_>>(),"budget_mode":"estimated"})
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
            serde_json::to_string(&json!({"version":2,"sessions":sessions}))?.as_bytes(),
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

fn select_for_session(
    stores: &[Store],
    candidates: &[Value],
    session: &str,
    host: &str,
    channel: &str,
    config: &Value,
    prefix: &str,
) -> Result<(String, Vec<Value>)> {
    if session.is_empty() || candidates.is_empty() || stores.is_empty() {
        return assemble("", "", candidates, config, channel, prefix);
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
            return assemble("", "", candidates, config, channel, prefix);
        }
        Err(error) => return Err(error),
    };
    let path = root.join(".session_injected.json");
    let mut sessions = load_sessions(&path)?;
    let key = json!([host, channel, session]).to_string();
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
        .filter(|v| !seen.contains(&identity(v)))
        .cloned()
        .collect();
    let output = assemble("", "", &fresh, config, channel, prefix)?;
    if !output.1.is_empty() {
        seen.extend(output.1.iter().map(identity));
        let mut ids: Vec<_> = seen.into_iter().collect();
        ids.sort();
        sessions.insert(key, json!({"ts":Utc::now().to_rfc3339(),"ids":ids}));
        save_sessions(&path, sessions)?;
    }
    Ok(output)
}

fn bump_selected(stores: &[Store], selected: &[Value], config: &Value) {
    for item in selected {
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

pub fn inject(event: &str, payload: &Value, session: &str, channel: &str) -> Result<Value> {
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
        return Ok(result(
            assemble(&base, "", &[], &config, channel, "")?.0,
            &[],
        ));
    }
    let mut candidates = Vec::new();
    let mut prefix = String::new();
    if event == "turn_start" {
        let prompt = text(payload, "prompt").trim();
        if prompt.chars().count() < 10 {
            return Ok(result(String::new(), &[]));
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
        let mut names = Vec::new();
        let mut seen = HashSet::new();
        for file in files {
            let file = file
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("file_touch.files must contain strings"))?;
            let Some(name) = Path::new(file).file_name().and_then(|v| v.to_str()) else {
                continue;
            };
            if name.is_empty() || names.iter().any(|v| v == name) {
                continue;
            }
            names.push(name.to_owned());
            if !stores.is_empty() {
                for item in search_entries(&stores, name, 2, "", false, false, false)? {
                    if seen.insert(identity(&item)) {
                        candidates.push(item);
                    }
                }
            }
        }
        prefix = format!(
            "## Memories relevant to {}",
            names
                .iter()
                .map(|v| inline(v))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let (context, selected) = select_for_session(
        &stores,
        &candidates,
        session,
        text(payload, "host"),
        channel,
        &config,
        &prefix,
    )?;
    let update = payload
        .get("update_access")
        .and_then(Value::as_bool)
        .unwrap_or(event == "turn_start");
    if update {
        bump_selected(&stores, &selected, &config);
    }
    Ok(result(context, &selected))
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
}
