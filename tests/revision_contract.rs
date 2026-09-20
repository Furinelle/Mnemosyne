use serde_json::{Value, json};
use std::{
    fs,
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
    let out = run(root, args, value);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn setup() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join(".git")).unwrap();
    assert!(
        run(t.path(), &["init", "--no-agent-files"], &json!({}))
            .status
            .success()
    );
    ok(t.path(), &["store-upgrade", "--commit"], &json!({}));
    t
}
fn write(root: &Path, event: &str, importance: u32) -> Value {
    ok(root,&["write-v2"],&json!({"type":"codebase","title":"Quartz routing","content":format!("routing port {event}"),"importance":importance,"origin":"fixture","source_session_id":"s","source_event_id":event,"finding_key":"f","source_kind":"code_observation","verification_state":"unverified"}))["memory_ref"].clone()
}
fn show(root: &Path, reference: &Value) -> Value {
    ok(
        root,
        &["show-v2", reference["memory_id"].as_str().unwrap()],
        &json!({}),
    )
}
fn history(root: &Path, reference: &Value) -> Value {
    serde_json::from_slice(
        &fs::read(
            root.join(".mnemosyne/history")
                .join(reference["memory_id"].as_str().unwrap())
                .join("manifest.json"),
        )
        .unwrap(),
    )
    .unwrap()
}
#[test]
fn revisions_cli_mcp_cas_heat_and_canonical_rebuild() {
    let t = setup();
    let root = t.path();
    let reference = write(root, "8080", 70);
    let first = show(root, &reference);
    assert_eq!(first["revision"]["semantic_rev"], 1);
    ok(root, &["search", "routing", "--format", "json"], &json!({}));
    let heated = show(root, &reference);
    assert_eq!(heated["revision"]["semantic_rev"], 1);
    assert!(
        heated["memory"]["access_count"].as_i64().unwrap()
            > first["memory"]["access_count"].as_i64().unwrap()
    );
    let request = json!({"memory_ref":reference,"expected_rev":1,"expected_hash":first["revision"]["semantic_hash"],"changes":{"body":"## Quartz routing\n\nrouting port 9090","tags":["corrected"]}});
    let second = ok(root, &["revise-v2"], &request);
    assert_eq!(second["revision"]["semantic_rev"], 2);
    let stale = run(root, &["revise-v2"], &request);
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("REVISION_CONFLICT"));
    let current = show(root, &reference);
    assert_eq!(
        current["memory"]["access_count"],
        heated["memory"]["access_count"]
    );
    let rpc = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mnemosyne_write","arguments":{"version":2,"operation":"revise","request":{"memory_ref":reference,"expected_rev":2,"expected_hash":current["revision"]["semantic_hash"],"changes":{"evidence":"local test fixture"}}}}});
    let reply = ok(root, &["mcp", "serve"], &rpc);
    assert_ne!(reply["result"]["isError"], true, "{reply}");
    assert_eq!(
        reply["result"]["structuredContent"]["revision"]["semantic_rev"],
        3
    );
    let before = history(root, &reference);
    assert_eq!(before["entries"].as_array().unwrap().len(), 3);
    for name in ["rust-index.sqlite", "vectors-rust.sqlite"] {
        let _ = fs::remove_file(root.join(".mnemosyne").join(name));
    }
    assert!(run(root, &["reindex"], &json!({})).status.success());
    assert_eq!(history(root, &reference), before);
    assert_eq!(show(root, &reference)["revision"]["semantic_rev"], 3);
    let original = fs::read_to_string(
        root.join(".mnemosyne/history")
            .join(reference["memory_id"].as_str().unwrap())
            .join("1.md"),
    )
    .unwrap();
    assert!(original.contains("routing port 8080"));
    assert!(!original.contains("9090"));
}
#[test]
fn maintenance_and_relations_record_semantics_without_heat_history() {
    let t = setup();
    let root = t.path();
    let first = write(root, "8080", 90);
    let second = write(root, "9090", 90);
    let a = first["memory_id"].as_str().unwrap();
    let b = second["memory_id"].as_str().unwrap();
    assert!(
        run(root, &["link", a, b, "--rel", "supersedes"], &json!({}))
            .status
            .success()
    );
    assert_eq!(show(root, &first)["revision"]["semantic_rev"], 2);
    assert_eq!(show(root, &second)["memory"]["status"], "superseded");
    assert_eq!(show(root, &second)["revision"]["semantic_rev"], 2);
    let before = history(root, &first);
    assert!(
        run(root, &["maintain", "--scope", "project"], &json!({}))
            .status
            .success()
    );
    assert_eq!(history(root, &first), before);
    let weak = write(root, "weak", 1);
    let id = weak["memory_id"].as_str().unwrap();
    assert!(
        run(root, &["maintain", "--scope", "project"], &json!({}))
            .status
            .success()
    );
    assert!(
        !root
            .join(".mnemosyne/working")
            .join(format!("{id}.md"))
            .exists()
    );
    let archived = show(root, &weak);
    assert_eq!(archived["memory"]["status"], "deprecated");
    assert_eq!(archived["revision"]["semantic_rev"], 2);
    assert!(!root.join(".mnemosyne/.relations-operation.json").exists());
}

