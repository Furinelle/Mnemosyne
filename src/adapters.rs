//! Native host adapters. Installation prints suggestions; it never edits host settings.
use crate::{context, ingest, store::*};
use anyhow::{Result, bail, ensure};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use fs2::FileExt;
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Command, Stdio},
};

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => ensure!(
            !meta.file_type().is_symlink(),
            "Symlink is not allowed: {}",
            path.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

fn auto_init_at(cwd: &Path, global: &Path) -> Result<()> {
    let mut current = cwd.canonicalize()?;
    loop {
        if current.join(".git").exists() {
            let target = current.join(".mnemosyne");
            if current.join(".mnemosyne-disable").exists() || target.exists() {
                return Ok(());
            }
            let global = global
                .canonicalize()
                .unwrap_or_else(|_| global.to_path_buf());
            if target != global {
                ensure_store(&Store {
                    scope: "project".into(),
                    root: target,
                })?;
            }
            return Ok(());
        }
        if !current.pop() {
            return Ok(());
        }
    }
}

fn maybe_auto_init() {
    let setting = std::env::var("MNEMOSYNE_AUTO_INIT")
        .unwrap_or_else(|_| "1".into())
        .to_lowercase();
    if ["0", "false", "no"].contains(&setting.as_str()) {
        return;
    }
    if let Ok(cwd) = std::env::current_dir()
        && let Err(error) = auto_init_at(&cwd, &global_store().root)
    {
        eprintln!("mnemosyne: auto-init skipped: {error}");
    }
}

fn maintenance_due(path: &Path) -> Result<bool> {
    reject_symlink(path)?;
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(e.into()),
    };
    let stamp = DateTime::parse_from_rfc3339(text.trim())
        .map(|s| s.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%dT%H:%M:%S%.f")
                .ok()
                .and_then(|s| Local.from_local_datetime(&s).single())
                .map(|s| s.with_timezone(&Utc))
        });
    Ok(stamp.is_none_or(|s| Utc::now().signed_duration_since(s) >= chrono::Duration::hours(24)))
}

fn claim_maintenance(root: &Path, spawn: impl FnOnce() -> Result<()>) -> Result<bool> {
    if !root.exists() {
        return Ok(false);
    }
    reject_symlink(root)?;
    let lock_path = root.join(".last_maintain.lock");
    reject_symlink(&lock_path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path)?;
    match lock.try_lock_exclusive() {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
        Err(e) => return Err(e.into()),
    }
    let marker = root.join(".last_maintain");
    if !maintenance_due(&marker)? {
        return Ok(false);
    }
    spawn()?;
    let tmp = root.join(format!(".last_maintain.{}.tmp", uuid::Uuid::new_v4()));
    let written = (|| -> Result<()> {
        let mut file = OpenOptions::new().create_new(true).write(true).open(&tmp)?;
        file.write_all(Utc::now().to_rfc3339().as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, &marker)?;
        Ok(())
    })();
    if written.is_err() {
        let _ = fs::remove_file(tmp);
    }
    written?;
    Ok(true)
}

fn maybe_maintain() {
    let Ok(stores) = stores_for_scope("all") else {
        return;
    };
    for store in stores {
        let scheduled = claim_maintenance(&store.root, || {
            let mut command = Command::new(std::env::current_exe()?);
            command
                .args(["maintain", "--scope", &store.scope])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                command.process_group(0);
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x00000008 | 0x00000200);
            }
            let mut child = command.spawn()?;
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Ok(())
        });
        if let Err(error) = scheduled {
            eprintln!("mnemosyne: maintenance scheduling skipped: {error}");
        }
    }
}

fn hook_output(event: &str, output: &Value) -> Option<Value> {
    let context = output["context"].as_str().unwrap_or("");
    if context.is_empty() {
        None
    } else if event == "Stop" {
        Some(json!({"systemMessage":context}))
    } else {
        Some(json!({"hookSpecificOutput":{"hookEventName":event,"additionalContext":context}}))
    }
}

