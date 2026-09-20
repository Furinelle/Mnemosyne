//! R04/R05 transport contracts use only temporary stores and host state.
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

struct Sandbox(tempfile::TempDir);
impl Sandbox {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        for dir in ["home", "global", "project/.git", "tmp"] {
            fs::create_dir_all(temp.path().join(dir)).unwrap();
        }
        let sandbox = Self(temp);
        sandbox.ok(&["init", "--no-agent-files"], "");
        sandbox
    }
    fn project(&self) -> PathBuf {
        self.0.path().join("project")
    }
    fn run(&self, args: &[&str], input: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mnemosyne"))
            .env_clear()
            .env("HOME", self.0.path().join("home"))
            .env("MNEMOSYNE_HOME", self.0.path().join("global"))
            .env("TMPDIR", self.0.path().join("tmp"))
            .env("PATH", "/usr/bin:/bin")
            .current_dir(self.project())
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
        child.wait_with_output().unwrap()
    }
    fn ok(&self, args: &[&str], input: &str) -> String {
        let out = self.run(args, input);
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }
    fn json(&self, args: &[&str], input: &str) -> Value {
        serde_json::from_str(&self.ok(args, input)).unwrap()
    }
    fn mcp(&self, requests: &[Value]) -> Vec<Value> {
        let input = requests
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        self.ok(&["mcp", "serve"], &input)
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
}
fn call(id: u32, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})
}
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}
fn snapshot(from: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn collect(base: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(base, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(base).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    collect(from, from, &mut files);
    files
}
fn memory_request(event: &str) -> Value {
    json!({"type":"codebase","title":"Quartz routing","content":"quartz migration uses port 8080","importance":70,"source":"fixture","origin":"m1-test","source_session_id":"s","source_event_id":event,"finding_key":"f","source_kind":"code_observation","verification_state":"unverified"})
}

#[test]
fn r04_a07_cli_mcp_versions_preview_and_copy_restore() {
    let s = Sandbox::new();
    let root = s.project().join(".mnemosyne");
    let legacy = s.ok(
        &[
            "write",
            "--type",
            "codebase",
            "--importance",
            "70",
            "--title",
            "Legacy",
            "--content",
            "legacy fixture",
            "--force",
        ],
        "",
    );
    let legacy_id = legacy.trim().strip_prefix("Wrote ").unwrap();
    let before_config = fs::read(root.join("config.toml")).unwrap();
    let before = snapshot(&root);
    let pre = s.0.path().join("pre-upgrade");
    copy_tree(&root, &pre);
    let preview = s.json(&["store-upgrade"], "");
    assert_eq!(preview["status"], "upgrade_available");
    assert!(!root.join("store.json").exists());
    assert_eq!(
        snapshot(&root),
        before,
        "upgrade preview changed a store file"
    );
    assert_eq!(fs::read(root.join("config.toml")).unwrap(), before_config);
    let manifest = s.json(&["store-upgrade", "--commit"], "");
    assert_eq!(manifest["schema_version"], 2);
    let cli_write = s.json(&["write-v2"], &memory_request("cli-1").to_string());
    let cli_ref = &cli_write["memory_ref"];
    let cli_id = cli_ref["memory_id"].as_str().unwrap();
    let store_id = cli_ref["store_id"].as_str().unwrap();
    let cli_show = s.json(&["show-v2", cli_id, "--store-id", store_id], "");
    assert_eq!(cli_show["memory_ref"], *cli_ref);
    let replies = s.mcp(&[
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        call(2, "mnemosyne_show", json!({"version":2,"id":cli_id,"store_id":store_id})),
        call(3, "mnemosyne_write", json!({"version":2,"operation":"memory","request":memory_request("mcp-2")})),
        call(4, "mnemosyne_show", json!({"id":legacy_id})),
        call(5, "mnemosyne_write", json!({"version":2,"operation":"memory","request":memory_request("mixed"),"type":"codebase"})),
        call(6, "mnemosyne_show", json!({"id":legacy_id,"store_id":store_id})),
    ]);
    assert_eq!(replies[0]["result"]["tools"].as_array().unwrap().len(), 8);
    assert_eq!(replies[1]["result"]["structuredContent"], cli_show);
    let mcp_write = &replies[2]["result"]["structuredContent"];
    assert_eq!(mcp_write["status"], "created");
    let mcp_id = mcp_write["memory_ref"]["memory_id"].as_str().unwrap();
    assert_eq!(
        s.json(&["show-v2", mcp_id, "--store-id", store_id], "")["memory_ref"],
        mcp_write["memory_ref"]
    );
    assert!(
        replies[3]["result"]["structuredContent"]["text"]
            .as_str()
            .unwrap()
            .contains("legacy fixture")
    );
    assert_eq!(replies[4]["error"]["code"], -32602);
    assert_eq!(replies[5]["error"]["code"], -32602);
    let old_write = s.run(
        &[
            "write",
            "--type",
            "codebase",
            "--importance",
            "70",
            "--title",
            "Blocked",
            "--content",
            "do not route",
            "--force",
        ],
        "",
    );
    assert!(!old_write.status.success());
    assert!(String::from_utf8_lossy(&old_write.stderr).contains("INCOMPATIBLE_SCHEMA"));
    assert!(s.ok(&["show", legacy_id], "").contains("legacy fixture"));
    let upgraded = s.0.path().join("upgraded-copy");
    copy_tree(&root, &upgraded);
    fs::remove_dir_all(&root).unwrap();
    copy_tree(&upgraded, &root);
    assert_eq!(
        s.json(&["show-v2", cli_id, "--store-id", store_id], "")["memory_ref"],
        *cli_ref
    );
    fs::remove_dir_all(&root).unwrap();
    copy_tree(&pre, &root);
    assert!(!root.join("store.json").exists());
    assert_eq!(fs::read(root.join("config.toml")).unwrap(), before_config);
    assert!(s.ok(&["show", legacy_id], "").contains("legacy fixture"));
    assert!(
        s.ok(
            &[
                "write",
                "--type",
                "codebase",
                "--importance",
                "70",
                "--title",
                "Restored",
                "--content",
                "legacy after restore",
                "--force"
            ],
            ""
        )
        .contains("Wrote ")
    );
}

#[test]
fn r05_a06_cli_mcp_hook_share_candidates_and_estimated_budget() {
    let s = Sandbox::new();
    s.json(&["store-upgrade", "--commit"], "");
    fs::write(s.project().join("task.txt"), "quartz migration").unwrap();
    let memory = s.json(&["write-v2"], &memory_request("context-1").to_string());
    let memory_id = memory["memory_ref"]["memory_id"].as_str().unwrap();
    let checkpoint = s.json(&["checkpoint", "new"], &json!({"task_id":"task-1","goal":"finish quartz migration","scoped_paths":["task.txt"],"next_action":"check routing","expires":"2099-01-01T00:00:00Z"}).to_string());
    let checkpoint_id = checkpoint["id"].as_str().unwrap();
    let cli = s.json(
        &[
            "prep",
            "quartz migration",
            "--format",
            "json",
            "--limit",
            "3",
            "--budget",
            "2000",
            "--task-id",
            "task-1",
        ],
        "",
    );
    let mcp = s.mcp(&[call(
        1,
        "mnemosyne_prep_context",
        json!({"version":2,"task":"quartz migration","limit":3,"budget":2000,"task_id":"task-1"}),
    )]);
    let remote = &mcp[0]["result"]["structuredContent"];
    assert_eq!(cli["selected"], remote["selected"]);
    assert_eq!(cli["omitted"], remote["omitted"]);
    assert!(
        cli["selected"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["id"] == memory_id)
    );
    assert!(
        cli["selected"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["id"] == checkpoint_id)
    );
    for bundle in [&cli, remote] {
        assert_eq!(bundle["budget_mode"], "estimated");
        assert!(bundle["estimated_tokens"].as_u64().unwrap() <= 2000);
    }
    let hook = s.json(&["hook", "UserPromptSubmit"], &json!({"prompt":"quartz migration","host":"claude-code","session_id":"hook-s","task_id":"task-1","budget":2000}).to_string());
    let context = hook["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains(memory_id) && context.contains(checkpoint_id));
    assert!(mnemosyne::context::approx_tokens(context) <= 2000);
}
