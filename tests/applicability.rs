use mnemosyne::{applicability, schema::Memory, store::Store};
use serde_json::json;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}
fn setup() -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "--quiet"]);
    git(
        temp.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(temp.path(), &["config", "user.name", "Test"]);
    fs::create_dir_all(temp.path().join("src/a")).unwrap();
    fs::create_dir_all(temp.path().join("src/b")).unwrap();
    fs::write(temp.path().join("src/a/mod.rs"), "a1\n").unwrap();
    fs::write(temp.path().join("src/b/mod.rs"), "b1\n").unwrap();
    git(temp.path(), &["add", "."]);
    git(temp.path(), &["commit", "--quiet", "-m", "first"]);
    let store = Store {
        scope: "project".into(),
        root: temp.path().join(".mnemosyne"),
    };
    fs::create_dir(&store.root).unwrap();
    (temp, store)
}
fn memory(applies_to: &str) -> Memory {
    let mut memory = Memory::default();
    memory.extra.insert("applies_to".into(), json!(applies_to));
    memory
        .extra
        .insert("observed_commit".into(), json!("receipt-only"));
    memory.extra.insert("branch".into(), json!("display-only"));
    memory
}
#[test]
fn exact_commit_is_conservative_for_old_and_dirty_worktrees() {
    let (temp, store) = setup();
    let old = git(temp.path(), &["rev-parse", "HEAD"]);
    let mut memory = memory("commit");
    memory.extra.insert("applies_commit".into(), json!(&old));
    assert_eq!(
        applicability::evaluate(&store, &memory).unwrap()["status"],
        "applicable"
    );
    fs::write(temp.path().join("src/a/mod.rs"), "a2\n").unwrap();
    assert_eq!(
        applicability::evaluate(&store, &memory).unwrap()["reason"],
        "dirty_worktree"
    );
    git(temp.path(), &["add", "."]);
    git(temp.path(), &["commit", "--quiet", "-m", "second"]);
    assert_eq!(
        applicability::evaluate(&store, &memory).unwrap()["status"],
        "not_applicable"
    );
}
#[test]
fn tree_requires_cleanliness_and_paths_distinguish_same_basenames() {
    let (temp, store) = setup();
    let tree = git(temp.path(), &["rev-parse", "HEAD^{tree}"]);
    let original_branch = git(temp.path(), &["rev-parse", "--abbrev-ref", "HEAD"]);
    let mut exact = memory("tree");
    exact.extra.insert("applies_tree".into(), json!(tree));
    assert_eq!(
        applicability::evaluate(&store, &exact).unwrap()["status"],
        "applicable"
    );
    fs::write(temp.path().join("untracked.txt"), "dirty\n").unwrap();
    assert_eq!(
        applicability::evaluate(&store, &exact).unwrap()["status"],
        "unknown"
    );
    fs::remove_file(temp.path().join("untracked.txt")).unwrap();
    let mut paths = memory("paths");
    paths.extra.insert(
        "related_paths".into(),
        json!(["src/a/mod.rs", "src/b/mod.rs"]),
    );
    paths.extra.insert(
        "related_path_hashes".into(),
        json!([
            "0111f7554519f7126c570c154b894f1fbcddf4faa126f6d644b974dab6c77411",
            "e10a1287bfc72ab847878fa7737ea038aa327a3920d6c8c28b8e6484e013e913"
        ]),
    );
    assert_eq!(
        applicability::evaluate(&store, &paths).unwrap()["status"],
        "applicable"
    );
    git(
        temp.path(),
        &["checkout", "--quiet", "-b", "same-basename-other-branch"],
    );
    fs::write(temp.path().join("src/b/mod.rs"), "a1\n").unwrap();
    git(temp.path(), &["add", "src/b/mod.rs"]);
    git(
        temp.path(),
        &["commit", "--quiet", "-m", "same basename, other branch"],
    );
    assert_eq!(
        applicability::evaluate(&store, &paths).unwrap()["status"],
        "not_applicable"
    );
    git(temp.path(), &["checkout", "--quiet", &original_branch]);
    assert_eq!(
        applicability::evaluate(&store, &paths).unwrap()["status"],
        "applicable"
    );
}
#[test]
fn global_or_unsafe_evidence_is_unknown() {
    let (_temp, mut store) = setup();
    let mut repo_wide = memory("repo-wide");
    store.scope = "global".into();
    assert_eq!(
        applicability::evaluate(&store, &repo_wide).unwrap()["status"],
        "unknown"
    );
    store.scope = "project".into();
    repo_wide.extra.insert("applies_to".into(), json!("paths"));
    repo_wide
        .extra
        .insert("related_paths".into(), json!(["../secret"]));
    repo_wide
        .extra
        .insert("related_path_hashes".into(), json!(["0".repeat(64)]));
    assert_eq!(
        applicability::evaluate(&store, &repo_wide).unwrap()["status"],
        "unknown"
    );
}

