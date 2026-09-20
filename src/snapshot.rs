//! Portable, bounded directory snapshots. Markdown remains the source of truth.
use crate::{
    provenance::{StoreManifest, read_manifest},
    store::{Store, lock_store},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_FILES: usize = 10_000;
const MAX_FILE: u64 = 64 * 1024 * 1024;
const MAX_TOTAL: u64 = 1024 * 1024 * 1024;
const MAX_MANIFEST: u64 = 2 * 1024 * 1024;
const ALLOWED: &[&str] = &[
    "core.md",
    "store.json",
    "working",
    "archive",
    "history",
    "evidence",
    "checkpoints",
    "proposals",
];
const EXCLUDED: &[&str] = &[
    "config.toml (host settings and API selectors)",
    "raw transcripts and logs",
    "model files",
    "SQLite databases and WAL",
    "locks and journals",
    "per-memory *.md.lock files",
    "unknown paths",
    "files with detected credential material (snapshot refused)",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotManifest {
    pub format_version: u32,
    pub software_version: String,
    pub store_schema_version: u32,
    pub store_id: String,
    pub files: Vec<FileEntry>,
    pub excluded: Vec<String>,
}

fn safe_relative(value: &str) -> Result<&Path> {
    ensure!(
        !value.is_empty() && !value.contains('\\'),
        "invalid snapshot path"
    );
    let path = Path::new(value);
    ensure!(
        path.components().all(|c| matches!(c, Component::Normal(_)))
            && path.to_str() == Some(value)
            && !value
                .split('/')
                .any(|part| part == "." || part == ".." || part.is_empty()),
        "invalid snapshot path: {value}"
    );
    let parts: Vec<_> = path.components().map(|part| part.as_os_str()).collect();
    let top = parts[0];
    ensure!(
        ALLOWED.iter().any(|allowed| top == *allowed),
        "snapshot path not allowed: {value}"
    );
    if top == "core.md" || top == "store.json" {
        ensure!(parts.len() == 1, "metadata path must be a file");
        return Ok(path);
    }
    let valid = if top == "working" {
        parts.len() == 2 && value.ends_with(".md")
    } else if top == "archive" {
        parts.len() == 3 && value.ends_with(".md")
    } else if top == "history" {
        parts.len() == 3 && (value.ends_with(".md") || parts[2] == "manifest.json")
    } else {
        parts.len() == 2 && value.ends_with(".json")
    };
    ensure!(valid, "snapshot path not allowed: {value}");
    Ok(path)
}

fn regular(path: &Path) -> Result<fs::Metadata> {
    let meta =
        fs::symlink_metadata(path).with_context(|| format!("cannot inspect {}", path.display()))?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "non-regular snapshot file: {}",
        path.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            meta.nlink() == 1,
            "hardlinked snapshot file: {}",
            path.display()
        );
    }
    ensure!(meta.len() <= MAX_FILE, "snapshot file too large");
    Ok(meta)
}

fn directory(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir() && !meta.file_type().is_symlink(),
        "non-directory snapshot path: {}",
        path.display()
    );
    Ok(())
}

fn destination(path: &Path) -> Result<&Path> {
    match fs::symlink_metadata(path) {
        Ok(_) => bail!("snapshot target must not exist"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    directory(parent)?;
    Ok(parent)
}

fn publish(stage: &Path, target: &Path) -> Result<()> {
    // The last destination check alone cannot prevent another process from creating it.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::{
            ffi::{CString, c_char, c_int, c_uint},
            os::unix::ffi::OsStrExt,
        };
        let from = CString::new(stage.as_os_str().as_bytes())?;
        let to = CString::new(target.as_os_str().as_bytes())?;
        #[cfg(target_os = "macos")]
        let rc = {
            unsafe extern "C" {
                fn renamex_np(from: *const c_char, to: *const c_char, flags: c_uint) -> c_int;
            }
            unsafe { renamex_np(from.as_ptr(), to.as_ptr(), 0x0000_0004) } // RENAME_EXCL
        };
        #[cfg(target_os = "linux")]
        let rc = {
            unsafe extern "C" {
                fn renameat2(
                    fromfd: c_int,
                    from: *const c_char,
                    tofd: c_int,
                    to: *const c_char,
                    flags: c_uint,
                ) -> c_int;
            }
            unsafe { renameat2(-100, from.as_ptr(), -100, to.as_ptr(), 1) } // AT_FDCWD, RENAME_NOREPLACE
        };
        ensure!(
            rc == 0,
            "snapshot target changed: {}",
            std::io::Error::last_os_error()
        );
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        destination(target)?;
        fs::rename(stage, target)?;
    }
    Ok(())
}

