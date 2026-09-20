use serde_json::{Value, json};
use std::{
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

fn run(root: &Path, args: &[&str], input: &str) -> Output {
    let mut child = command(root, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}
fn command(root: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_mnemosyne"));
    c.env_clear()
        .current_dir(root)
        .env("HOME", root.join("home"))
        .env("MNEMOSYNE_HOME", root.join("global"))
        .args(args);
    c
}
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn setup() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join(".git")).unwrap();
    success(run(t.path(), &["init", "--no-agent-files"], ""));
    t
}
#[test]
fn native_cli_round_trip_chinese_and_safety() {
    let t = setup();
    let root = t.path();
    let first = success(run(
        root,
        &[
            "write",
            "--type",
            "codebase",
            "--importance",
            "70",
            "--title",
            "Service config",
            "--content",
            "认证服务 port 8080",
            "--force",
        ],
        "",
    ));
    let id = first.trim().strip_prefix("Wrote ").unwrap();
    success(run(
        root,
        &[
            "write",
            "--type",
            "codebase",
            "--importance",
            "70",
            "--title",
            "Service config",
            "--content",
            "认证服务 healthcheck /health",
            "--force",
        ],
        "",
    ));
    let result: Value = serde_json::from_str(&success(run(
        root,
        &["search", "认证服务", "--format", "json"],
        "",
    )))
    .unwrap();
    assert_eq!(result.as_array().unwrap().len(), 2);
    assert!(
        result
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["status"] == "active")
    );
    assert!(success(run(root, &["show", id], "")).contains("port 8080"));
    let failed = run(
        root,
        &["inject", "--event", "turn_start", "--fail-safe"],
        "not json",
    );
    assert!(failed.status.success());
    assert!(failed.stdout.is_empty());
    assert!(!failed.stderr.is_empty());
    let bad = run(
        root,
        &[
            "write",
            "--type",
            "../../outside",
            "--importance",
            "70",
            "--content",
            "test",
        ],
        "",
    );
    assert!(!bad.status.success());
}

#[test]
fn concurrent_replays_write_once() {
    let t = setup();
    let args = [
        "write",
        "--type",
        "codebase",
        "--importance",
        "70",
        "--title",
        "Concurrent",
        "--content",
        "same fact",
        "--force",
    ];
    let children: Vec<_> = (0..8)
        .map(|_| {
            command(t.path(), &args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    for child in children {
        success(child.wait_with_output().unwrap());
    }
    let count = std::fs::read_dir(t.path().join(".mnemosyne/working"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|s| s == "md"))
        .count();
    assert_eq!(count, 1);
}

#[test]
fn mcp_respects_exposure_and_has_no_stdout_noise() {
    let t = setup();
    let config = t.path().join(".mnemosyne/config.toml");
    std::fs::write(
        config,
        "[mcp]\nexpose_global = false\nexpose_project = true\n",
    )
    .unwrap();
    let input=[json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),json!({"jsonrpc":"2.0","method":"notifications/initialized"}),json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"mnemosyne_read_core","arguments":{"scope":"global"}}}),json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"mnemosyne_search","arguments":{"query":"missing"}}})].iter().map(Value::to_string).collect::<Vec<_>>().join("\n");
    let output = success(run(t.path(), &["mcp", "serve"], &input));
    let replies: Vec<Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 3);
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "mnemosyne");
    assert_eq!(replies[1]["result"]["isError"], true);
    assert_eq!(
        replies[2]["result"]["structuredContent"]["items"],
        json!([])
    );
}

#[test]
fn legacy_markdown_and_aliases_work_without_python() {
    let t = setup();
    std::fs::write(t.path().join(".mnemosyne/working/legacy.md"),"---\nid: legacy\ntype: codebase\nstrength: 70\ntags: [\"a,b\", 中文]\ncustom_field: \"path: C:\\\\work\"\nstatus: active\n---\n## Legacy\n\n旧库兼容验证\n").unwrap();
    let shown = success(run(t.path(), &["show", "legacy"], ""));
    assert!(shown.contains("custom_field"));
    assert!(shown.contains("a,b"));
    assert_eq!(
        success(run(t.path(), &["prep", "旧库兼容验证"], "")),
        success(run(t.path(), &["codex-prep", "旧库兼容验证"], ""))
    );
    let finding = r#"{"findings":[{"type":"pitfall","importance":70,"title":"safe replay","content":"same","tags":[]}]}"#;
    success(run(
        t.path(),
        &["codex-ingest", "--format", "json", "--commit"],
        finding,
    ));
    let replay = success(run(
        t.path(),
        &["ingest", "--format", "json", "--commit"],
        finding,
    ));
    assert!(replay.contains("duplicate"));
}

