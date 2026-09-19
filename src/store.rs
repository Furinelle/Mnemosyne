use crate::schema::{Memory, parse_memory, serialize_memory};
use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use serde_json::{Value, json};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

const DEFAULT_CONFIG_TOML: &str = r#"[thresholds]
decay_per_run = 1
bonus_access = 5
bonus_write = 10
bonus_recall = 20
core_strength = 80
core_access_count = 3
archive_strength = 30
deprecated_strength = 5

[memory]
types = ['arch_decision', 'pitfall', 'codebase', 'preference', 'handoff', 'session_summary']

[injection]
max_tokens = 2000
summary_chars = 120
show_command_template = ''

[hooks]
write_tools = ['Edit', 'Write']

[search]
index_enabled = true

[embedding]
enabled = false
backend = 'onnx'
model = 'BAAI/bge-small-zh-v1.5'
dimensions = 512
batch_size = 32

[rerank]
enabled = false
backend = 'cross_encoder'
model = 'BAAI/bge-reranker-base'
top_n = 5

[distill]
enabled = false
engine = 'heuristic'
session_summary = false
confidence_threshold = 0.6
max_findings_per_session = 5
dedup_threshold = 0.85
subject_threshold = 0.5

[distill.llm]
model = ''

[fusion]
rrf_k = 60
link_expansion = true
link_expansion_decay_fallback = 0.5
link_expansion_max_hops = 1
bm25_pool_size = 50
vec_pool_size = 50

[relations]
allow_custom = false

[mcp]
expose_global = true
expose_project = true
default_search_limit = 5

[mcp.sse]
enabled = false
port = 3700
host = '127.0.0.1'
"#;
const PROJECT_CORE: &str = include_str!("../assets/templates/core_project.md");
const GLOBAL_CORE: &str = include_str!("../assets/templates/core_global.md");
const TRUSTED: &[&[&str]] = &[
    &["distill", "llm", "api_base"],
    &["distill", "llm", "api_key_env"],
    &["distill", "llm", "backend"],
    &["embedding", "api_base"],
    &["embedding", "api_key_env"],
    &["embedding", "onnx_path"],
    &["rerank", "onnx_path"],
    &["mcp", "sse", "host"],
    &["mcp", "sse", "port"],
];

#[derive(Clone, Debug)]
pub struct Store {
    pub scope: String,
    pub root: PathBuf,
}