fn stop_payload(event: &Value) -> Result<Option<Value>> {
    if event["stop_hook_active"].as_bool().unwrap_or(false) {
        return Ok(None);
    }
    let Some(path) = event["transcript_path"].as_str().filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    // JSONL preambles can exceed 100 records; find the first actual message
    // without reading the whole transcript into memory a second time.
    let mut format = "auto";
    let mut saw_json = false;
    if let Ok(file) = File::open(path) {
        for line in BufReader::new(file).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if !serde_json::from_str::<Value>(&line).is_ok_and(|v| v.is_object()) {
                if saw_json {
                    return Ok(None);
                }
                format = "text";
                break;
            }
            saw_json = true;
            let found = ingest::detect_format(&line);
            if found != "text" {
                format = found;
                break;
            }
        }
    }
    if saw_json && format == "auto" {
        return Ok(None);
    }
    let source = match format {
        "codex-jsonl" => "codex",
        "grok-jsonl" => "grok-build",
        _ => "claude-code",
    };
    Ok(Some(
        json!({"transcript":{"path":path,"format":format},"source":source}),
    ))
}

pub fn hook(event: &str, payload: &Value) -> Result<Option<Value>> {
    ensure!(payload.is_object(), "Hook payload must be an object");
    let session = payload["session_id"].as_str().unwrap_or("");
    match event {
        "SessionStart" => {
            if matches!(payload["source"].as_str(), Some("resume" | "compact")) {
                return Ok(None);
            }
            maybe_auto_init();
            let output = context::inject("session_start", &json!({}), session, "cli")?;
            let result = hook_output(event, &output);
            if result.is_some() {
                maybe_maintain();
            }
            Ok(result)
        }
        "UserPromptSubmit" => {
            let output = context::inject(
                "turn_start",
                &json!({"prompt":payload["prompt"].as_str().unwrap_or(""),"host":"claude-code"}),
                session,
                "cli",
            )?;
            Ok(hook_output(event, &output))
        }
        "PreToolUse" => {
            let Some(path) = payload["tool_input"]["file_path"]
                .as_str()
                .filter(|s| !s.is_empty())
            else {
                return Ok(None);
            };
            let config = load_config(None)?;
            let tool = payload["tool_name"].as_str().unwrap_or("");
            let allowed = config["hooks"]["write_tools"]
                .as_array()
                .is_some_and(|tools| tools.iter().any(|name| name.as_str() == Some(tool)));
            if !allowed {
                return Ok(None);
            }
            let output = context::inject(
                "file_touch",
                &json!({"files":[path],"host":"claude-code"}),
                session,
                "cli",
            )?;
            // No permissionDecision: context injection must never bypass tool approval.
            Ok(hook_output(event, &output))
        }
        "Stop" => {
            let Some(payload) = stop_payload(payload)? else {
                return Ok(None);
            };
            Ok(hook_output(event, &ingest::session_end(&payload)?))
        }
        _ => bail!("Unknown host hook: {event}"),
    }
}

