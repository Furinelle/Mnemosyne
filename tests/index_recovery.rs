use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

use mnemosyne::{
    schema::{Memory, serialize_memory},
    store::{Store, ensure_store},
};
use rusqlite::Connection;
use serde_json::Value;

fn cli(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mnemosyne"));
    command
        .env_clear()
        .env("MNEMOSYNE_HOME", root.join("global"))
        .current_dir(root)
        .args(args);
    command
}

fn command(root: &Path, args: &[&str]) -> Output {
    cli(root, args).output().unwrap()
}

fn fixture(upgraded: bool) -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join(".git")).unwrap();
    let store = Store {
        scope: "project".into(),
        root: temp.path().join(".mnemosyne"),
    };
    ensure_store(&store).unwrap();
    write(&store, "record", "recoveryneedle old evidence");
    if upgraded {
        let output = command(
            temp.path(),
            &["store-upgrade", "--scope", "project", "--commit"],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(search(temp.path())[0]["id"], "record");
    (temp, store)
}

fn write(store: &Store, id: &str, body: &str) {
    let memory = Memory {
        id: id.into(),
        memory_type: "codebase".into(),
        body: body.into(),
        canonical_summary: body.into(),
        injection_summary: body.into(),
        ..Default::default()
    };
    fs::write(
        store.working_dir().join(format!("{id}.md")),
        serialize_memory(&memory),
    )
    .unwrap();
}

fn search(root: &Path) -> Value {
    let output = command(
        root,
        &[
            "search",
            "recoveryneedle",
            "--scope",
            "project",
            "--format",
            "json",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn indexed(store: &Store) -> Vec<String> {
    let connection = Connection::open(store.root.join("rust-index.sqlite")).unwrap();
    connection
        .prepare("SELECT memory_json FROM memories ORDER BY path")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn cli_keeps_failed_input_evidence_and_converges_after_repair_in_legacy_and_v2_stores() {
    for upgraded in [false, true] {
        let (temp, store) = fixture(upgraded);
        let path = store.working_dir().join("record.md");
        let before = indexed(&store);
        for bad_input in [
            b"".as_slice(),
            b"---\nid: record\n".as_slice(),
            b"---\nid: record\ninjection_summary: partial summary\n---\n".as_slice(),
            b"\xff".as_slice(),
        ] {
            fs::write(&path, bad_input).unwrap();
            let output = command(
                temp.path(),
                &[
                    "search",
                    "recoveryneedle",
                    "--scope",
                    "project",
                    "--format",
                    "json",
                ],
            );
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let result: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(result[0]["id"], "record", "{result}");
            assert_eq!(result[0]["index_status"], "stale");
            for field in [
                "summary",
                "title",
                "evidence",
                "tags",
                "links",
                "related_paths",
            ] {
                assert!(
                    result[0].get(field).is_none(),
                    "stale knowledge field {field} was exposed"
                );
            }
            assert!(result[0]["score_breakdown"]["index_source_warning"].is_string());
            assert!(
                result[0]["warnings"][0]
                    .as_str()
                    .unwrap()
                    .contains("must not be cited")
            );
            assert!(String::from_utf8_lossy(&output.stderr).contains("index source unavailable"));
            assert_eq!(indexed(&store), before);
            assert_eq!(
                fs::read(&path).unwrap(),
                bad_input,
                "search must not write cached evidence over failed canonical input"
            );
        }
        let output = command(
            temp.path(),
            &["prep", "recoveryneedle investigation", "--format", "json"],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let bundle: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(bundle["selected"].as_array().unwrap().is_empty());
        assert_eq!(bundle["omitted"][0]["reason"], "index_source_unavailable");
        assert!(!bundle["context"].as_str().unwrap().contains("old evidence"));
        let mut child = cli(
            temp.path(),
            &["inject", "--event", "turn_start", "--format", "json"],
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(br#"{"prompt":"recoveryneedle investigation"}"#)
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let injected: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(injected["memory_ids"], serde_json::json!([]));
        assert_eq!(
            injected["context_bundle"]["omitted"][0]["reason"],
            "index_source_unavailable"
        );
        assert!(
            !injected["context"]
                .as_str()
                .unwrap()
                .contains("old evidence")
        );
        write(
            &store,
            "record",
            "recoveryneedle repaired complete evidence",
        );
        let result = search(temp.path());
        assert_eq!(
            result[0]["summary"],
            "recoveryneedle repaired complete evidence"
        );
        assert!(
            result[0]["score_breakdown"]
                .get("index_source_warning")
                .is_none()
        );
        fs::remove_file(path).unwrap();
        assert_eq!(search(temp.path()), serde_json::json!([]));
        assert!(indexed(&store).is_empty());
    }
}

#[test]
fn cli_reindex_rolls_back_all_rows_on_failed_input_then_retries() {
    let (temp, store) = fixture(false);
    let before = indexed(&store);
    write(&store, "new", "recoveryneedle newly added evidence");
    fs::write(store.working_dir().join("record.md"), b"\xff").unwrap();
    let output = command(temp.path(), &["reindex", "--scope", "project"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("existing cache preserved"));
    assert_eq!(indexed(&store), before);
    write(&store, "record", "recoveryneedle recovered record");
    let output = command(temp.path(), &["reindex", "--scope", "project"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(indexed(&store).len(), 2);
    assert_eq!(search(temp.path()).as_array().unwrap().len(), 2);
}

#[test]
fn cli_failed_input_cannot_resurrect_expired_superseded_or_redacted_evidence() {
    for retired in ["expired", "superseded", "redacted"] {
        let (temp, store) = fixture(false);
        let path = store.working_dir().join("record.md");
        let mut memory =
            mnemosyne::schema::parse_memory(&fs::read_to_string(&path).unwrap()).unwrap();
        match retired {
            "expired" => memory.expires = "2000-01-01".into(),
            "superseded" => memory.status = "superseded".into(),
            _ => {
                memory.body = "redacted".into();
                memory.canonical_summary = "redacted".into();
                memory.injection_summary = "redacted".into();
            }
        }
        fs::write(&path, serialize_memory(&memory)).unwrap();
        assert_eq!(search(temp.path()), serde_json::json!([]));
        fs::write(path, "").unwrap();
        assert_eq!(search(temp.path()), serde_json::json!([]));
        if retired == "redacted" {
            assert!(!indexed(&store).join(" ").contains("old evidence"));
        }
    }
}

#[test]
fn cli_and_prep_prioritize_fresh_evidence_over_high_scoring_stale_diagnostics() {
    let (temp, store) = fixture(false);
    write(
        &store,
        "fresh",
        &format!("recoveryneedle currentfresh {}", "padding ".repeat(400)),
    );
    assert!(
        command(temp.path(), &["reindex", "--scope", "project"])
            .status
            .success()
    );
    assert_eq!(
        search(temp.path())[0]["id"],
        "record",
        "the soon-stale record must initially outrank the fresh record"
    );
    fs::write(store.working_dir().join("record.md"), "").unwrap();
    let output = command(
        temp.path(),
        &[
            "search",
            "recoveryneedle",
            "--scope",
            "project",
            "--format",
            "json",
            "--limit",
            "1",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result.as_array().unwrap().len(), 1);
    assert_eq!(result[0]["id"], "fresh");
    assert!(result[0].get("index_status").is_none());
    let output = command(
        temp.path(),
        &[
            "prep",
            "recoveryneedle investigation",
            "--format",
            "json",
            "--limit",
            "1",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bundle: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(bundle["selected"][0]["id"], "fresh");
    assert!(bundle["context"].as_str().unwrap().contains("currentfresh"));
    assert!(!bundle["context"].as_str().unwrap().contains("old evidence"));
}

#[cfg(unix)]
#[test]
fn cli_checks_read_access_even_when_cached_fingerprint_is_unchanged() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, store) = fixture(false);
    let path = store.working_dir().join("record.md");
    let mut memory = mnemosyne::schema::parse_memory(&fs::read_to_string(&path).unwrap()).unwrap();
    memory.links.push(mnemosyne::schema::Link {
        id: "linked".into(),
        rel: "related".into(),
    });
    fs::write(&path, serialize_memory(&memory)).unwrap();
    write(&store, "linked", "unrelated linked evidence");
    assert!(
        command(temp.path(), &["reindex", "--scope", "project"])
            .status
            .success()
    );
    let before = indexed(&store);
    let permissions = fs::metadata(&path).unwrap().permissions();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o0)).unwrap();
    let denied = fs::read_to_string(&path).is_err();
    let output = command(
        temp.path(),
        &[
            "search",
            "recoveryneedle",
            "--scope",
            "project",
            "--format",
            "json",
        ],
    );
    fs::set_permissions(&path, permissions).unwrap();
    if denied {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            result.as_array().unwrap().len(),
            1,
            "unreadable cached source must not expand links"
        );
        assert_eq!(result[0]["index_status"], "stale");
        assert!(result[0].get("summary").is_none());
        assert!(String::from_utf8_lossy(&output.stderr).contains("source_unreadable"));
        assert_eq!(indexed(&store), before);
    }
    assert_eq!(
        search(temp.path())[0]["summary"],
        "recoveryneedle old evidence"
    );
}

#[cfg(unix)]
#[test]
fn cli_directory_read_error_cannot_commit_missing_files_as_deletions() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, store) = fixture(false);
    let before = indexed(&store);
    let working = store.working_dir();
    let permissions = fs::metadata(&working).unwrap().permissions();
    fs::set_permissions(&working, fs::Permissions::from_mode(0o0)).unwrap();
    let denied = fs::read_dir(&working).is_err();
    let output = command(
        temp.path(),
        &[
            "search",
            "recoveryneedle",
            "--scope",
            "project",
            "--format",
            "json",
        ],
    );
    fs::set_permissions(&working, permissions).unwrap();
    if denied {
        assert!(!output.status.success());
        assert!(!output.stderr.is_empty());
        assert_eq!(indexed(&store), before);
    }
    assert_eq!(search(temp.path())[0]["id"], "record");
}
