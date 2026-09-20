//! Conservative applicability checks for code-scoped memories.
use crate::{schema::Memory, store::Store};
use anyhow::Result;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

const MAX_PATHS: usize = 64;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_GIT_OUTPUT: u64 = 64 * 1024;

struct Repo {
    root: PathBuf,
}

enum Worktree {
    Clean,
    Dirty,
    LocalFilter,
    Gitlink,
}

fn text(memory: &Memory, key: &str) -> Option<String> {
    memory.extra.get(key)?.as_str().map(str::to_owned)
}

fn result(memory: &Memory, status: &str, reason: &str) -> Value {
    json!({
        "status": status,
        "reason": reason,
        "applies_to": text(memory, "applies_to"),
        "observed_commit": text(memory, "observed_commit"),
        "branch": text(memory, "branch"),
    })
}

fn git_result(root: &Path, args: &[&str]) -> Option<(Option<i32>, Vec<u8>)> {
    let path = std::env::var_os("PATH")?;
    let mut child = Command::new("git")
        .env_clear()
        .env("PATH", path)
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-c")
        .arg("core.untrackedCache=false")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut output = Vec::new();
    child
        .stdout
        .take()?
        .take(MAX_GIT_OUTPUT + 1)
        .read_to_end(&mut output)
        .ok()?;
    if output.len() as u64 > MAX_GIT_OUTPUT {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    Some((child.wait().ok()?.code(), output))
}

fn git(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let (status, output) = git_result(root, args)?;
    (status == Some(0)).then_some(output)
}

fn repo(store: &Store) -> Option<Repo> {
    (store.scope == "project").then_some(())?;
    let project = store.root.parent()?.canonicalize().ok()?;
    let root = String::from_utf8(git(&project, &["rev-parse", "--show-toplevel"])?).ok()?;
    Some(Repo {
        root: Path::new(root.trim()).canonicalize().ok()?,
    })
}

fn revision(repo: &Repo, value: &str) -> Option<String> {
    String::from_utf8(git(&repo.root, &["rev-parse", "--verify", value])?)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn local_filters(repo: &Repo) -> Option<bool> {
    let (status, output) = git_result(
        &repo.root,
        &[
            "config",
            "--includes",
            "--name-only",
            "--get-regexp",
            r"^filter\..*\.(clean|process)$",
        ],
    )?;
    match status {
        Some(0) => Some(!output.is_empty()),
        Some(1) => Some(false),
        _ => None,
    }
}

fn has_gitlink(repo: &Repo) -> Option<bool> {
    Some(
        git(&repo.root, &["ls-files", "--stage", "-z"])?
            .split(|byte| *byte == b'\0')
            .any(|line| line.starts_with(b"160000 ")),
    )
}

fn clean(repo: &Repo) -> Option<Worktree> {
    if local_filters(repo)? {
        return Some(Worktree::LocalFilter);
    }
    if has_gitlink(repo)? {
        return Some(Worktree::Gitlink);
    }
    let status = git(
        &repo.root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--",
            ".",
            ":(exclude).mnemosyne",
        ],
    )?;
    Some(if status.is_empty() {
        Worktree::Clean
    } else {
        Worktree::Dirty
    })
}

fn relative(value: &str) -> Option<&Path> {
    let path = Path::new(value);
    (!value.is_empty()
        && !value.contains('\\')
        && !path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
        && !path.components().any(|part| {
            matches!(part, Component::Normal(name) if name == ".git" || name == ".mnemosyne")
        }))
    .then_some(path)
}

fn digest_paths(memory: &Memory, repo: &Repo) -> Option<Vec<(String, String)>> {
    let paths = memory.extra.get("related_paths")?.as_array()?;
    let hashes = memory.extra.get("related_path_hashes")?.as_array()?;
    if paths.is_empty() || paths.len() != hashes.len() || paths.len() > MAX_PATHS {
        return None;
    }
    let mut total = 0_u64;
    let mut values = Vec::with_capacity(paths.len());
    let mut seen = HashSet::new();
    for (path, expected) in paths.iter().zip(hashes) {
        let path = relative(path.as_str()?)?;
        if !seen.insert(path) {
            return None;
        }
        let expected = expected.as_str()?;
        if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let mut full = repo.root.clone();
        for part in path.components() {
            full.push(part);
            if fs::symlink_metadata(&full).ok()?.file_type().is_symlink() {
                return None;
            }
        }
        let meta = fs::metadata(&full).ok()?;
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
            return None;
        }
        let mut data = Vec::with_capacity(meta.len() as usize);
        fs::File::open(full)
            .ok()?
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut data)
            .ok()?;
        let size = u64::try_from(data.len()).ok()?;
        if size > MAX_FILE_BYTES || total.saturating_add(size) > MAX_TOTAL_BYTES {
            return None;
        }
        total += size;
        values.push((
            path.to_string_lossy().into_owned(),
            format!("{:x}", Sha256::digest(data)),
        ));
    }
    Some(values)
}

