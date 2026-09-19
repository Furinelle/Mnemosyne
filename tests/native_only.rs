//! Native protocol acceptance: every child has an empty PATH and an isolated home.
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio},
};

struct Sandbox(tempfile::TempDir);
impl Sandbox {
    fn new() -> Self {
        let this = Self(tempfile::tempdir().unwrap());
        for name in ["home", "empty-bin", "global", "tmp", "server"] {
            fs::create_dir(this.root().join(name)).unwrap();
        }
        this
    }
    fn root(&self) -> &Path {
        self.0.path()
    }
    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mnemosyne"));
        command
            .env_clear()
            .current_dir(cwd)
            .args(args)
            .env("HOME", self.root().join("home"))
            .env("USERPROFILE", self.root().join("home"))
            .env("MNEMOSYNE_HOME", self.root().join("global"))
            .env("PATH", self.root().join("empty-bin"))
            .env("TMPDIR", self.root().join("tmp"))
            .env("XDG_CONFIG_HOME", self.root().join("home/config"))
            .env("XDG_DATA_HOME", self.root().join("home/data"))
            .env("XDG_CACHE_HOME", self.root().join("home/cache"));
        command
    }
    fn run(&self, cwd: &Path, args: &[&str], input: &str) -> Output {
        let mut child = self
            .command(cwd, args)
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
    fn ok(&self, cwd: &Path, args: &[&str], input: &str) -> String {
        let output = self.run(cwd, args, input);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn project(&self, name: &str) -> PathBuf {
        let path = self.root().join(name);
        fs::create_dir_all(path.join(".git")).unwrap();
        self.ok(&path, &["init", "--no-agent-files"], "");
        // Keep background maintenance out of deterministic event assertions.
        let now = chrono::Utc::now().to_rfc3339();
        fs::write(path.join(".mnemosyne/.last_maintain"), &now).unwrap();
        fs::write(self.root().join("global/.last_maintain"), &now).unwrap();
        path
    }
    fn write(&self, project: &Path, scope: &str, content: &str) -> String {
        self.ok(
            project,
            &[
                "write",
                "--scope",
                scope,
                "--type",
                "codebase",
                "--importance",
                "70",
                "--title",
                "Native fixture",
                "--content",
                content,
                "--force",
            ],
            "",
        )
        .trim()
        .strip_prefix("Wrote ")
        .unwrap()
        .to_owned()
    }
    fn hook(&self, project: &Path, event: &str, payload: Value) -> Option<Value> {
        let output = self.ok(project, &["hook", event], &payload.to_string());
        if output.trim().is_empty() {
            None
        } else {
            assert_eq!(
                output.lines().count(),
                1,
                "Hook stdout must contain one JSON envelope"
            );
            Some(serde_json::from_str(&output).unwrap())
        }
    }
}

fn access_count(markdown: &str) -> u64 {
    markdown
        .lines()
        .find_map(|line| line.strip_prefix("access_count: "))
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn native_host_event_envelopes_need_no_python_and_never_grant_tool_permission() {
    let sandbox = Sandbox::new();
    for name in ["python", "python3"] {
        let error = Command::new(name)
            .env_clear()
            .env("PATH", sandbox.root().join("empty-bin"))
            .output()
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }
    for host in ["claude-code", "codex", "grok-build"] {
        let project = sandbox.project(host);
        fs::write(
            project.join(".mnemosyne/core.md"),
            format!("# Project Core\n\n{host} native core invariant\n"),
        )
        .unwrap();
        let id = sandbox.write(
            &project,
            "project",
            "nativeonlyneedle worker.rs preserves all originals",
        );
        let start = sandbox
            .hook(
                &project,
                "SessionStart",
                json!({"source":"startup","session_id":host}),
            )
            .unwrap();
        assert_eq!(start["hookSpecificOutput"]["hookEventName"], "SessionStart");
        assert!(
            start["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap()
                .contains(host)
        );
        for source in ["resume", "compact"] {
            assert!(
                sandbox
                    .hook(&project, "SessionStart", json!({"source":source}))
                    .is_none()
            );
        }
        let prompt = json!({"prompt":"Explain nativeonlyneedle and its safety invariant", "session_id":host});
        let turn = sandbox
            .hook(&project, "UserPromptSubmit", prompt.clone())
            .unwrap();
        assert_eq!(
            turn["hookSpecificOutput"]["hookEventName"],
            "UserPromptSubmit"
        );
        assert!(
            turn["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap()
                .contains(&id)
        );
        assert!(sandbox.hook(&project, "UserPromptSubmit", prompt).is_none());
        assert_eq!(access_count(&sandbox.ok(&project, &["show", &id], "")), 1);
        let edit = json!({"tool_name":"Edit","tool_input":{"file_path":"src/worker.rs"},"session_id":format!("{host}-file")});
        let file = sandbox.hook(&project, "PreToolUse", edit).unwrap();
        assert_eq!(file["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert!(
            file["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .unwrap()
                .contains(&id)
        );
        assert!(
            file["hookSpecificOutput"]
                .get("permissionDecision")
                .is_none()
        );
        assert_eq!(
            access_count(&sandbox.ok(&project, &["show", &id], "")),
            1,
            "file_touch must not inflate access by default"
        );
        assert!(
            sandbox
                .hook(
                    &project,
                    "PreToolUse",
                    json!({"tool_name":"Read","tool_input":{"file_path":"src/worker.rs"}})
                )
                .is_none()
        );
        assert!(
            sandbox
                .hook(
                    &project,
                    "Stop",
                    json!({"stop_hook_active":true,"transcript_path":"/must-not-open"})
                )
                .is_none()
        );
        let bad = sandbox.run(&project, &["hook", "UserPromptSubmit"], "not json");
        assert!(bad.status.success() && bad.stdout.is_empty() && !bad.stderr.is_empty());
        fs::write(
            project.join(".mnemosyne/config.toml"),
            "[injection]\nmax_tokens = 1\n",
        )
        .unwrap();
        let tiny = sandbox.run(&project, &["hook", "SessionStart"], "{}");
        assert!(tiny.status.success() && tiny.stdout.is_empty());
        assert!(String::from_utf8_lossy(&tiny.stderr).contains("BUDGET_TOO_SMALL"));
    }
}

#[test]
fn native_stop_accepts_three_transcripts_with_source_roles_and_replay_intact() {
    let sandbox = Sandbox::new();
    for (host, source) in [
        ("claude", "claude-code"),
        ("codex", "codex"),
        ("grok", "grok-build"),
    ] {
        let project = sandbox.project(host);
        fs::write(
            project.join(".mnemosyne/config.toml"),
            "[distill]\nenabled = true\nengine = 'heuristic'\nsession_summary = false\n",
        )
        .unwrap();
        let content = format!("不要用 print 调试，改用 logging，{host}nativefixture。");
        let rows = match host {
            "claude" => vec![
                json!({"type":"user","message":{"role":"user","content":content}}),
                json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"不要用 logging，改用 unsafe-tool-evidence"}]}}),
                json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"错误根因是配置，修复配置。"},{"type":"text","text":"Done."}]}}),
            ],
            "codex" => {
                let mut rows =
                    vec![json!({"type":"session_meta","payload":{"id":"native-test"}}); 125];
                rows.extend([
                    json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":content}]}}),
                    json!({"type":"response_item","payload":{"type":"message","role":"assistant","channel":"analysis","content":[{"type":"output_text","text":"错误根因是配置，修复配置。"}]}}),
                    json!({"type":"response_item","payload":{"type":"function_call_output","output":"不要用 logging，改用 unsafe-tool-evidence"}}),
                    json!({"type":"response_item","payload":{"type":"message","role":"assistant","channel":"final","content":[{"type":"output_text","text":"Done."}]}}),
                ]);
                rows
            }
            _ => vec![
                json!({"type":"user","content":content}),
                json!({"type":"tool","content":"不要用 logging，改用 unsafe-tool-evidence"}),
                json!({"type":"assistant","content":"Done."}),
            ],
        };
        let transcript = project.join("session.jsonl");
        fs::write(
            &transcript,
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let payload = json!({"transcript_path":transcript,"session_id":host});
        assert!(
            sandbox
                .hook(
                    &project,
                    "Stop",
                    json!({"stop_hook_active":true,"transcript_path":transcript})
                )
                .is_none()
        );
        assert_eq!(
            fs::read_dir(project.join(".mnemosyne/working"))
                .unwrap()
                .count(),
            0
        );
        let first = sandbox
            .hook(&project, "Stop", payload.clone())
            .expect("Stop should report the one saved preference");
        assert!(
            first["systemMessage"]
                .as_str()
                .unwrap()
                .contains("auto-saved")
        );
        let paths: Vec<_> = fs::read_dir(project.join(".mnemosyne/working"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "md"))
            .collect();
        assert_eq!(
            paths.len(),
            1,
            "{host}: hidden reasoning/tool content must not become memory"
        );
        let before = fs::read(&paths[0]).unwrap();
        let markdown = String::from_utf8(before.clone()).unwrap();
        assert!(
            markdown.contains(&format!("source: {source}\n")),
            "{markdown}"
        );
        assert!(markdown.contains(&content));
        assert!(!markdown.contains("unsafe-tool-evidence"));
        assert!(
            sandbox.hook(&project, "Stop", payload).is_none(),
            "Replay must not emit a second save receipt"
        );
        assert_eq!(fs::read(&paths[0]).unwrap(), before);
        assert_eq!(
            fs::read_dir(project.join(".mnemosyne/working"))
                .unwrap()
                .filter(|entry| entry
                    .as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|e| e == "md"))
                .count(),
            1
        );
    }
}

