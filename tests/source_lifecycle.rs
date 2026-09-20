//! R01 regressions. All commands run with an empty environment and private stores.
use mnemosyne::{
    schema::Memory,
    store::{Store, ensure_store, load_memories_unlocked, working_path, write_memory},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

struct Sandbox {
    temp: tempfile::TempDir,
    store: Store,
}
impl Sandbox {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        for name in ["home", "global", "empty-bin", "tmp", "project/.git"] {
            fs::create_dir_all(temp.path().join(name)).unwrap();
        }
        let store = Store {
            scope: "project".into(),
            root: temp.path().join("project/.mnemosyne"),
        };
        ensure_store(&store).unwrap();
        Self { temp, store }
    }
    fn root(&self) -> &Path {
        self.temp.path()
    }
    fn run(&self, args: &[&str], input: &str) -> String {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mnemosyne"))
            .env_clear()
            .current_dir(self.root().join("project"))
            .args(args)
            .env("HOME", self.root().join("home"))
            .env("USERPROFILE", self.root().join("home"))
            .env("MNEMOSYNE_HOME", self.root().join("global"))
            .env("PATH", self.root().join("empty-bin"))
            .env("TMPDIR", self.root().join("tmp"))
            .env("XDG_CONFIG_HOME", self.root().join("home/config"))
            .env("XDG_DATA_HOME", self.root().join("home/data"))
            .env("XDG_CACHE_HOME", self.root().join("home/cache"))
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
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn config(&self, text: &str) {
        fs::write(self.store.config_path(), text).unwrap();
    }
    fn memories(&self, archive: bool) -> Vec<Memory> {
        load_memories_unlocked(&self.store, archive)
            .unwrap()
            .into_iter()
            .map(|(_, m)| m)
            .collect()
    }
    fn fixture(&self, id: &str, status: &str, strength: i64, accesses: i64, expires: &str) {
        let mut memory = Memory {
            id: id.into(),
            memory_type: "codebase".into(),
            source: "fixture".into(),
            status: status.into(),
            strength,
            access_count: accesses,
            expires: expires.into(),
            body: format!("## Lifecycle\n\nrelayfixture {id}"),
            injection_summary: format!("relayfixture {id}"),
            ..Default::default()
        };
        if status == "superseded" {
            memory
                .extra
                .insert("invalidated_by".into(), json!("replacement"));
        }
        write_memory(&working_path(&self.store, &memory).unwrap(), &memory).unwrap();
    }
    fn maintain(&self) -> Value {
        serde_json::from_str(&self.run(&["maintain", "--scope", "project"], "")).unwrap()
    }
    fn search(&self, archive: bool, superseded: bool) -> Value {
        let mut args = vec![
            "search",
            "relayfixture",
            "--scope",
            "project",
            "--format",
            "json",
        ];
        if archive {
            args.push("--archive");
        }
        if superseded {
            args.push("--include-superseded");
        }
        serde_json::from_str(&self.run(&args, "")).unwrap()
    }
    fn write(&self, content: &str, source: &str, extra: &[&str]) {
        let mut args = vec![
            "write",
            "--type",
            "codebase",
            "--importance",
            "70",
            "--title",
            "Service config",
            "--content",
            content,
            "--source",
            source,
        ];
        args.extend_from_slice(extra);
        self.run(&args, "");
    }
    fn ingress(&self, adapter: &str, source: &str) {
        match adapter {
            "cli" => self.write("port 8080", source, &[]),
            "mcp" => {
                let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mnemosyne_write","arguments":{"type":"codebase","importance":70,"title":"Service config","content":"port 8080","source":source}}});
                let reply: Value =
                    serde_json::from_str(&self.run(&["mcp", "serve"], &request.to_string()))
                        .unwrap();
                assert!(reply.get("error").is_none(), "{reply}");
                assert_ne!(reply["result"]["isError"], true, "{reply}");
            }
            "ingest" => {
                let findings = json!([{"type":"codebase","importance":70,"title":"Service config","content":"port 8080"}]);
                self.run(
                    &["ingest", "--commit", "--format", "json", "--source", source],
                    &findings.to_string(),
                );
            }
            "distill" => {
                self.config("[distill]\nengine = 'host'\n");
                let turn = json!({"role":"assistant","text":"**Findings:**\n- type: codebase\n- importance: 70\n- title: Service config\n- content: |\n    port 8080\n"});
                self.run(
                    &[
                        "distill",
                        "--stdin",
                        "--format",
                        "role-jsonl",
                        "--commit",
                        "--source",
                        source,
                    ],
                    &turn.to_string(),
                );
            }
            _ => panic!("unknown adapter"),
        }
    }
}

#[test]
fn r01_a01_all_ingress_preserves_sources_and_same_source_replay() {
    for adapter in ["cli", "mcp", "ingest", "distill"] {
        let sandbox = Sandbox::new();
        sandbox.ingress(adapter, "codex");
        sandbox.ingress(adapter, "codex");
        assert_eq!(sandbox.memories(false).len(), 1, "{adapter}: exact replay");
        sandbox.ingress(adapter, "claude-code");
        let memories = sandbox.memories(false);
        assert_eq!(memories.len(), 2, "{adapter}: second source was lost");
        assert!(memories.iter().any(|m| m.source == "codex"));
        assert!(memories.iter().any(|m| m.source == "claude-code"));
    }
    let sandbox = Sandbox::new();
    for source in ["", "agent", "  AGENT  ", "   "] {
        sandbox.ingress("mcp", source);
    }
    assert_eq!(
        sandbox.memories(false).len(),
        1,
        "effective default source must be shared"
    );
    assert_eq!(sandbox.memories(false)[0].source, "agent");
    sandbox.write("port 8080", "agent", &["--allow-duplicate"]);
    assert_eq!(sandbox.memories(false).len(), 2);
}

#[test]
fn r01_a02_complementary_and_changed_ports_coexist() {
    let sandbox = Sandbox::new();
    for content in ["port 8080", "healthcheck /health", "port 8081"] {
        sandbox.write(content, "codex", &[]);
    }
    let memories = sandbox.memories(false);
    assert_eq!(memories.len(), 3);
    assert!(memories.iter().all(|m| m.status == "active"));
}

#[test]
fn r01_a03_small_text_changes_and_metadata_are_not_duplicates() {
    let sandbox = Sandbox::new();
    let content = format!(
        "{}\nversion 1.2.3; must enable authentication.",
        "Firewall certificate routing logging retry timeout and storage must be checked. "
            .repeat(10)
    );
    sandbox.write(&content, "codex", &[]);
    sandbox.write(&content.replace("1.2.3", "1.2.4"), "codex", &[]);
    sandbox.write(
        &content.replace("must enable", "must not enable"),
        "codex",
        &[],
    );
    sandbox.write(&content, "codex", &["--evidence", "review:42"]);
    sandbox.write(&content, "codex", &["--expires", "2099-01-01"]);
    sandbox.write(&content, "codex", &["--tags", "first,second"]);
    sandbox.write(&content, "codex", &["--tags", "second,first"]);
    assert_eq!(sandbox.memories(false).len(), 7);
}

#[test]
fn r01_a04_working_superseded_stays_invalid_after_repeated_maintenance() {
    let sandbox = Sandbox::new();
    sandbox.config(
        "[thresholds]\narchive_strength = -100\ndeprecated_strength = 5\ndecay_per_run = 1\n",
    );
    sandbox.fixture("old", "superseded", 4, 0, "");
    for _ in 0..2 {
        sandbox.maintain();
    }
    let memories = sandbox.memories(false);
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].status, "superseded");
    assert_eq!(memories[0].strength, 2, "legacy per-run decay must remain");
    assert_eq!(memories[0].extra["invalidated_by"], "replacement");
    assert!(sandbox.search(false, false).as_array().unwrap().is_empty());
}