fn gather(root: &Path, relative: &Path, out: &mut Vec<PathBuf>, skip_locks: bool) -> Result<()> {
    let path = root.join(relative);
    let meta = fs::symlink_metadata(&path)?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        for item in fs::read_dir(&path)? {
            let item = item?;
            gather(root, &relative.join(item.file_name()), out, skip_locks)?;
        }
    } else {
        let rel = relative.to_str().context("non-UTF8 snapshot path")?;
        if skip_locks
            && (rel.starts_with("working/") || rel.starts_with("archive/"))
            && rel.ends_with(".md.lock")
        {
            regular(&path)?;
            return Ok(());
        }
        safe_relative(rel)?;
        regular(&path)?;
        ensure!(out.len() < MAX_FILES, "too many snapshot files");
        out.push(relative.to_path_buf());
    }
    Ok(())
}

fn copy_hash(source: &Path, target: &Path, expected: u64) -> Result<String> {
    let mut input = File::open(source)?;
    let mut output = File::create_new(target)?;
    let mut hash = Sha256::new();
    let mut copied = 0_u64;
    let mut buf = [0_u8; 65536];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        copied += n as u64;
        ensure!(
            copied <= expected && copied <= MAX_FILE,
            "snapshot file changed during copy"
        );
        hash.update(&buf[..n]);
        output.write_all(&buf[..n])?;
    }
    ensure!(copied == expected, "snapshot file changed during copy");
    output.sync_all()?;
    Ok(format!("{:x}", hash.finalize()))
}