#[test]
fn consolidation_retains_different_values_and_old_evidence() {
    let t = setup();
    for content in ["server port 8080", "server port 8080", "server port 9090"] {
        success(run(
            t.path(),
            &[
                "write",
                "--type",
                "codebase",
                "--importance",
                "70",
                "--title",
                "Service",
                "--content",
                content,
                "--allow-duplicate",
                "--force",
            ],
            "",
        ));
    }
    let output: Value = serde_json::from_str(&success(run(
        t.path(),
        &["consolidate", "--threshold", "0.5", "--commit"],
        "",
    )))
    .unwrap();
    assert_eq!(
        output["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["applied"] == true)
            .count(),
        1
    );
    let results: Value = serde_json::from_str(&success(run(
        t.path(),
        &["search", "server", "--format", "json"],
        "",
    )))
    .unwrap();
    assert_eq!(results.as_array().unwrap().len(), 2);
    let all: Value = serde_json::from_str(&success(run(
        t.path(),
        &[
            "search",
            "server",
            "--format",
            "json",
            "--include-superseded",
        ],
        "",
    )))
    .unwrap();
    assert_eq!(all.as_array().unwrap().len(), 3);
}

#[test]
fn installation_preview_does_not_modify_host_and_config_hides_endpoints() {
    let t = setup();
    let home = t.path().join("host");
    for agent in ["codex", "claude-code", "grok", "antigravity"] {
        let output = success(run(t.path(), &["install", agent, "--dry-run"], ""));
        assert!(output.contains("configuration suggestion"));
    }
    assert!(!home.exists());
    let output = success(run(t.path(), &["config", "--format", "json"], ""));
    assert!(!output.contains("api_base"));
    assert!(!output.contains("api_key_env"));
}

#[test]
#[cfg(unix)]
fn sqlite_cache_symlink_cannot_modify_another_file() {
    let t = setup();
    let outside = t.path().join("outside.db");
    std::fs::write(&outside, "must remain untouched").unwrap();
    std::os::unix::fs::symlink(&outside, t.path().join(".mnemosyne/rust-index.sqlite")).unwrap();
    let result = run(t.path(), &["reindex", "--scope", "project"], "");
    assert!(!result.status.success());
    assert_eq!(
        std::fs::read_to_string(outside).unwrap(),
        "must remain untouched"
    );
}

#[test]
fn mcp_project_path_works_without_host_project_cwd() {
    let t = tempfile::tempdir().unwrap();
    let project = t.path().join("project");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    success(run(&project, &["init", "--no-agent-files"], ""));
    let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mnemosyne_write","arguments":{"type":"codebase","importance":70,"content":"explicit project only","project_path":project}}});
    let reply: Value = serde_json::from_str(&success(run(
        t.path(),
        &["mcp", "serve"],
        &request.to_string(),
    )))
    .unwrap();
    assert_eq!(reply["result"]["structuredContent"]["status"], "created");
    assert!(!t.path().join(".mnemosyne").exists());
    let id = reply["result"]["structuredContent"]["id"].as_str().unwrap();
    assert!(success(run(&project, &["show", id], "")).contains("explicit project only"));
    let request = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"mnemosyne_write","arguments":{"type":"codebase","importance":70,"content":"no implicit root project"}}});
    let reply: Value = serde_json::from_str(&success(run(
        t.path(),
        &["mcp", "serve"],
        &request.to_string(),
    )))
    .unwrap();
    assert_eq!(reply["result"]["isError"], true);
    assert!(!t.path().join(".mnemosyne").exists());
}
