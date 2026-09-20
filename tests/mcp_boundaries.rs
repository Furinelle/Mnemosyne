use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

fn run(root: &Path, input: &[u8], limit: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mnemosyne"));
    command
        .env_clear()
        .current_dir(root)
        .args(["mcp", "serve"])
        .env("HOME", root.join("home"))
        .env("MNEMOSYNE_HOME", root.join("global"))
        .env("PATH", root.join("empty-bin"));
    if let Some(limit) = limit {
        command.env("MNEMOSYNE_MAX_INPUT_BYTES", limit);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _ = child.stdin.take().unwrap().write_all(input);
    child.wait_with_output().unwrap()
}
fn replies(root: &Path, requests: &[Value]) -> Vec<Value> {
    let input = requests
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let output = run(root, input.as_bytes(), None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}
fn sandbox() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    for folder in [
        "home",
        "global",
        "empty-bin",
        "project/.git",
        "project/.mnemosyne/working",
    ] {
        fs::create_dir_all(temp.path().join(folder)).unwrap();
    }
    fs::write(
        temp.path().join("project/.mnemosyne/config.toml"),
        "[mcp]\nexpose_global = false\n",
    )
    .unwrap();
    temp
}
fn call(id: Value, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})
}
fn init(id: i64, version: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"initialize","params":{"protocolVersion":version,"clientInfo":{"name":"isolated-fixture","version":"1"},"capabilities":{}}})
}
#[test]
fn notifications_never_reply_or_execute_mutating_tools() {
    let t = sandbox();
    let mut mutation = call(
        json!(1),
        "mnemosyne_write",
        json!({"project_path":t.path().join("project"),"type":"codebase","importance":70,"content":"must not write"}),
    );
    mutation.as_object_mut().unwrap().remove("id");
    let result = replies(
        t.path(),
        &[
            json!({"jsonrpc":"2.0","method":"ping"}),
            json!({"jsonrpc":"2.0","method":"notifications/future","params":{"anything":true}}),
            mutation,
            json!({"jsonrpc":"2.0","id":9,"method":"ping"}),
        ],
    );
    assert_eq!(
        result.len(),
        1,
        "notifications must not get fabricated id:null responses"
    );
    assert_eq!(result[0]["id"], 9);
    assert_eq!(
        fs::read_dir(t.path().join("project/.mnemosyne/working"))
            .unwrap()
            .count(),
        0
    );
}
#[test]
fn malformed_envelopes_cannot_write() {
    let t = sandbox();
    let valid = call(
        json!(1),
        "mnemosyne_write",
        json!({"project_path":t.path().join("project"),"type":"codebase","importance":70,"content":"malformed write"}),
    );
    let mut requests = Vec::new();
    for id in [Value::Null, json!(true), json!(1.25), json!([]), json!({})] {
        let mut value = valid.clone();
        value["id"] = id;
        requests.push(value);
    }
    for version in [Value::Null, json!("1.0"), json!(2)] {
        let mut value = valid.clone();
        value["jsonrpc"] = version;
        requests.push(value);
    }
    let mut no_version = valid.clone();
    no_version.as_object_mut().unwrap().remove("jsonrpc");
    requests.push(no_version);
    let mut unknown = valid;
    unknown["unexpected"] = json!(true);
    requests.push(unknown);
    let result = replies(t.path(), &requests);
    assert_eq!(result.len(), requests.len());
    assert!(
        result.iter().all(|v| v["error"]["code"] == -32600),
        "{result:?}"
    );
    assert_eq!(
        fs::read_dir(t.path().join("project/.mnemosyne/working"))
            .unwrap()
            .count(),
        0
    );
}
#[test]
fn protocol_errors_are_distinct_from_tool_execution_errors() {
    let t = sandbox();
    let project = t.path().join("project");
    let result = replies(
        t.path(),
        &[
            call(
                json!(1),
                "mnemosyne_show",
                json!({"project_path":project,"id":"missing"}),
            ),
            call(
                json!(2),
                "mnemosyne_read_core",
                json!({"project_path":project,"scope":"global"}),
            ),
            call(
                json!(3),
                "mnemosyne_search",
                json!({"project_path":project,"query":"x","limit":-1}),
            ),
            call(
                json!(4),
                "mnemosyne_search",
                json!({"query":"x","unexpected":true}),
            ),
            call(
                json!(5),
                "mnemosyne_write",
                json!({"type":"codebase","importance":"high","content":"bad"}),
            ),
            call(json!(6), "unknown", json!({})),
            json!({"jsonrpc":"2.0","id":7,"method":"unknown"}),
        ],
    );
    for value in &result[..2] {
        assert!(
            value.get("error").is_none(),
            "business error used JSON-RPC internal error: {value}"
        );
        assert_eq!(value["result"]["isError"], true);
        assert_eq!(value["result"]["content"][0]["type"], "text");
    }
    for value in &result[2..6] {
        assert_eq!(value["error"]["code"], -32602, "{value}");
    }
    assert_eq!(result[6]["error"]["code"], -32601);
}
#[test]
fn initialization_negotiates_the_declared_version_and_preserves_legacy_direct_calls() {
    let t = sandbox();
    let result = replies(
        t.path(),
        &[
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
            init(2, "2099-01-01"),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/list"}),
            init(5, "2024-11-05"),
            json!({"jsonrpc":"2.0","id":"ping-string","method":"ping"}),
        ],
    );
    assert_eq!(result.len(), 6);
    assert_eq!(result[0]["result"]["tools"].as_array().unwrap().len(), 8);
    assert_eq!(result[1]["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(result[2]["error"]["code"], -32002);
    assert_eq!(result[3]["result"]["tools"].as_array().unwrap().len(), 8);
    assert!(result[4].get("error").is_some());
    assert_eq!(result[5]["result"], json!({}));
}
#[test]
fn stdio_is_bounded_and_invalid_utf8_does_not_kill_the_next_request() {
    let t = sandbox();
    let input = b"\xff\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n";
    let output = run(t.path(), input, None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(response[0]["error"]["code"], -32700);
    assert_eq!(response[1]["id"], 2);
    let oversized = vec![b'x'; 1025];
    let output = run(t.path(), &oversized, Some("1024"));
    assert!(
        !output.status.success(),
        "over-limit unterminated line must disconnect"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("INPUT_TOO_LARGE"));
    assert!(output.stdout.is_empty());
    for invalid in ["0", "-1", "not-a-number", "67108865"] {
        let output = run(t.path(), b"", Some(invalid));
        assert!(
            !output.status.success(),
            "invalid input limit {invalid} must not disable the bound"
        );
    }
}