#[test]
fn malformed_exact_evidence_is_unknown() {
    let (temp, store) = setup();
    let mut commit = memory("commit");
    commit.extra.insert("applies_commit".into(), json!("HEAD"));
    assert_eq!(
        applicability::evaluate(&store, &commit).unwrap()["reason"],
        "malformed_applicability_evidence"
    );
    let mut tree = memory("tree");
    tree.extra.insert("applies_tree".into(), json!("deadbeef"));
    assert_eq!(
        applicability::evaluate(&store, &tree).unwrap()["status"],
        "unknown"
    );
    drop(temp);
}

#[cfg(unix)]
#[test]
fn evaluation_does_not_run_repo_fsmonitor_or_change_index() {
    let (temp, store) = setup();
    let commit = git(temp.path(), &["rev-parse", "HEAD"]);
    let script = temp.path().join(".git/fsmonitor-test.sh");
    let sentinel = std::path::PathBuf::from(format!("{}.ran", script.display()));
    fs::write(&script, "#!/bin/sh\n: > \"$0.ran\"\n").unwrap();
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).unwrap();
    git(
        temp.path(),
        &["config", "core.fsmonitor", script.to_str().unwrap()],
    );
    git(temp.path(), &["config", "core.untrackedCache", "true"]);
    git(temp.path(), &["update-index", "--fsmonitor"]);
    git(temp.path(), &["status", "--porcelain"]);
    assert!(sentinel.exists(), "fixture fsmonitor command did not run");
    fs::remove_file(&sentinel).unwrap();
    let index = fs::read(temp.path().join(".git/index")).unwrap();
    let mut memory = memory("commit");
    memory.extra.insert("applies_commit".into(), json!(commit));
    assert_eq!(
        applicability::evaluate(&store, &memory).unwrap()["status"],
        "applicable"
    );
    assert!(!sentinel.exists());
    assert_eq!(fs::read(temp.path().join(".git/index")).unwrap(), index);
}

#[cfg(unix)]
#[test]
fn evaluation_does_not_run_local_clean_filter() {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "--quiet"]);
    git(
        temp.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(temp.path(), &["config", "user.name", "Test"]);
    fs::write(temp.path().join("tracked.txt"), "one\n").unwrap();
    fs::write(
        temp.path().join(".gitattributes"),
        "tracked.txt filter=sentinel\n",
    )
    .unwrap();
    git(temp.path(), &["add", "tracked.txt", ".gitattributes"]);
    git(temp.path(), &["commit", "--quiet", "-m", "initial"]);
    let store = Store {
        scope: "project".into(),
        root: temp.path().join(".mnemosyne"),
    };
    let commit = git(temp.path(), &["rev-parse", "HEAD"]);
    let script = temp.path().join(".git/filter-test.sh");
    let sentinel = std::path::PathBuf::from(format!("{}.ran", script.display()));
    fs::write(&script, "#!/bin/sh\n: > \"$0.ran\"\ncat\n").unwrap();
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).unwrap();
    git(
        temp.path(),
        &["config", "filter.sentinel.clean", script.to_str().unwrap()],
    );
    fs::write(temp.path().join("tracked.txt"), "two\n").unwrap();
    git(temp.path(), &["status", "--porcelain"]);
    assert!(sentinel.exists(), "fixture clean filter did not run");
    fs::remove_file(&sentinel).unwrap();
    let index = fs::read(temp.path().join(".git/index")).unwrap();
    let mut memory = memory("commit");
    memory.extra.insert("applies_commit".into(), json!(commit));
    let result = applicability::evaluate(&store, &memory).unwrap();
    assert_eq!(result["status"], "unknown");
    assert_eq!(result["reason"], "local_filter_configured");
    assert!(!sentinel.exists());
    assert_eq!(fs::read(temp.path().join(".git/index")).unwrap(), index);
    // Effective config includes repository includes and worktree configuration.
    git(temp.path(), &["config", "--unset", "filter.sentinel.clean"]);
    let included = temp.path().join(".git/included-config");
    git(
        temp.path(),
        &[
            "config",
            "--file",
            included.to_str().unwrap(),
            "filter.sentinel.clean",
            script.to_str().unwrap(),
        ],
    );
    git(
        temp.path(),
        &["config", "include.path", included.to_str().unwrap()],
    );
    let result = applicability::evaluate(&store, &memory).unwrap();
    assert_eq!(result["reason"], "local_filter_configured");
    assert!(!sentinel.exists());
    assert_eq!(fs::read(temp.path().join(".git/index")).unwrap(), index);
}

#[test]
fn gitlink_prevents_worktree_status_check() {
    let (temp, store) = setup();
    let commit = git(temp.path(), &["rev-parse", "HEAD"]);
    git(
        temp.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{commit},vendor/submodule"),
        ],
    );
    let mut memory = memory("commit");
    memory.extra.insert("applies_commit".into(), json!(commit));
    assert_eq!(
        applicability::evaluate(&store, &memory).unwrap()["reason"],
        "gitlink_present"
    );
}
