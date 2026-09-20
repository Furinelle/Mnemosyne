use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
fn call(root: &Path, args: &[&str], input: Value) -> (bool, String) {
    let mut p = Command::new(env!("CARGO_BIN_EXE_mnemosyne"))
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
    p.stdin
        .take()
        .unwrap()
        .write_all(format!("{input}\n").as_bytes())
        .unwrap();
    let out = p.wait_with_output().unwrap();
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}
fn ok(root: &Path, args: &[&str], input: Value) -> Value {
    let (success, text) = call(root, args, input);
    assert!(success, "{args:?}: {text}");
    serde_json::from_str(&text).unwrap()
}
#[test]
fn cli_and_mcp_proposals_keep_human_approval_boundary() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir(root.join(".git")).unwrap();
    assert!(call(root, &["init", "--no-agent-files"], json!({})).0);
    ok(root, &["store-upgrade", "--commit"], json!({}));
    let w = ok(
        root,
        &["write-v2"],
        json!({"type":"codebase","content":"Quartz routing","title":"fixture","importance":70,"origin":"test","source_session_id":"s","source_event_id":"e","finding_key":"f","source_kind":"tool_output","verification_state":"unverified"}),
    );
    let id = w["memory_ref"]["memory_id"].as_str().unwrap();
    let v = ok(root, &["show-v2", id], json!({}));
    let request = json!({"decision":"REFINE","reason":"test correction","evidence":[],"targets":[{"memory_ref":w["memory_ref"],"expected_rev":v["revision"]["semantic_rev"],"expected_hash":v["revision"]["semantic_hash"],"body":"Reviewed correction"}]});
    let rpc = |operation: &str, request: Value| json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mnemosyne_write","arguments":{"version":2,"operation":operation,"request":request}}});
    let p = ok(root, &["mcp", "serve"], rpc("proposal", request));
    assert_ne!(p["result"]["isError"], true, "{p}");
    let p = &p["result"]["structuredContent"];
    assert_eq!(p["state"], "pending");
    let denied = ok(
        root,
        &["mcp", "serve"],
        rpc(
            "approve",
            json!({"id":p["id"],"confirm_hash":p["summary_hash"]}),
        ),
    );
    assert!(denied.get("error").is_some() || denied["result"]["isError"] == true);
    let applied = ok(
        root,
        &[
            "proposal",
            "approve",
            "--id",
            p["id"].as_str().unwrap(),
            "--confirm-hash",
            p["summary_hash"].as_str().unwrap(),
        ],
        json!({}),
    );
    assert_eq!(applied["state"], "applied");
    let batch = ok(root, &["sleep", "export"], json!({}));
    assert!(batch["inputs"].as_array().unwrap().len() == 1);
    let out = ok(
        root,
        &["sleep", "import"],
        json!({"batch":batch,"proposals":[]}),
    );
    assert_eq!(out["facts_modified"], false);
}

#[test]
fn mcp_semantic_relations_and_body_rewrites_are_pending() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir(root.join(".git")).unwrap();
    assert!(call(root, &["init", "--no-agent-files"], json!({})).0);
    ok(root, &["store-upgrade", "--commit"], json!({}));
    let mut refs = vec![];
    for event in ["old", "new"] {
        refs.push(ok(root,&["write-v2"],json!({"type":"codebase","content":event,"title":event,"importance":70,"origin":"test","source_session_id":"s","source_event_id":event,"finding_key":"f","source_kind":"tool_output","verification_state":"unverified"}))["memory_ref"].clone());
    }
    let rpc = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mnemosyne_link","arguments":{"id1":refs[1]["memory_id"],"id2":refs[0]["memory_id"],"rel":"supersedes"}}});
    let reply = ok(root, &["mcp", "serve"], rpc);
    assert_eq!(
        reply["result"]["structuredContent"]["state"], "pending",
        "{reply}"
    );
    let v = ok(
        root,
        &["show-v2", refs[0]["memory_id"].as_str().unwrap()],
        json!({}),
    );
    assert_eq!(v["memory"]["status"], "active");
    let rpc = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mnemosyne_write","arguments":{"version":2,"operation":"revise","request":{"memory_ref":refs[0],"expected_rev":v["revision"]["semantic_rev"],"expected_hash":v["revision"]["semantic_hash"],"changes":{"body":"rewritten by host"}}}}});
    let reply = ok(root, &["mcp", "serve"], rpc);
    assert_eq!(
        reply["result"]["structuredContent"]["state"], "pending",
        "{reply}"
    );
    let after = ok(
        root,
        &["show-v2", refs[0]["memory_id"].as_str().unwrap()],
        json!({}),
    );
    assert_eq!(after["memory"]["body"], v["memory"]["body"]);
}