struct Mcp {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    sequence: u64,
}
impl Mcp {
    fn start(sandbox: &Sandbox) -> Self {
        let mut child = sandbox
            .command(&sandbox.root().join("server"), &["mcp", "serve"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        Self {
            input: child.stdin.take(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
            sequence: 0,
        }
    }
    fn send(&mut self, request: Value) -> Value {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{request}").unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        assert!(
            self.output.read_line(&mut line).unwrap() > 0,
            "MCP closed before its response"
        );
        let response: Value =
            serde_json::from_str(&line).expect("MCP stdout must contain only JSON-RPC messages");
        assert_eq!(response["id"], request["id"]);
        assert_eq!(response["jsonrpc"], "2.0");
        response
    }
    fn call(&mut self, name: &str, args: Value) -> Value {
        self.sequence += 1;
        self.send(json!({"jsonrpc":"2.0","id":self.sequence,"method":"tools/call","params":{"name":name,"arguments":args}}))
    }
    fn ok(&mut self, name: &str, args: Value) -> Value {
        let response = self.call(name, args);
        assert!(response.get("error").is_none(), "{name}: {response}");
        let result = &response["result"];
        assert!(result["content"].is_array());
        assert!(result["structuredContent"].is_object());
        result["structuredContent"].clone()
    }
    fn close(&mut self) {
        drop(self.input.take());
        let status = self.child.wait().unwrap();
        let mut stderr = String::new();
        self.child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        assert!(status.success(), "{stderr}");
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn antigravity_native_mcp_keeps_two_projects_and_hidden_global_memory_separate() {
    let sandbox = Sandbox::new();
    let a = sandbox.project("alpha");
    let b = sandbox.project("beta");
    for (project, core) in [(&a, "ALPHA_CORE_ONLY"), (&b, "BETA_CORE_ONLY")] {
        fs::write(project.join(".mnemosyne/core.md"), core).unwrap();
        fs::write(
            project.join(".mnemosyne/config.toml"),
            "[mcp]\nexpose_global = false\nexpose_project = true\n",
        )
        .unwrap();
    }
    let global = sandbox.write(&a, "global", "GLOBAL_PRIVATE_NEVER_EXPOSE");
    fs::write(sandbox.root().join("global/core.md"), "GLOBAL_CORE_PRIVATE").unwrap();
    let mut mcp = Mcp::start(&sandbox);
    let initialized=mcp.send(json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"antigravity-native-fixture","version":"1"}}}));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "mnemosyne");
    writeln!(
        mcp.input.as_mut().unwrap(),
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .unwrap();
    let schema = mcp.send(json!({"jsonrpc":"2.0","id":"schema","method":"tools/list"}));
    for tool in schema["result"]["tools"].as_array().unwrap() {
        assert_eq!(
            tool["inputSchema"]["properties"]["project_path"]["type"],
            "string"
        );
    }
    let first=mcp.ok("mnemosyne_write",json!({"project_path":a,"type":"codebase","importance":70,"title":"Alpha one","content":"alphaexclusive durable fact one","source":"antigravity"}))["id"].as_str().unwrap().to_owned();
    let second=mcp.ok("mnemosyne_write",json!({"project_path":a,"type":"codebase","importance":70,"title":"Alpha two","content":"alphaexclusive durable fact two","source":"antigravity"}))["id"].as_str().unwrap().to_owned();
    let other=mcp.ok("mnemosyne_write",json!({"project_path":b,"type":"codebase","importance":70,"title":"Beta","content":"betaexclusive private fact","source":"antigravity"}))["id"].as_str().unwrap().to_owned();
    assert!(!sandbox.root().join("server/.mnemosyne").exists());
    for (project, query, expected) in [(&a, "alphaexclusive", 2), (&b, "betaexclusive", 1)] {
        let hits = mcp.ok(
            "mnemosyne_search",
            json!({"project_path":project,"query":query}),
        );
        assert_eq!(hits["items"].as_array().unwrap().len(), expected);
    }
    assert_eq!(
        mcp.ok(
            "mnemosyne_search",
            json!({"project_path":b,"query":"alphaexclusive"})
        )["items"],
        json!([])
    );
    assert_eq!(
        mcp.ok("mnemosyne_read_core", json!({"project_path":a})),
        json!({"project":"ALPHA_CORE_ONLY"})
    );
    let shown = mcp.ok("mnemosyne_show", json!({"project_path":a,"id":first}));
    assert!(
        shown["text"]
            .as_str()
            .unwrap()
            .contains("source: antigravity")
    );
    assert_eq!(
        access_count(shown["text"].as_str().unwrap()),
        0,
        "MCP search must stay read-only"
    );
    assert!(
        mcp.call("mnemosyne_show", json!({"project_path":b,"id":first}))
            .get("error")
            .is_some()
    );
    assert!(
        mcp.call("mnemosyne_show", json!({"project_path":a,"id":global}))
            .get("error")
            .is_some()
    );
    assert!(
        mcp.call(
            "mnemosyne_read_core",
            json!({"project_path":a,"scope":"global"})
        )
        .get("error")
        .is_some()
    );
    assert!(mcp.call("mnemosyne_write",json!({"project_path":a,"scope":"global","type":"codebase","importance":70,"content":"unauthorized"})).get("error").is_some());
    assert!(
        mcp.call(
            "mnemosyne_link",
            json!({"project_path":a,"id1":first,"id2":other,"rel":"related"})
        )
        .get("error")
        .is_some()
    );
    mcp.ok(
        "mnemosyne_link",
        json!({"project_path":a,"id1":first,"id2":second,"rel":"related"}),
    );
    let graph = mcp.ok(
        "mnemosyne_graph",
        json!({"project_path":a,"id":first,"format":"ascii","depth":1}),
    );
    let graph = graph["text"].as_str().unwrap();
    assert!(graph.contains(&first) && graph.contains(&second) && !graph.contains(&other));
    let prep = mcp.ok(
        "mnemosyne_prep_context",
        json!({"project_path":a,"task":"alphaexclusive"}),
    );
    let prep = prep["text"].as_str().unwrap();
    assert!(prep.contains("ALPHA_CORE_ONLY") && prep.contains(&first));
    assert!(
        !prep.contains("BETA_CORE_ONLY")
            && !prep.contains("GLOBAL_CORE_PRIVATE")
            && !prep.contains(&other)
    );
    assert!(prep.contains("mnemosyne_show") && !prep.contains("python"));
    let first_path = a.join(".mnemosyne/working").join(format!("{first}.md"));
    let other_path = b.join(".mnemosyne/working").join(format!("{other}.md"));
    let before = fs::read(&first_path).unwrap();
    assert_eq!(
        access_count(std::str::from_utf8(&before).unwrap()),
        0,
        "MCP prep must not inflate usage"
    );
    let other_before = fs::read(&other_path).unwrap();
    let dry = mcp.ok(
        "mnemosyne_maintain",
        json!({"project_path":a,"dry_run":true}),
    );
    assert_eq!(dry["processed"], 2);
    assert!(dry.get("core_candidates").is_none());
    assert_eq!(fs::read(&first_path).unwrap(), before);
    mcp.ok(
        "mnemosyne_maintain",
        json!({"project_path":a,"dry_run":false}),
    );
    assert_ne!(fs::read(&first_path).unwrap(), before);
    assert_eq!(fs::read(&other_path).unwrap(), other_before);
    assert!(
        mcp.call(
            "mnemosyne_write",
            json!({"type":"codebase","importance":70,"content":"no implicit cwd project"})
        )
        .get("error")
        .is_some()
    );
    assert!(
        mcp.call("mnemosyne_read_core", json!({"project_path":"../alpha"}))
            .get("error")
            .is_some()
    );
    assert!(!sandbox.root().join("server/.mnemosyne").exists());
    mcp.close();
}