fn reject_credentials(path: &Path) -> Result<()> {
    let content = String::from_utf8(fs::read(path)?).context("snapshot text is not UTF-8")?;
    if path.extension().is_some_and(|ext| ext == "json") {
        fn has_secret_key(value: &Value) -> bool {
            match value {
                Value::Object(map) => map.iter().any(|(key, child)| {
                    [
                        "api_key",
                        "access_token",
                        "client_secret",
                        "password",
                        "authorization",
                    ]
                    .contains(&key.to_ascii_lowercase().as_str())
                        || has_secret_key(child)
                }),
                Value::Array(items) => items.iter().any(has_secret_key),
                _ => false,
            }
        }
        let value: Value = serde_json::from_str(&content)?;
        ensure!(
            !has_secret_key(&value),
            "possible credential in {}",
            path.display()
        );
    }
    for line in content.lines() {
        let line = line.trim().to_ascii_lowercase();
        ensure!(
            !(line.contains("authorization: bearer ")
                || (line.contains("-----begin") && line.contains("private key"))),
            "possible credential in {}",
            path.display()
        );
        for key in ["api_key", "access_token", "client_secret", "password"] {
            if let Some(rest) = line.strip_prefix(key) {
                let rest = rest.trim_start();
                ensure!(
                    !rest.starts_with('=') && !rest.starts_with(':'),
                    "possible credential in {}",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

fn read_manifest_file(path: &Path) -> Result<SnapshotManifest> {
    ensure!(regular(path)?.len() <= MAX_MANIFEST, "manifest too large");
    let manifest: SnapshotManifest = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(manifest.format_version == 1, "unsupported snapshot format");
    ensure!(manifest.files.len() <= MAX_FILES, "too many snapshot files");
    uuid::Uuid::parse_str(&manifest.store_id)?;
    let mut seen = HashSet::new();
    let mut total = 0_u64;
    for entry in &manifest.files {
        safe_relative(&entry.path)?;
        ensure!(seen.insert(entry.path.clone()), "duplicate snapshot path");
        ensure!(entry.size <= MAX_FILE, "snapshot file too large");
        total = total
            .checked_add(entry.size)
            .context("snapshot size overflow")?;
        ensure!(total <= MAX_TOTAL, "snapshot too large");
        ensure!(
            entry.sha256.len() == 64 && entry.sha256.bytes().all(|c| c.is_ascii_hexdigit()),
            "invalid SHA-256"
        );
    }
    ensure!(seen.contains("store.json"), "missing store manifest");
    Ok(manifest)
}

fn validate(source: &Path) -> Result<SnapshotManifest> {
    directory(source)?;
    directory(&source.join("files"))?;
    let manifest = read_manifest_file(&source.join("manifest.json"))?;
    let mut top_entries = HashSet::new();
    for entry in fs::read_dir(source)? {
        top_entries.insert(entry?.file_name());
    }
    ensure!(
        top_entries.len() == 2
            && top_entries.contains(std::ffi::OsStr::new("files"))
            && top_entries.contains(std::ffi::OsStr::new("manifest.json")),
        "unexpected snapshot content"
    );
    let mut disk = Vec::new();
    for top in fs::read_dir(source.join("files"))? {
        let top = top?;
        let name = top
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("non-UTF8 path"))?;
        ensure!(
            ALLOWED.contains(&name.as_str()),
            "unexpected snapshot content"
        );
        gather(&source.join("files"), Path::new(&name), &mut disk, false)?;
    }
    let listed: HashSet<_> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    ensure!(disk.len() == listed.len(), "snapshot file count mismatch");
    for rel in disk {
        let name = rel.to_str().context("non-UTF8 path")?;
        ensure!(listed.contains(name), "unlisted snapshot file");
    }
    for entry in &manifest.files {
        let path = source.join("files").join(&entry.path);
        ensure!(
            regular(&path)?.len() == entry.size,
            "snapshot size mismatch"
        );
        let mut file = File::open(path)?;
        let mut hash = Sha256::new();
        let mut buf = [0_u8; 65536];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
        }
        ensure!(
            format!("{:x}", hash.finalize()) == entry.sha256,
            "snapshot hash mismatch: {}",
            entry.path
        );
        reject_credentials(&source.join("files").join(&entry.path))?;
        if entry.path.starts_with("working/") || entry.path.starts_with("archive/") {
            let memory = crate::schema::parse_memory(&fs::read_to_string(
                source.join("files").join(&entry.path),
            )?)?;
            ensure!(
                Path::new(&entry.path)
                    .file_stem()
                    .and_then(|name| name.to_str())
                    == Some(memory.id.as_str()),
                "memory path and ID differ"
            );
        }
        if entry.path.starts_with("checkpoints/") {
            let checkpoint: crate::checkpoint::Checkpoint =
                serde_json::from_slice(&fs::read(source.join("files").join(&entry.path))?)?;
            ensure!(
                checkpoint.schema_version == 1
                    && checkpoint.store_id == manifest.store_id
                    && Path::new(&entry.path)
                        .file_stem()
                        .and_then(|name| name.to_str())
                        == Some(checkpoint.id.as_str()),
                "invalid checkpoint in snapshot"
            );
        }
        if entry.path.starts_with("proposals/") {
            let proposal: crate::proposals::Proposal =
                serde_json::from_slice(&fs::read(source.join("files").join(&entry.path))?)?;
            let request_hash = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&proposal.request)?)
            );
            ensure!(
                proposal.version == 1
                    && proposal.store_id == manifest.store_id
                    && proposal.id == request_hash
                    && proposal.summary_hash == request_hash
                    && entry.path == format!("proposals/{request_hash}.json"),
                "invalid proposal in snapshot"
            );
        }
    }
    let identity: StoreManifest =
        serde_json::from_slice(&fs::read(source.join("files/store.json"))?)?;
    ensure!(
        identity.schema_version == manifest.store_schema_version
            && identity.store_id == manifest.store_id,
        "snapshot identity mismatch"
    );
    ensure!(
        identity.schema_version == 2
            && identity.min_writer_version <= crate::provenance::WRITER_VERSION,
        "incompatible store schema"
    );
    let history_dir = source.join("files/history");
    if history_dir.exists() {
        let store = Store {
            scope: "project".into(),
            root: source.join("files"),
        };
        for item in fs::read_dir(history_dir)? {
            let item = item?;
            directory(&item.path())?;
            let id = item
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 history ID"))?;
            ensure!(
                crate::revisions::read_history(&store, &id)?.1.is_some(),
                "missing history manifest"
            );
        }
    }
    Ok(manifest)
}

