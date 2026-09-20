use serde_json::{Value, json};
use std::{
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
fn run(root: &Path, args: &[&str], value: &Value) -> Output {
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
        .write_all((value.to_string() + "\n").as_bytes())
        .unwrap();
    p.wait_with_output().unwrap()
}
fn ok(root: &Path, args: &[&str], value: &Value) -> Value {
    let r = run(root, args, value);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    serde_json::from_slice(&r.stdout).unwrap()
}
fn sandbox() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir(t.path().join(".git")).unwrap();
    for file in ["a.rs", "b.rs"] {
        std::fs::write(t.path().join(file), "before").unwrap();
    }
    let r = run(t.path(), &["init", "--no-agent-files"], &json!({}));
    assert!(r.status.success());
    ok(t.path(), &["store-upgrade", "--commit"], &json!({}));
    t
}
fn data(source: &str) -> Value {
    json!({"task_id":"task-1","goal":"Repair two modules","completed_actions":["changed a.rs","changed b.rs"],"artifacts":["a.rs","b.rs"],"unresolved":["test a fails","test b fails"],"next_action":"fix tests","source_session":"session-1","source_agent":source,"scoped_paths":["a.rs","b.rs"],"expires":"2099-01-01T00:00:00Z"})
}
fn rpc(root: &Path, name: &str, args: Value) -> Value {
    let reply = ok(
        root,
        &["mcp", "serve"],
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}}),
    );
    assert!(reply.get("error").is_none(), "{reply}");
    assert_ne!(reply["result"]["isError"], true, "{reply}");
    reply["result"]["structuredContent"].clone()
}
#[test]
fn r06_failures_plans_and_changed_workspace_remain_honest() {
    let t = sandbox();
    let root = t.path();
    std::fs::write(root.join("a.rs"), "changed a").unwrap();
    std::fs::write(root.join("b.rs"), "changed b").unwrap();
    let fingerprint = ok(
        root,
        &["checkpoint", "observe", "--paths", "a.rs,b.rs"],
        &json!({}),
    )["fingerprint"]
        .clone();
    let mut request = data("codex");
    let mut tests = Vec::new();
    for command in ["test 1 -eq 2", "test 2 -eq 3"] {
        let failure = Command::new("/bin/sh")
            .args(["-c", command])
            .current_dir(root)
            .env_clear()
            .status()
            .unwrap()
            .code()
            .unwrap();
        assert_eq!(failure, 1);
        tests.push(json!({"command":command,"cwd":".","worktree_fingerprint":fingerprint,"exit_code":failure,"executed_at":"2026-09-20T00:00:00Z"}));
    }
    tests.push(json!({"command":"planned full suite","cwd":"."}));
    request["tests"] = json!(tests);
    let checkpoint = ok(root, &["checkpoint", "new"], &request);
    let id = checkpoint["id"].as_str().unwrap();
    let view = ok(root, &["checkpoint", "load", id], &json!({}));
    assert_eq!(view["verification"], "reported_only");
    assert_eq!(view["test_results"][0]["status"], "reported_fail");
    assert_eq!(view["test_results"][1]["status"], "reported_fail");
    assert_eq!(view["test_results"][2]["status"], "not_executed_or_unknown");
    assert_eq!(view["worktree_matches"], true);
    let before = view["checkpoint"]["worktree"]["observed_commit"].clone();
    std::fs::write(root.join("a.rs"), "changed c").unwrap();
    let changed = ok(root, &["checkpoint", "load", id], &json!({}));
    assert_eq!(changed["worktree_matches"], false);
    assert_eq!(changed["current_worktree"]["observed_commit"], before);
    assert_eq!(changed["test_results"][0]["needs_revalidation"], true);
    assert_eq!(changed["test_results"][0]["status"], "reported_fail");
    let mut forged = request.clone();
    forged["tests"][0]["origin"] = json!("observed");
    assert!(!run(root, &["checkpoint", "new"], &forged).status.success());
    let mut bad = request;
    bad["scoped_paths"] = json!(["../outside"]);
    assert!(!run(root, &["checkpoint", "new"], &bad).status.success());
}
#[test]
fn r06_updates_compare_revision_and_close_preserves_record() {
    let t = sandbox();
    let root = t.path();
    let initial = ok(root, &["checkpoint", "new"], &data("codex"));
    let id = initial["id"].as_str().unwrap();
    let results = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            run(
                root,
                &["checkpoint", "update", id, "--expected-revision", "1"],
                &data("claude-code"),
            )
        });
        let b = scope.spawn(|| {
            run(
                root,
                &["checkpoint", "update", id, "--expected-revision", "1"],
                &data("grok"),
            )
        });
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_eq!(results.iter().filter(|r| r.status.success()).count(), 1);
    assert!(
        results
            .iter()
            .any(|r| String::from_utf8_lossy(&r.stderr).contains("REVISION_CONFLICT"))
    );
    let closed = ok(
        root,
        &["checkpoint", "close", id, "--expected-revision", "2"],
        &json!({}),
    );
    assert_eq!(closed["state"], "closed");
    assert_eq!(closed["revision"], 3);
    let loaded = ok(root, &["checkpoint", "load", id], &json!({}));
    assert_eq!(
        loaded["checkpoint"]["data"]["unresolved"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        !run(
            root,
            &["checkpoint", "update", id, "--expected-revision", "3"],
            &data("codex")
        )
        .status
        .success()
    );
    assert_eq!(
        std::fs::read_dir(root.join(".mnemosyne/working"))
            .unwrap()
            .count(),
        0
    );
}
#[test]
fn r06_twelve_directed_host_handoffs_use_same_native_contract() {
    let t = sandbox();
    let root = t.path();
    let hosts = ["codex", "claude-code", "grok", "antigravity"];
    let mut count = 0;
    for writer in hosts {
        for reader in hosts {
            if writer == reader {
                continue;
            }
            let input = data(writer);
            let record = if writer == "antigravity" {
                rpc(
                    root,
                    "mnemosyne_write",
                    json!({"version":2,"operation":"checkpoint_new","request":input}),
                )
            } else {
                ok(root, &["checkpoint", "new"], &input)
            };
            let id = record["id"].as_str().unwrap();
            let view = if reader == "antigravity" {
                rpc(
                    root,
                    "mnemosyne_show",
                    json!({"version":2,"kind":"checkpoint","id":id}),
                )
            } else {
                ok(root, &["checkpoint", "load", id], &json!({}))
            };
            assert_eq!(view["checkpoint"]["data"]["source_agent"], writer);
            assert_eq!(
                view["checkpoint"]["data"]["unresolved"],
                json!(["test a fails", "test b fails"])
            );
            assert_eq!(view["verification"], "reported_only");
            count += 1;
        }
    }
    assert_eq!(count, 12);
}