pub fn install(agent: &str, dry_run: bool) -> Result<String> {
    let intro = if dry_run {
        "Dry run: configuration suggestion only; no host settings were changed."
    } else {
        "Configuration suggestion only; no host settings were changed. Apply the following configuration explicitly in the host."
    };
    match agent {
        "claude-code" | "claude" | "codex" | "grok" => {
            let mut hooks = serde_json::Map::new();
            for event in ["SessionStart", "UserPromptSubmit", "PreToolUse", "Stop"] {
                if agent == "codex" && event == "PreToolUse" {
                    continue;
                }
                let mut entry = json!({"hooks":[{"type":"command","command":format!("mnemosyne hook {event}")}]});
                if event == "PreToolUse" {
                    entry["matcher"] = json!("Edit|Write");
                }
                hooks.insert(event.into(), json!([entry]));
            }
            Ok(format!(
                "{intro}\n\nMerge these hooks into the host hook settings (Codex: ~/.codex/hooks.json; Claude/Grok: ~/.claude/settings.json); preserve unrelated hooks. Ensure the native mnemosyne executable is on PATH.\n\n{}",
                serde_json::to_string_pretty(&json!({"hooks":hooks}))?
            ))
        }
        "antigravity" => Ok(format!(
            "{intro}\n\nMerge this MCP definition into Antigravity mcp_config.json. Supply project_path in tool calls for project memory.\n{}",
            serde_json::to_string_pretty(
                &json!({"mcpServers":{"mnemosyne":{"command":std::env::current_exe()?,"args":["mcp","serve"]}}})
            )?
        )),
        _ => bail!("Unsupported agent: {agent}. Supported: codex, claude-code, grok, antigravity"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_keeps_permissions_untouched_and_stop_is_not_reentrant() -> Result<()> {
        let output = hook_output("PreToolUse", &json!({"context":"Relevant memory"})).unwrap();
        assert!(
            output["hookSpecificOutput"]
                .get("permissionDecision")
                .is_none()
        );
        assert_eq!(output["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(
            hook_output("Stop", &json!({"context":"Saved"})).unwrap(),
            json!({"systemMessage":"Saved"})
        );
        assert!(
            stop_payload(&json!({"stop_hook_active":true,"transcript_path":"/not/read"}))?
                .is_none()
        );
        assert!(hook("SessionStart", &json!({"source":"compact"}))?.is_none());
        assert!(hook("not-an-event", &json!({})).is_err());
        Ok(())
    }
    #[test]
    fn auto_init_stays_in_nearest_repository_and_honors_opt_out() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let repo = tmp.path().join("project");
        fs::create_dir_all(repo.join(".git"))?;
        fs::create_dir_all(repo.join("deep/nested"))?;
        let global = tmp.path().join("global");
        fs::write(repo.join(".mnemosyne-disable"), "")?;
        auto_init_at(&repo.join("deep/nested"), &global)?;
        assert!(!repo.join(".mnemosyne").exists());
        fs::remove_file(repo.join(".mnemosyne-disable"))?;
        auto_init_at(&repo.join("deep/nested"), &global)?;
        assert!(repo.join(".mnemosyne/core.md").exists());
        assert!(!global.exists());
        Ok(())
    }
    #[test]
    fn maintenance_claim_is_per_store_and_once_per_day() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let root = tmp.path().to_path_buf();
                let count = count.clone();
                std::thread::spawn(move || {
                    claim_maintenance(&root, || {
                        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok(())
                    })
                    .unwrap()
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
        fs::write(
            tmp.path().join(".last_maintain"),
            (Utc::now() - chrono::Duration::hours(25)).to_rfc3339(),
        )?;
        assert!(claim_maintenance(tmp.path(), || Ok(()))?);
        Ok(())
    }
    #[test]
    fn stop_detects_source_and_install_only_describes_configuration() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let path = tmp.path().join("session.jsonl");
        fs::write(
            &path,
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[]}})
                .to_string(),
        )?;
        assert_eq!(
            stop_payload(&json!({"transcript_path":path}))?.unwrap()["source"],
            "codex"
        );
        let preamble = tmp.path().join("long-preamble.jsonl");
        let mut rows = (0..101)
            .map(|i| json!({"type":"session_meta","payload":{"n":i}}).to_string())
            .collect::<Vec<_>>();
        rows.push(
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[]}})
                .to_string(),
        );
        fs::write(&preamble, rows.join("\n"))?;
        let detected = stop_payload(&json!({"transcript_path":preamble}))?.unwrap();
        assert_eq!(detected["source"], "codex");
        assert_eq!(detected["transcript"]["format"], "codex-jsonl");
        fs::write(
            &preamble,
            json!({"type":"session_meta","payload":{}}).to_string(),
        )?;
        assert!(stop_payload(&json!({"transcript_path":preamble}))?.is_none());
        let suggestion = install("claude-code", true)?;
        assert!(suggestion.contains("mnemosyne hook PreToolUse"));
        assert!(!suggestion.contains("permissionDecision"));
        assert!(install("antigravity", false)?.contains("project_path"));
        Ok(())
    }
}