/// Create a portable directory snapshot at a new path.
pub fn create(store: &Store, destination_path: &Path) -> Result<SnapshotManifest> {
    let parent = destination(destination_path)?;
    if let (Ok(store_root), Ok(target_parent)) = (store.root.canonicalize(), parent.canonicalize())
    {
        ensure!(
            !target_parent.starts_with(store_root),
            "snapshot target cannot be inside source store"
        );
    }
    let _lock = lock_store(store)?;
    let identity = read_manifest(store)?.context("snapshot requires an upgraded store")?;
    ensure!(
        identity.schema_version == 2
            && identity.min_writer_version <= crate::provenance::WRITER_VERSION,
        "incompatible store schema"
    );
    for pending in [".relations-operation.json", ".relations-mutation.json"] {
        ensure!(
            !store.root.join(pending).exists(),
            "pending cross-store operation; snapshot refused"
        );
    }
    let commits = store.root.join(".relations-commits");
    if commits.exists() {
        directory(&commits)?;
        ensure!(
            fs::read_dir(commits)?.next().is_none(),
            "pending cross-store decision; snapshot refused"
        );
    }
    let stage = tempfile::Builder::new()
        .prefix(".mnemosyne-snapshot-")
        .tempdir_in(parent)?;
    let files = stage.path().join("files");
    fs::create_dir(&files)?;
    let mut paths = Vec::new();
    for top in ALLOWED {
        let path = store.root.join(top);
        if path.exists() || fs::symlink_metadata(&path).is_ok() {
            gather(&store.root, Path::new(top), &mut paths, true)?;
        }
    }
    paths.sort();
    let mut entries = Vec::new();
    let mut total = 0_u64;
    for rel in paths {
        let src = store.root.join(&rel);
        let meta = regular(&src)?;
        reject_credentials(&src)?;
        total = total
            .checked_add(meta.len())
            .context("snapshot size overflow")?;
        ensure!(total <= MAX_TOTAL, "snapshot too large");
        let dest = files.join(&rel);
        fs::create_dir_all(dest.parent().unwrap())?;
        let sha256 = copy_hash(&src, &dest, meta.len())?;
        entries.push(FileEntry {
            path: rel.to_str().unwrap().to_string(),
            size: meta.len(),
            sha256,
        });
    }
    let manifest = SnapshotManifest {
        format_version: 1,
        software_version: env!("CARGO_PKG_VERSION").to_string(),
        store_schema_version: identity.schema_version,
        store_id: identity.store_id,
        files: entries,
        excluded: EXCLUDED.iter().map(|s| (*s).to_string()).collect(),
    };
    let mut output = File::create_new(stage.path().join("manifest.json"))?;
    serde_json::to_writer_pretty(&mut output, &manifest)?;
    output.write_all(b"\n")?;
    output.sync_all()?;
    validate(stage.path())?;
    destination(destination_path)?;
    publish(stage.path(), destination_path)?;
    File::open(parent)?.sync_all()?;
    Ok(manifest)
}