/// Evaluate only explicit applicability evidence. `observed_commit` and `branch`
/// are preserved for callers but deliberately do not affect the result.
pub fn evaluate(store: &Store, memory: &Memory) -> Result<Value> {
    let Some(applies_to) = text(memory, "applies_to") else {
        return Ok(result(memory, "unknown", "missing_applies_to"));
    };
    let Some(repo) = repo(store) else {
        return Ok(result(memory, "unknown", "project_repository_unavailable"));
    };
    match applies_to.as_str() {
        "repo-wide" => Ok(result(memory, "applicable", "project_store")),
        "commit" | "tree" => {
            let Some(expected) = text(
                memory,
                if applies_to == "commit" {
                    "applies_commit"
                } else {
                    "applies_tree"
                },
            ) else {
                return Ok(result(memory, "unknown", "missing_applicability_evidence"));
            };
            if !matches!(expected.len(), 40 | 64)
                || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Ok(result(
                    memory,
                    "unknown",
                    "malformed_applicability_evidence",
                ));
            }
            let Some(worktree) = clean(&repo) else {
                return Ok(result(memory, "unknown", "worktree_state_unavailable"));
            };
            match worktree {
                Worktree::Clean => (),
                Worktree::Dirty => return Ok(result(memory, "unknown", "dirty_worktree")),
                Worktree::LocalFilter => {
                    return Ok(result(memory, "unknown", "local_filter_configured"));
                }
                Worktree::Gitlink => return Ok(result(memory, "unknown", "gitlink_present")),
            }
            let target = if applies_to == "commit" {
                "HEAD"
            } else {
                "HEAD^{tree}"
            };
            let Some(current) = revision(&repo, target) else {
                return Ok(result(memory, "unknown", "repository_revision_unavailable"));
            };
            Ok(result(
                memory,
                if current.eq_ignore_ascii_case(&expected) {
                    "applicable"
                } else {
                    "not_applicable"
                },
                if current.eq_ignore_ascii_case(&expected) {
                    "exact_match"
                } else {
                    "exact_mismatch"
                },
            ))
        }
        "paths" => {
            let Some(actual) = digest_paths(memory, &repo) else {
                return Ok(result(memory, "unknown", "path_evidence_unavailable"));
            };
            let expected = memory.extra["related_path_hashes"].as_array().unwrap();
            let matches = actual.iter().zip(expected).all(|((_, actual), expected)| {
                expected
                    .as_str()
                    .is_some_and(|value| value.eq_ignore_ascii_case(actual))
            });
            Ok(result(
                memory,
                if matches {
                    "applicable"
                } else {
                    "not_applicable"
                },
                if matches {
                    "path_digest_match"
                } else {
                    "path_digest_mismatch"
                },
            ))
        }
        _ => Ok(result(memory, "unknown", "unsupported_applies_to")),
    }
}