#[test]
fn r01_a05_archiving_does_not_make_superseded_searchable() {
    let sandbox = Sandbox::new();
    sandbox.fixture("old", "superseded", 4, 0, "");
    assert_eq!(sandbox.maintain()["archived"], 1);
    assert!(sandbox.memories(false).is_empty());
    let memories = sandbox.memories(true);
    assert_eq!(memories[0].status, "superseded");
    assert_eq!(memories[0].extra["invalidated_by"], "replacement");
    assert!(sandbox.search(true, false).as_array().unwrap().is_empty());
    assert_eq!(sandbox.search(true, true).as_array().unwrap().len(), 1);
}

#[test]
fn r01_a06_superseded_never_becomes_core_and_valid_behavior_is_unchanged() {
    let sandbox = Sandbox::new();
    sandbox.config(
        "[thresholds]\narchive_strength = -100\ndeprecated_strength = 5\ndecay_per_run = 1\n",
    );
    sandbox.fixture("old", "superseded", 95, 10, "");
    sandbox.fixture("valid", "active", 95, 10, "");
    sandbox.fixture("cold", "active", 4, 0, "");
    sandbox.fixture("expired", "active", 90, 10, "2000-01-01");
    let report = sandbox.maintain();
    let candidates = report["core_candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 1, "superseded fact was offered for core");
    assert_eq!(candidates[0]["id"], "valid");
    assert_eq!(report["archived"], 1);
    assert_eq!(report["deprecated"], 1);
    let memories = sandbox.memories(true);
    let get = |id: &str| memories.iter().find(|m| m.id == id).unwrap();
    assert_eq!(
        (get("old").status.as_str(), get("old").strength),
        ("superseded", 94)
    );
    assert_eq!(
        (get("valid").status.as_str(), get("valid").strength),
        ("active", 94)
    );
    assert_eq!(
        (get("cold").status.as_str(), get("cold").strength),
        ("deprecated", 3)
    );
    assert_eq!(
        get("expired").strength,
        90,
        "expired records must not decay"
    );
    assert!(
        !sandbox
            .search(false, false)
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == "expired")
    );
}