fn remap_json(path: &Path, old: &str, new: &str) -> Result<()> {
    let mut value: Value = serde_json::from_slice(&fs::read(path)?)?;
    fn visit(value: &mut Value, old: &str, new: &str) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    if key == "store_id" && child.as_str() == Some(old) {
                        *child = Value::String(new.to_string());
                    } else {
                        visit(child, old, new);
                    }
                }
            }
            Value::Array(items) => {
                for child in items {
                    visit(child, old, new);
                }
            }
            _ => (),
        }
    }
    visit(&mut value, old, new);
    let mut file = File::create(path)?;
    serde_json::to_writer_pretty(&mut file, &value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

/// Restore into a missing target. `fork` creates a new independent store ID.
pub fn restore(source: &Path, target: &Path, fork: bool) -> Result<SnapshotManifest> {
    let parent = destination(target)?;
    let mut manifest = validate(source)?;
    let stage = tempfile::Builder::new()
        .prefix(".mnemosyne-restore-")
        .tempdir_in(parent)?;
    for entry in &manifest.files {
        let rel = safe_relative(&entry.path)?;
        let dest = stage.path().join(rel);
        fs::create_dir_all(dest.parent().unwrap())?;
        let sha = copy_hash(&source.join("files").join(rel), &dest, entry.size)?;
        ensure!(sha == entry.sha256, "snapshot changed during restore");
    }
    if fork {
        let new_id = uuid::Uuid::new_v4().to_string();
        for entry in &mut manifest.files {
            if entry.path.starts_with("proposals/") {
                let old_path = stage.path().join(&entry.path);
                let mut proposal: crate::proposals::Proposal =
                    serde_json::from_slice(&fs::read(&old_path)?)?;
                for target in &mut proposal.request.targets {
                    ensure!(
                        target.memory_ref.store_id == manifest.store_id,
                        "fork cannot remap an external proposal target"
                    );
                    target.memory_ref.store_id = new_id.clone();
                }
                proposal.store_id = new_id.clone();
                let hash = format!(
                    "{:x}",
                    Sha256::digest(serde_json::to_vec(&proposal.request)?)
                );
                proposal.fork_origin_summary_hash = Some(proposal.summary_hash.clone());
                proposal.approved_at = None;
                proposal.approved_summary_hash = None;
                if proposal.state == "pending" {
                    proposal.state = "stale".into();
                }
                proposal.id = hash.clone();
                proposal.summary_hash = hash.clone();
                let new_path = stage.path().join("proposals").join(format!("{hash}.json"));
                let mut file = File::create_new(&new_path)?;
                serde_json::to_writer_pretty(&mut file, &proposal)?;
                file.write_all(b"\n")?;
                file.sync_all()?;
                fs::remove_file(old_path)?;
                entry.path = format!("proposals/{hash}.json");
                let bytes = fs::read(new_path)?;
                entry.size = bytes.len() as u64;
                entry.sha256 = format!("{:x}", Sha256::digest(&bytes));
            } else if entry.path == "store.json" || entry.path.starts_with("checkpoints/") {
                let path = stage.path().join(&entry.path);
                remap_json(&path, &manifest.store_id, &new_id)?;
                let bytes = fs::read(&path)?;
                entry.size = bytes.len() as u64;
                entry.sha256 = format!("{:x}", Sha256::digest(&bytes));
            }
        }
        manifest.store_id = new_id;
    }
    let restored = Store {
        scope: "project".into(),
        root: stage.path().to_path_buf(),
    };
    let identity = read_manifest(&restored)?.context("restored store identity missing")?;
    ensure!(
        identity.schema_version == manifest.store_schema_version,
        "restored schema mismatch"
    );
    // Rebuild the file-only client directory before publishing the private stage.
    crate::api::update_markdown_index(&restored, None)?;
    destination(target)?;
    publish(stage.path(), target)?;
    File::open(parent)?.sync_all()?;
    Ok(manifest)
}