impl Store {
    pub fn core_path(&self) -> PathBuf {
        self.root.join("core.md")
    }
    pub fn working_dir(&self) -> PathBuf {
        self.root.join("working")
    }
    pub fn archive_dir(&self) -> PathBuf {
        self.root.join("archive")
    }
    pub fn config_path(&self) -> PathBuf {
        self.root.join("config.toml")
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn expand_home(path: PathBuf) -> PathBuf {
    if path == Path::new("~") {
        home()
    } else if let Ok(rest) = path.strip_prefix("~/") {
        home().join(rest)
    } else {
        path
    }
}

pub fn global_store() -> Store {
    Store {
        scope: "global".into(),
        root: expand_home(
            std::env::var_os("MNEMOSYNE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("~/.mnemosyne")),
        ),
    }
}

pub fn find_project_store() -> Option<Store> {
    let mut current = std::env::current_dir().ok()?.canonicalize().ok()?;
    let global = global_store().root.canonicalize().ok();
    loop {
        let candidate = current.join(".mnemosyne");
        if candidate.exists() && candidate.canonicalize().ok() != global {
            return Some(Store {
                scope: "project".into(),
                root: candidate,
            });
        }
        if current.join(".git").exists() || !current.pop() {
            return None;
        }
    }
}

pub fn project_store() -> Store {
    find_project_store().unwrap_or_else(|| Store {
        scope: "project".into(),
        root: std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".mnemosyne"),
    })
}

pub fn stores_for_scope(scope: &str) -> Result<Vec<Store>> {
    match scope {
        "global" => Ok(vec![global_store()]),
        "project" => Ok(vec![project_store()]),
        "all" => {
            let mut stores = vec![global_store()];
            if let Some(project) = find_project_store() {
                stores.push(project);
            }
            Ok(stores)
        }
        _ => bail!("unknown scope: {scope}"),
    }
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            bail!("symlink is not allowed: {}", path.display())
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn checked_directory(path: &Path) -> Result<()> {
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        bail!("parent traversal is not allowed: {}", path.display());
    }
    reject_symlink(path)?;
    if path.exists() && !path.is_dir() {
        bail!("not a directory: {}", path.display());
    }
    Ok(())
}

fn create_dir(path: &Path) -> Result<()> {
    checked_directory(path)?;
    fs::create_dir_all(path)?;
    checked_directory(path)
}

pub fn ensure_store(store: &Store) -> Result<()> {
    create_dir(&store.root)?;
    create_dir(&store.working_dir())?;
    create_dir(&store.archive_dir())?;
    for (path, contents) in [
        (
            store.core_path(),
            if store.scope == "project" {
                PROJECT_CORE
            } else {
                GLOBAL_CORE
            },
        ),
        (
            store.config_path(),
            if store.scope == "project" {
                DEFAULT_CONFIG_TOML
            } else {
                ""
            },
        ),
    ] {
        if path == store.config_path() && store.scope != "project" {
            continue;
        }
        reject_symlink(&path)?;
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(contents.as_bytes())?;
                file.sync_all()?;
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn default_config() -> Value {
    json!({
      "thresholds":{"decay_per_run":1,"bonus_access":5,"bonus_write":10,"bonus_recall":20,"core_strength":80,"core_access_count":3,"archive_strength":30,"deprecated_strength":5},
      "memory":{"types":["arch_decision","pitfall","codebase","preference","handoff","session_summary"]},
      "injection":{"max_tokens":2000,"summary_chars":120,"show_command_template":""},
      "hooks":{"write_tools":["Edit","Write"]}, "search":{"index_enabled":true},
      "embedding":{"enabled":false,"backend":"onnx","model":"BAAI/bge-small-zh-v1.5","onnx_path":"","api_base":"https://api.openai.com/v1","api_key_env":"OPENAI_API_KEY","dimensions":512,"batch_size":32},
      "rerank":{"enabled":false,"backend":"cross_encoder","model":"BAAI/bge-reranker-base","onnx_path":"","top_n":5},
      "distill":{"enabled":false,"engine":"heuristic","session_summary":false,"confidence_threshold":0.6,"max_findings_per_session":5,"dedup_threshold":0.85,"subject_threshold":0.5,"llm":{"backend":"openai","model":"","api_base":"https://api.openai.com/v1","api_key_env":"OPENAI_API_KEY"}},
      "fusion":{"rrf_k":60,"link_expansion":true,"link_expansion_decay_fallback":0.5,"link_expansion_max_hops":1,"bm25_pool_size":50,"vec_pool_size":50},
      "relations":{"allow_custom":false},
      "mcp":{"expose_global":true,"expose_project":true,"default_search_limit":5,"sse":{"enabled":false,"port":3700,"host":"127.0.0.1"}}
    })
}

fn at<'a>(mut data: &'a Value, path: &[&str]) -> Option<&'a Value> {
    for key in path {
        data = data.get(*key)?;
    }
    Some(data)
}

fn remove_at(mut data: &mut Value, path: &[&str]) -> Option<Value> {
    for key in &path[..path.len() - 1] {
        data = data.get_mut(*key)?;
    }
    data.as_object_mut()?.remove(path[path.len() - 1])
}

fn set_at(mut data: &mut Value, path: &[&str], value: Value) {
    for key in &path[..path.len() - 1] {
        if !data.is_object() {
            *data = json!({});
        }
        data = &mut data[*key];
    }
    if !data.is_object() {
        *data = json!({});
    }
    data[path[path.len() - 1]] = value;
}

fn merge_config(target: &mut Value, loaded: &Value) {
    if let (Some(dst), Some(src)) = (target.as_object_mut(), loaded.as_object()) {
        for (section, fields) in src {
            if let Some(fields) = fields.as_object() {
                let entry = dst.entry(section).or_insert_with(|| json!({}));
                if let Some(entry) = entry.as_object_mut() {
                    for (key, value) in fields {
                        entry.insert(key.clone(), value.clone());
                    }
                }
            }
        }
    }
}

fn read_config(store: &Store) -> Result<Value> {
    checked_directory(&store.root)?;
    let path = store.config_path();
    reject_symlink(&path)?;
    if !path.exists() {
        return Ok(json!({}));
    }
    let text = fs::read_to_string(&path)?;
    let parsed: toml::Value =
        toml::from_str(&text).with_context(|| format!("invalid config: {}", path.display()))?;
    Ok(serde_json::to_value(parsed)?)
}

pub fn load_config(store: Option<&Store>) -> Result<Value> {
    let chosen = store
        .cloned()
        .unwrap_or_else(|| find_project_store().unwrap_or_else(global_store));
    let mut config = default_config();
    let mut loaded = read_config(&chosen)?;
    if chosen.scope != "global" {
        for path in TRUSTED {
            if let Some(value) = remove_at(&mut loaded, path)
                && Some(&value) != at(&default_config(), path)
            {
                eprintln!(
                    "mnemosyne: ignoring {} from {} (only global config may set it)",
                    path.join("."),
                    chosen.config_path().display()
                );
            }
        }
    }
    merge_config(&mut config, &loaded);
    if chosen.scope != "global" {
        let global = load_config(Some(&global_store()))?;
        for path in TRUSTED {
            if let Some(value) = at(&global, path) {
                set_at(&mut config, path, value.clone());
            }
        }
    }
    Ok(config)
}

pub fn read_core(store: &Store) -> Result<String> {
    checked_directory(&store.root)?;
    let path = store.core_path();
    reject_symlink(&path)?;
    if !path.exists() {
        return Ok(String::new());
    }
    Ok(fs::read_to_string(path)?)
}

fn memory_paths(store: &Store, include_archive: bool) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let working = store.working_dir();
    checked_directory(&working)?;
    if working.exists() {
        for entry in fs::read_dir(&working)? {
            let path = entry?.path();
            if path.extension().is_some_and(|v| v == "md")
                && path.is_file()
                && fs::symlink_metadata(&path)?.file_type().is_file()
            {
                paths.push(path);
            }
        }
    }
    paths.sort();
    if include_archive {
        let archive = store.archive_dir();
        checked_directory(&archive)?;
        if archive.exists() {
            let mut archived = Vec::new();
            for month in fs::read_dir(&archive)? {
                let month = month?.path();
                if fs::symlink_metadata(&month)?.file_type().is_symlink() || !month.is_dir() {
                    continue;
                }
                for entry in fs::read_dir(&month)? {
                    let path = entry?.path();
                    if path.extension().is_some_and(|v| v == "md")
                        && fs::symlink_metadata(&path)?.file_type().is_file()
                    {
                        archived.push(path);
                    }
                }
            }
            archived.sort();
            paths.extend(archived);
        }
    }
    Ok(paths)
}

pub fn load_memories(store: &Store, include_archive: bool) -> Result<Vec<(PathBuf, Memory)>> {
    if !store.root.exists() {
        return Ok(Vec::new());
    }
    let _lock = lock_store(store)?;
    load_memories_unlocked(store, include_archive)
}

/// Cache files are disposable, but must not redirect SQLite writes outside the store.
pub fn cache_path(store: &Store, name: &str) -> Result<PathBuf> {
    checked_directory(&store.root)?;
    ensure!(
        !name.contains('/') && !name.contains('\\'),
        "Invalid cache filename"
    );
    let path = store.root.join(name);
    for suffix in ["", "-wal", "-shm", "-journal"] {
        reject_symlink(&store.root.join(format!("{name}{suffix}")))?;
    }
    Ok(path)
}

/// Use only while holding `lock_store` (or a read-only store lock with no pending journal).
pub fn load_memories_unlocked(
    store: &Store,
    include_archive: bool,
) -> Result<Vec<(PathBuf, Memory)>> {
    let mut memories = Vec::new();
    for path in memory_paths(store, include_archive)? {
        if let Ok(text) = fs::read_to_string(&path)
            && let Ok(memory) = parse_memory(&text)
        {
            memories.push((path, memory));
        }
    }
    Ok(memories)
}

pub struct StoreLock {
    file: File,
}
impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn lock_file(path: &Path) -> Result<StoreLock> {
    lock_file_timeout(path, Duration::from_secs(30))
}

fn lock_file_timeout(path: &Path, timeout: Duration) -> Result<StoreLock> {
    reject_symlink(path)?;
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    reject_symlink(path)?;
    let until = Instant::now() + timeout;
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(StoreLock { file }),
            Err(e) if e.kind() == ErrorKind::WouldBlock && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(e) => return Err(e.into()),
        }
    }
}