#[test]
fn r06_expiry_close_and_corrupt_records_do_not_leak_into_context() {
    use mnemosyne::{checkpoint, provenance::Clock, store::Store};
    struct Time(chrono::DateTime<chrono::Utc>);
    impl Clock for Time {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            self.0
        }
    }
    let t = sandbox();
    let store = Store {
        scope: "project".into(),
        root: t.path().join(".mnemosyne"),
    };
    let now = Time("2026-09-20T00:00:00Z".parse().unwrap());
    let later = Time("2100-01-01T00:00:00Z".parse().unwrap());
    let input = serde_json::from_value(data("codex")).unwrap();
    let record = checkpoint::create(&store, input, &now).unwrap();
    assert!(
        checkpoint::active_context(std::slice::from_ref(&store), None, &now)
            .unwrap()
            .is_empty()
    );
    let active =
        checkpoint::active_context(std::slice::from_ref(&store), Some("task-1"), &now).unwrap();
    assert_eq!(active.len(), 1);
    assert!(
        checkpoint::active_context(std::slice::from_ref(&store), Some("task-1"), &later)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        checkpoint::load(&store, &record.id, &later).unwrap()["expired"],
        true
    );
    let mut renewal = data("codex");
    renewal["expires"] = json!("2200-01-01T00:00:00Z");
    assert!(
        checkpoint::update(
            &store,
            &record.id,
            1,
            serde_json::from_value(renewal).unwrap(),
            &later
        )
        .is_err()
    );
    std::fs::write(store.root.join("checkpoints/invalid.json"), "{").unwrap();
    let with_warning =
        checkpoint::active_context(std::slice::from_ref(&store), Some("task-1"), &now).unwrap();
    assert_eq!(with_warning.len(), 2);
    assert_eq!(with_warning[1]["warnings"][0], "checkpoint_read_failed");
    checkpoint::close(&store, &record.id, 1, &now).unwrap();
    let closed =
        checkpoint::active_context(std::slice::from_ref(&store), Some("task-1"), &now).unwrap();
    assert!(closed.iter().all(|v| v["kind"] != "checkpoint"));
    assert_eq!(
        checkpoint::load(&store, &record.id, &now).unwrap()["checkpoint"]["state"],
        "closed"
    );
    let manifest_path = store.root.join("store.json");
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["min_writer_version"] = json!(99);
    std::fs::write(manifest_path, manifest.to_string()).unwrap();
    assert!(
        checkpoint::create(&store, serde_json::from_value(data("codex")).unwrap(), &now).is_err()
    );
}

#[test]
fn r06_real_commit_does_not_hide_dirty_content_or_allow_symlinks() {
    use mnemosyne::{checkpoint, store::Store};
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    for args in [
        vec!["init", "-q"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "fixture",
        ],
    ] {
        let result = Command::new("/usr/bin/git")
            .args(args)
            .current_dir(root)
            .env_clear()
            .env("HOME", root)
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    std::fs::write(root.join("a.rs"), "before").unwrap();
    let store = Store {
        scope: "project".into(),
        root: root.join(".mnemosyne"),
    };
    let before = checkpoint::observe(&store, &["a.rs".into()]).unwrap();
    assert!(before.observed_commit.is_some());
    std::fs::write(root.join("a.rs"), "after").unwrap();
    let after = checkpoint::observe(&store, &["a.rs".into()]).unwrap();
    assert_eq!(before.observed_commit, after.observed_commit);
    assert_ne!(before.fingerprint, after.fingerprint);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("a.rs", root.join("link.rs")).unwrap();
        assert!(checkpoint::observe(&store, &["link.rs".into()]).is_err());
    }
}