#[test]
fn old_v2_store_requires_explicit_writer_upgrade_before_history() {
    let t = setup();
    let root = t.path();
    let path = root.join(".mnemosyne/store.json");
    let mut manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let identity = manifest["store_id"].clone();
    manifest["min_writer_version"] = json!(2);
    fs::write(&path, manifest.to_string()).unwrap();
    let before = fs::read(&path).unwrap();
    assert_eq!(
        ok(root, &["store-upgrade"], &json!({}))["status"],
        "upgrade_available"
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(!root.join(".mnemosyne/history").exists());
    let request = json!({"type":"codebase","title":"old writer","content":"fact","origin":"fixture","source_session_id":"s","source_event_id":"e","finding_key":"f","source_kind":"code_observation","verification_state":"unverified"});
    let blocked = run(root, &["write-v2"], &request);
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("UPGRADE_REQUIRED"));
    let upgraded = ok(root, &["store-upgrade", "--commit"], &json!({}));
    assert_eq!(upgraded["store_id"], identity);
    assert_eq!(upgraded["min_writer_version"], 3);
    let reference = ok(root, &["write-v2"], &request)["memory_ref"].clone();
    assert_eq!(show(root, &reference)["revision"]["semantic_rev"], 1);
}

#[test]
fn archive_is_semantic_even_when_status_does_not_change() {
    let t = setup();
    let root = t.path();
    let reference = write(root, "expiry", 95);
    let shown = show(root, &reference);
    ok(
        root,
        &["revise-v2"],
        &json!({"memory_ref":reference,"expected_rev":1,"expected_hash":shown["revision"]["semantic_hash"],"changes":{"expires":"2000-01-01"}}),
    );
    assert!(
        run(root, &["maintain", "--scope", "project"], &json!({}))
            .status
            .success()
    );
    let archived = show(root, &reference);
    assert_eq!(archived["memory"]["status"], "active");
    assert!(archived["memory"]["extra"]["archived_at"].is_string());
    assert_eq!(archived["revision"]["semantic_rev"], 3);
}

#[test]
fn cross_store_links_reject_mixed_writer_protocols() {
    let t = setup();
    let root = t.path();
    let project = write(root, "project", 70);
    assert!(
        run(
            root,
            &[
                "write",
                "--scope",
                "global",
                "--type",
                "codebase",
                "--title",
                "global",
                "--content",
                "legacy fact",
                "--importance",
                "70"
            ],
            &json!({})
        )
        .status
        .success()
    );
    let files: Vec<_> = fs::read_dir(root.join("global/working"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    let global = files[0].file_stem().unwrap().to_str().unwrap();
    let result = run(
        root,
        &["link", project["memory_id"].as_str().unwrap(), global],
        &json!({}),
    );
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("UPGRADE_REQUIRED"));
    assert!(!root.join("global/.relations-operation.json").exists());
    assert!(!root.join("global/history").exists());
    assert_eq!(show(root, &project)["revision"]["semantic_rev"], 1);
}

#[test]
fn historical_cli_and_mcp_share_read_only_contract() {
    fn files(root: &Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
        fn visit(
            root: &Path,
            dir: &Path,
            out: &mut std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
        ) {
            for entry in fs::read_dir(dir).unwrap() {
                let p = entry.unwrap().path();
                if p.is_dir() {
                    visit(root, &p, out)
                } else {
                    out.insert(
                        p.strip_prefix(root).unwrap().to_path_buf(),
                        fs::read(&p).unwrap(),
                    );
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        visit(root, root, &mut out);
        out
    }
    let t = setup();
    let root = t.path();
    let reference = write(root, "5432", 70);
    let id = reference["memory_id"].as_str().unwrap();
    let before = files(root);
    let cli = ok(
        root,
        &[
            "search",
            "routing",
            "--scope",
            "project",
            "--as-of",
            "2099-01-01",
            "--format",
            "json",
        ],
        &json!({}),
    );
    let mcp = ok(
        root,
        &["mcp", "serve"],
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mnemosyne_search","arguments":{"query":"routing","scope":"project","as_of":"2099-01-01"}}}),
    );
    assert_eq!(cli, mcp["result"]["structuredContent"]);
    assert_eq!(cli["items"].as_array().unwrap().len(), 1);
    let image = ok(root, &["show", id, "--revision", "1"], &json!({}));
    let history = ok(root, &["history", id], &json!({}));
    assert_eq!(history["entries"][0]["revision"], 1);
    let remote = ok(
        root,
        &["mcp", "serve"],
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mnemosyne_show","arguments":{"version":2,"id":id,"revision":1}}}),
    );
    assert_eq!(image, remote["result"]["structuredContent"]);
    assert_eq!(files(root), before);
    let journal = root.join(".mnemosyne/.relations-operation.json");
    fs::write(&journal, "{\"version\":3}").unwrap();
    let blocked = run(root, &["history", id], &json!({}));
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("PENDING_RECOVERY"));
    assert_eq!(fs::read_to_string(&journal).unwrap(), "{\"version\":3}");
    #[cfg(unix)]
    {
        fs::remove_file(&journal).unwrap();
        std::os::unix::fs::symlink("missing-journal", &journal).unwrap();
        let blocked = run(root, &["history", id], &json!({}));
        assert!(!blocked.status.success());
        assert!(String::from_utf8_lossy(&blocked.stderr).contains("Symlink"));
    }
}
