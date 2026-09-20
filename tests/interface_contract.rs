//! R00: the same fixture crosses native CLI and MCP without ambient host state.
use serde_json::{Value, json};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
fn call(root: &Path, args: &[&str], input: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mnemosyne"))
        .env_clear()
        .env("HOME", root.join("home"))
        .env("MNEMOSYNE_HOME", root.join("global"))
        .env("PATH", "")
        .current_dir(root)
        .args(args)
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
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}
#[test]
fn r00_same_fixture_cli_mcp_fields_and_aliases() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir(root.join(".git")).unwrap();
    call(root, &["init", "--no-agent-files"], "");
    let fixture = r#"{"findings":[{"type":"codebase","importance":70,"title":"Contract","content":"quartz service uses port 8080","evidence":"fixture:1"}]}"#;
    assert_eq!(
        call(root, &["ingest", "--format", "json"], fixture),
        call(root, &["codex-ingest", "--format", "json"], fixture)
    );
    call(root, &["ingest", "--format", "json", "--commit"], fixture);
    let cli: Value =
        serde_json::from_str(&call(root, &["search", "quartz", "--format", "json"], "")).unwrap();
    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"mnemosyne_search","arguments":{"query":"quartz"}}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"mnemosyne_prep_context","arguments":{"task":"quartz"}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"mnemosyne_codex_prep","arguments":{"task":"quartz"}}}),
    ];
    let input = requests
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let output = call(root, &["mcp", "serve"], &input);
    let replies: Vec<Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies[0]["result"]["tools"].as_array().unwrap().len(), 8);
    let mut cli_fields = cli.clone();
    let mut mcp_fields = replies[1]["result"]["structuredContent"]["items"].clone();
    // CLI search credits access after ranking; MCP search is read-only and the
    // subsequent call ranks the newly increased strength. Canonical fields match.
    assert!(cli[0]["score"].as_f64().unwrap() < mcp_fields[0]["score"].as_f64().unwrap());
    for rows in [&mut cli_fields, &mut mcp_fields] {
        for row in rows.as_array_mut().unwrap() {
            row.as_object_mut().unwrap().remove("score");
            row.as_object_mut().unwrap().remove("score_breakdown");
        }
    }
    assert_eq!(cli_fields, mcp_fields);
    assert_ne!(replies[2]["result"]["isError"], true);
    assert_eq!(replies[2]["result"], replies[3]["result"]);
    assert_eq!(
        call(root, &["prep", "quartz"], ""),
        call(root, &["codex-prep", "quartz"], "")
    );
}