pub fn lock_store(store: &Store) -> Result<StoreLock> {
    create_dir(&store.root)?;
    let lock = lock_file(&store.root.join(".lock"))?;
    crate::relations::recover_pending(store)?;
    Ok(lock)
}

pub fn try_lock_store(store: &Store) -> Result<StoreLock> {
    create_dir(&store.root)?;
    let lock = lock_file_timeout(&store.root.join(".lock"), Duration::ZERO)?;
    crate::relations::recover_pending(store)?;
    Ok(lock)
}

pub fn lock_store_read_only(store: &Store) -> Result<StoreLock> {
    checked_directory(&store.root)?;
    lock_file(&store.root.join(".lock"))
}

pub fn working_path(store: &Store, memory: &Memory) -> Result<PathBuf> {
    if memory.id.is_empty()
        || !memory
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!("invalid memory id: {}", memory.id);
    }
    checked_directory(&store.root)?;
    checked_directory(&store.working_dir())?;
    let path = store.working_dir().join(format!("{}.md", memory.id));
    reject_symlink(&path)?;
    Ok(path)
}

pub fn write_memory(path: &Path, memory: &Memory) -> Result<()> {
    let parent = path.parent().context("memory path has no parent")?;
    if path.components().any(|c| matches!(c, Component::ParentDir))
        || path.extension().is_none_or(|v| v != "md")
    {
        bail!("invalid memory path: {}", path.display());
    }
    create_dir(parent)?;
    reject_symlink(path)?;
    let lock_path = path.with_extension("md.lock");
    let _lock = lock_file(&lock_path)?;
    reject_symlink(path)?;
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap().to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(serialize_memory(memory).as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub fn find_memory(
    memory_id: &str,
    stores: &[Store],
    include_archive: bool,
) -> Result<Option<(Store, PathBuf, Memory)>> {
    for store in stores {
        for (path, memory) in load_memories(store, include_archive)? {
            if memory.id == memory_id {
                return Ok(Some((store.clone(), path, memory)));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secure_atomic_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store {
            scope: "project".into(),
            root: dir.path().join(".mnemosyne"),
        };
        ensure_store(&store).unwrap();
        assert!(
            read_core(&store)
                .unwrap()
                .starts_with("# Project Core Memory")
        );
        let memory = Memory {
            id: "safe-id".into(),
            body: "# Title".into(),
            ..Default::default()
        };
        let path = working_path(&store, &memory).unwrap();
        write_memory(&path, &memory).unwrap();
        assert_eq!(load_memories(&store, false).unwrap()[0].1.title(), "Title");
        assert!(
            working_path(
                &store,
                &Memory {
                    id: "../bad".into(),
                    ..memory.clone()
                }
            )
            .is_err()
        );
        assert_eq!(
            load_config(Some(&store)).unwrap()["mcp"]["sse"]["host"],
            "127.0.0.1"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = dir.path().join("outside.md");
            symlink(&outside, store.working_dir().join("linked.md")).unwrap();
            assert!(write_memory(&store.working_dir().join("linked.md"), &memory).is_err());
            assert!(!outside.exists());
        }
    }
}
