//! Typed relations and recoverable two-file mutations across local stores.
use crate::{
    schema::{Link, Memory, serialize_memory},
    store::{
        Store, global_store, load_config, load_memories_unlocked, lock_store, lock_store_read_only,
    },
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};

const JOURNAL: &str = ".relations-operation.json";
const COMMITS: &str = ".relations-commits";

pub fn reverse(rel: &str) -> Option<&'static str> {
    match rel {
        "caused_by" => Some("causes"),
        "refines" => Some("refined_by"),
        "supersedes" => Some("superseded_by"),
        "contradicts" => Some("contradicts"),
        "related" => Some("related"),
        _ => None,
    }
}
pub fn weight(rel: &str) -> f64 {
    match rel {
        "caused_by" => 0.6,
        "refines" => 0.7,
        "supersedes" => 0.3,
        _ => 0.5,
    }
}
pub fn is_demoting(rel: &str) -> bool {
    rel == "supersedes"
}
pub fn is_symmetric(rel: &str) -> bool {
    matches!(rel, "contradicts" | "related")
}
pub fn warns(rel: &str) -> bool {
    rel == "contradicts"
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

#[derive(Clone, Serialize, Deserialize)]
struct Change {
    path: PathBuf,
    before: String,
    after: String,
    before_hash: String,
    after_hash: String,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    version: u32,
    changes: Vec<Change>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    coordinator_key: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct CommitDecision {
    version: u32,
    participants: BTreeMap<String, String>,
}

fn store_key(store: &Store) -> Result<String> {
    let root = fs::canonicalize(&store.root)?;
    Ok(digest(root.to_str().context("Store path is not UTF-8")?))
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => ensure!(
            !meta.file_type().is_symlink(),
            "Symlink in relation coordinator"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

// The coordinator comes only from trusted host configuration, never a journal
// path. The journal can select only a canonical UUID within this fixed directory.
fn decision_path(coordinator: &Store, operation_id: &str) -> Result<PathBuf> {
    ensure!(
        uuid::Uuid::parse_str(operation_id)?.to_string() == operation_id,
        "Invalid relation operation ID"
    );
    reject_symlink(&coordinator.root)?;
    let directory = coordinator.root.join(COMMITS);
    reject_symlink(&directory)?;
    let path = directory.join(format!("{operation_id}.json"));
    reject_symlink(&path)?;
    Ok(path)
}

fn prepare_coordinator(coordinator: &Store) -> Result<()> {
    reject_symlink(&coordinator.root)?;
    fs::create_dir_all(&coordinator.root)?;
    if let Some(parent) = coordinator.root.parent() {
        File::open(parent)?.sync_all()?;
    }
    let directory = coordinator.root.join(COMMITS);
    reject_symlink(&directory)?;
    fs::create_dir_all(&directory)?;
    File::open(&coordinator.root)?.sync_all()?;
    Ok(())
}

fn atomic_write(path: &Path, text: &str) -> Result<()> {
    let parent = path.parent().context("Missing parent")?;
    let tmp = parent.join(format!(".relations-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new().create_new(true).write(true).open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn checked_path(store: &Store, path: &Path) -> Result<PathBuf> {
    ensure!(
        !path.as_os_str().is_empty()
            && path.components().all(|c| matches!(c, Component::Normal(_))),
        "Unsafe relation journal path"
    );
    ensure!(
        matches!(path.components().next(),Some(Component::Normal(p)) if p == "working" || p == "archive"),
        "Journal target outside memory directories"
    );
    let root = fs::canonicalize(&store.root)?;
    let mut current = root.clone();
    for component in path.components() {
        current.push(component.as_os_str());
        ensure!(
            !fs::symlink_metadata(&current)?.file_type().is_symlink(),
            "Symlink in relation journal target"
        );
    }
    ensure!(current.is_file(), "Missing journal target");
    Ok(current)
}

/// Caller holds the store lock. Roll forward only when every file is a known
/// before/after image; an external edit leaves the journal and all files intact.
pub fn recover_pending(store: &Store) -> Result<()> {
    recover_with_coordinator(store, &global_store())
}

fn recover_with_coordinator(store: &Store, coordinator: &Store) -> Result<()> {
    let path = store.root.join(JOURNAL);
    if !path.exists() {
        return Ok(());
    }
    ensure!(
        !fs::symlink_metadata(&path)?.file_type().is_symlink(),
        "Symlink relation journal"
    );
    let raw = fs::read_to_string(&path)?;
    let journal: Journal = serde_json::from_str(&raw)?;
    let committed = match journal.version {
        1 => {
            ensure!(
                journal.changes.len() == 2
                    && journal.operation_id.is_none()
                    && journal.coordinator_key.is_none(),
                "Unsupported relation journal"
            );
            true
        }
        2 => {
            ensure!(journal.changes.len() == 1, "Unsupported relation journal");
            let operation = journal
                .operation_id
                .as_deref()
                .context("Missing operation ID")?;
            ensure!(
                journal.coordinator_key.as_deref() == Some(store_key(coordinator)?.as_str()),
                "RECOVERY_CONFLICT: trusted coordinator changed"
            );
            let decision = decision_path(coordinator, operation)?;
            if decision.exists() {
                let decision: CommitDecision =
                    serde_json::from_str(&fs::read_to_string(decision)?)?;
                ensure!(
                    decision.version == 1
                        && decision.participants.len() == 2
                        && decision.participants.get(&store_key(store)?) == Some(&digest(&raw)),
                    "RECOVERY_CONFLICT: journal does not match commit decision"
                );
                true
            } else {
                false
            }
        }
        _ => anyhow::bail!("Unsupported relation journal"),
    };
    let mut seen = HashSet::new();
    let mut pending = Vec::new();
    for change in &journal.changes {
        ensure!(
            digest(&change.before) == change.before_hash
                && digest(&change.after) == change.after_hash,
            "Corrupt relation journal images"
        );
        let target = checked_path(store, &change.path)?;
        ensure!(
            seen.insert(target.clone()),
            "Duplicate relation journal target"
        );
        let current = fs::read_to_string(&target)?;
        ensure!(
            current == change.before || (committed && current == change.after),
            "RECOVERY_CONFLICT: external edit at {}; journal retained",
            target.display()
        );
        if committed && current != change.after {
            pending.push((target, &change.after));
        }
    }
    for (target, after) in pending {
        atomic_write(&target, after)?;
    }
    fs::remove_file(&path)?;
    File::open(&store.root)?.sync_all()?;
    Ok(())
}

// Caller holds both participant locks in canonical order. No memory is changed
// before the one durable decision. The callback is for deterministic crash-stage
// tests; ordinary callers supply a no-op.
fn commit_cross_store(
    stores: [&Store; 2],
    changes: Vec<Change>,
    coordinator: &Store,
    mut checkpoint: impl FnMut(usize) -> Result<()>,
) -> Result<()> {
    ensure!(changes.len() == 2, "Expected two relation changes");
    prepare_coordinator(coordinator)?;
    let operation = uuid::Uuid::new_v4().to_string();
    let decision_path = decision_path(coordinator, &operation)?;
    let mut participants = BTreeMap::new();
    for (index, (store, change)) in stores.iter().zip(changes).enumerate() {
        let journal = Journal {
            version: 2,
            changes: vec![change],
            operation_id: Some(operation.clone()),
            coordinator_key: Some(store_key(coordinator)?),
        };
        let text = serde_json::to_string(&journal)?;
        ensure!(
            participants
                .insert(store_key(store)?, digest(&text))
                .is_none(),
            "Duplicate participant store"
        );
        ensure!(
            !store.root.join(JOURNAL).exists(),
            "Pending relation journal"
        );
        atomic_write(&store.root.join(JOURNAL), &text)?;
        checkpoint(index + 1)?;
    }
    atomic_write(
        &decision_path,
        &serde_json::to_string(&CommitDecision {
            version: 1,
            participants,
        })?,
    )?;
    checkpoint(3)?;
    for (index, store) in stores.iter().enumerate() {
        recover_with_coordinator(store, coordinator)?;
        checkpoint(index + 4)?;
    }
    // Both journal deletions are already fsynced before removing the decision.
    // ponytail: interrupted operations retain a small decision file so a delayed
    // peer can still recover; reclaim only with a future all-participant audit.
    fs::remove_file(&decision_path)?;
    File::open(
        decision_path
            .parent()
            .context("Missing decision directory")?,
    )?
    .sync_all()?;
    Ok(())
}

fn add_link(memory: &mut Memory, id: &str, rel: &str) -> bool {
    if memory.links.iter().any(|l| l.id == id && l.rel == rel) {
        return false;
    }
    memory.links.push(Link {
        id: id.to_owned(),
        rel: rel.to_owned(),
    });
    true
}

pub fn link_entries(
    stores: &[Store],
    id1: &str,
    id2: &str,
    rel: &str,
    allow_custom: bool,
) -> Result<Value> {
    link_entries_checked(stores, id1, id2, rel, allow_custom, None)
}

pub fn link_entries_checked(
    stores: &[Store],
    id1: &str,
    id2: &str,
    rel: &str,
    allow_custom: bool,
    expected: Option<(&Memory, &Memory)>,
) -> Result<Value> {
    link_entries_inner(
        stores,
        id1,
        id2,
        rel,
        allow_custom,
        expected,
        &global_store(),
    )
}

fn link_entries_inner(
    stores: &[Store],
    id1: &str,
    id2: &str,
    rel: &str,
    allow_custom: bool,
    expected: Option<(&Memory, &Memory)>,
    coordinator: &Store,
) -> Result<Value> {
    ensure!(id1 != id2, "Cannot link a memory to itself");
    ensure!(!rel.trim().is_empty(), "Empty relation");
    // Stable lock order also prevents a concurrent participant changing either
    // endpoint while we resolve legacy bare IDs across the visible stores.
    let mut selected: Vec<_> = stores.iter().filter(|s| s.root.exists()).collect();
    selected.sort_by_key(|s| fs::canonicalize(&s.root).unwrap_or_else(|_| s.root.clone()));
    selected.dedup_by(|a, b| fs::canonicalize(&a.root).ok() == fs::canonicalize(&b.root).ok());
    let _guards = selected
        .iter()
        .map(|s| lock_store(s))
        .collect::<Result<Vec<_>>>()?;
    let mut entries = Vec::new();
    for store in selected {
        recover_pending(store)?;
        for (path, memory) in load_memories_unlocked(store, true)? {
            entries.push((store, path, memory));
        }
    }
    let locate = |id: &str| -> Result<usize> {
        let matches: Vec<_> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.2.id == id)
            .map(|(i, _)| i)
            .collect();
        ensure!(matches.len() == 1, "Memory missing or ambiguous: {id}");
        Ok(matches[0])
    };
    let a = locate(id1)?;
    let b = locate(id2)?;
    if let Some((first, second)) = expected {
        for (current, old) in [(&entries[a].2, first), (&entries[b].2, second)] {
            ensure!(
                current.id == old.id
                    && current.body == old.body
                    && current.memory_type == old.memory_type
                    && current.status == old.status
                    && current.tags == old.tags
                    && current.expires == old.expires
                    && current.source == old.source
                    && current.extra == old.extra,
                "VERSION_CONFLICT: consolidation candidate changed"
            );
        }
    }
    let same_store = fs::canonicalize(&entries[a].0.root)? == fs::canonicalize(&entries[b].0.root)?;
    let store = entries[a].0;
    let config = load_config(Some(store))?;
    ensure!(
        reverse(rel).is_some()
            || allow_custom
            || config["relations"]["allow_custom"]
                .as_bool()
                .unwrap_or(false),
        "Unknown relation: {rel}"
    );
    if is_demoting(rel) {
        let mut queue = vec![id2.to_owned()];
        let mut visited = HashSet::new();
        while let Some(id) = queue.pop() {
            ensure!(id != id1, "Supersedes would create a cycle");
            if !visited.insert(id.clone()) {
                continue;
            }
            let matches = entries
                .iter()
                .filter(|(_, _, m)| m.id == id)
                .collect::<Vec<_>>();
            ensure!(matches.len() <= 1, "Ambiguous supersedes target: {id}");
            if let Some((_, _, m)) = matches.first() {
                queue.extend(
                    m.links
                        .iter()
                        .filter(|l| l.rel == "supersedes")
                        .map(|l| l.id.clone()),
                );
            }
        }
    }
    let mut first = entries[a].2.clone();
    let mut second = entries[b].2.clone();
    let added = add_link(&mut first, id2, rel);
    add_link(&mut second, id1, reverse(rel).unwrap_or(rel));
    if is_demoting(rel) && added {
        second.strength = (second.strength - 20).max(0);
        second.status = "superseded".into();
        second.extra.insert("invalidated_by".into(), json!(id1));
    }
    let mut changes = Vec::new();
    for (index, memory) in [(a, first), (b, second)] {
        let path = &entries[index].1;
        let destination = entries[index].0;
        let relative = path.strip_prefix(&destination.root)?.to_path_buf();
        checked_path(destination, &relative)?;
        let before = fs::read_to_string(path)?;
        // Detect an edit made outside our lock since endpoint resolution.
        ensure!(
            serialize_memory(&crate::schema::parse_memory(&before)?)
                == serialize_memory(&entries[index].2),
            "VERSION_CONFLICT: memory changed during relation operation"
        );
        let after = serialize_memory(&memory);
        changes.push(Change {
            path: relative,
            before_hash: digest(&before),
            after_hash: digest(&after),
            before,
            after,
        });
    }
    if changes.iter().any(|c| c.before != c.after) && !same_store {
        commit_cross_store([entries[a].0, entries[b].0], changes, coordinator, |_| {
            Ok(())
        })?;
    } else if changes.iter().any(|c| c.before != c.after) {
        atomic_write(
            &store.root.join(JOURNAL),
            &serde_json::to_string(&Journal {
                version: 1,
                changes,
                operation_id: None,
                coordinator_key: None,
            })?,
        )?;
        recover_pending(store)?;
    }
    Ok(json!({"ok":true,"rel":rel,"id1":id1,"id2":id2}))
}

pub fn graph(stores: &[Store], id: &str, depth: usize, format: &str) -> Result<String> {
    ensure!(
        matches!(format, "ascii" | "json" | "mermaid"),
        "Unknown graph format: {format}"
    );
    let mut selected: Vec<_> = stores.iter().filter(|s| s.root.exists()).collect();
    selected.sort_by_key(|s| fs::canonicalize(&s.root).unwrap_or_else(|_| s.root.clone()));
    selected.dedup_by(|a, b| fs::canonicalize(&a.root).ok() == fs::canonicalize(&b.root).ok());
    // Graph is read-only: hold the normal sidecar lock but never recover files.
    let mut guards = Vec::new();
    for store in &selected {
        let file = lock_store_read_only(store)?;
        ensure!(
            !store.root.join(JOURNAL).exists(),
            "Pending relation operation; run a writable command to recover"
        );
        guards.push(file);
    }
    let mut entries = HashMap::new();
    for store in selected {
        for (_, memory) in load_memories_unlocked(store, true)? {
            ensure!(
                !entries.contains_key(&memory.id),
                "Ambiguous memory ID: {}",
                memory.id
            );
            entries.insert(memory.id.clone(), (store.scope.clone(), memory));
        }
    }
    ensure!(entries.contains_key(id), "Memory not found: {id}");
    let mut queue = VecDeque::from([(id.to_owned(), 0usize)]);
    let mut visited = HashSet::from([id.to_owned()]);
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut edge_keys = HashSet::new();
    while let Some((current, distance)) = queue.pop_front() {
        let (scope, memory) = &entries[&current];
        nodes.push(
            json!({"id":current,"title":memory.title(),"type":memory.memory_type,"scope":scope}),
        );
        if distance >= depth {
            continue;
        }
        for link in &memory.links {
            if !entries.contains_key(&link.id) {
                continue;
            }
            if edge_keys.insert((current.clone(), link.id.clone(), link.rel.clone())) {
                edges.push(json!({"source":current,"target":link.id,"rel":link.rel}));
            }
            if visited.insert(link.id.clone()) {
                queue.push_back((link.id.clone(), distance + 1));
            }
        }
    }
    if format == "json" {
        return Ok(serde_json::to_string_pretty(
            &json!({"root":id,"nodes":nodes,"edges":edges}),
        )?);
    }
    let mut lines = Vec::new();
    if format == "mermaid" {
        lines.push("graph LR".to_owned());
        // Index-based diagram IDs avoid collisions in legacy punctuation IDs.
        let identifiers: HashMap<_, _> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n["id"].as_str().unwrap(), format!("m{i}")))
            .collect();
        for node in &nodes {
            lines.push(format!(
                "  {}[\"{}\"]",
                identifiers[node["id"].as_str().unwrap()],
                escape(node["title"].as_str().unwrap())
            ));
        }
        for edge in &edges {
            lines.push(format!(
                "  {} -- {} --> {}",
                identifiers[edge["source"].as_str().unwrap()],
                escape(edge["rel"].as_str().unwrap()),
                identifiers[edge["target"].as_str().unwrap()]
            ));
        }
    } else {
        lines.push(id.to_owned());
        let mut visited = HashSet::from([id.to_owned()]);
        let mut stack = edges
            .iter()
            .filter(|e| e["source"] == id)
            .rev()
            .map(|e| (e, 1usize))
            .collect::<Vec<_>>();
        while let Some((edge, level)) = stack.pop() {
            let target = edge["target"].as_str().unwrap();
            let new = visited.insert(target.to_owned());
            lines.push(format!(
                "{}-> [{}] {}{}",
                "  ".repeat(level),
                edge["rel"].as_str().unwrap(),
                target,
                if new { "" } else { " (cycle)" }
            ));
            if new {
                stack.extend(
                    edges
                        .iter()
                        .filter(|e| e["source"] == target)
                        .rev()
                        .map(|e| (e, level + 1)),
                );
            }
        }
    }
    Ok(lines.join("\n"))
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace(['\n', '\r'], " ")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ensure_store, load_memories, working_path, write_memory};
    fn fixture() -> Result<(tempfile::TempDir, Store)> {
        let tmp = tempfile::tempdir()?;
        let store = Store {
            scope: "project".into(),
            root: tmp.path().join(".mnemosyne"),
        };
        ensure_store(&store)?;
        for id in ["a", "b", "c"] {
            let memory = Memory {
                id: id.into(),
                memory_type: "codebase".into(),
                strength: 70,
                status: "active".into(),
                body: format!("## Title {id}"),
                ..Default::default()
            };
            write_memory(&working_path(&store, &memory)?, &memory)?;
        }
        Ok((tmp, store))
    }
    #[test]
    fn typed_links_are_idempotent_and_graph_honors_depth() -> Result<()> {
        let (_tmp, store) = fixture()?;
        let stores = std::slice::from_ref(&store);
        link_entries(stores, "a", "b", "supersedes", false)?;
        link_entries(stores, "a", "b", "supersedes", false)?;
        link_entries(stores, "b", "c", "related", false)?;
        assert!(link_entries(stores, "b", "a", "supersedes", false).is_err());
        let entries = load_memories(&store, true)?;
        let b = &entries.iter().find(|(_, m)| m.id == "b").unwrap().1;
        assert_eq!(b.strength, 50);
        assert_eq!(b.status, "superseded");
        let graph: Value = serde_json::from_str(&graph(stores, "a", 1, "json")?)?;
        assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(graph["edges"].as_array().unwrap().len(), 1);
        assert!(super::graph(stores, "a", 2, "ascii")?.contains("-> [supersedes] b"));
        assert!(super::graph(stores, "a", 2, "mermaid")?.contains("graph LR"));
        Ok(())
    }
    #[test]
    fn partial_commit_recovers_but_external_edit_is_preserved() -> Result<()> {
        let (_tmp, store) = fixture()?;
        let memories = load_memories(&store, true)?;
        let mut changes = Vec::new();
        for (path, memory) in memories.iter().take(2) {
            let before = fs::read_to_string(path)?;
            let mut after_memory = memory.clone();
            after_memory.strength = 42;
            let after = serialize_memory(&after_memory);
            changes.push(Change {
                path: path.strip_prefix(&store.root)?.to_path_buf(),
                before_hash: digest(&before),
                after_hash: digest(&after),
                before,
                after,
            });
        }
        let journal = Journal {
            version: 1,
            changes,
            operation_id: None,
            coordinator_key: None,
        };
        atomic_write(&store.root.join(JOURNAL), &serde_json::to_string(&journal)?)?;
        atomic_write(
            &store.root.join(&journal.changes[0].path),
            &journal.changes[0].after,
        )?;
        assert!(graph(std::slice::from_ref(&store), "a", 1, "json").is_err());
        assert!(store.root.join(JOURNAL).exists());
        let second = store.root.join(&journal.changes[1].path);
        atomic_write(&second, "external edit")?;
        assert!(recover_pending(&store).is_err());
        assert_eq!(fs::read_to_string(&second)?, "external edit");
        assert!(store.root.join(JOURNAL).exists());
        atomic_write(&second, &journal.changes[1].before)?;
        let _guard = lock_store(&store)?;
        recover_pending(&store)?;
        assert!(!store.root.join(JOURNAL).exists());
        assert_eq!(fs::read_to_string(second)?, journal.changes[1].after);
        Ok(())
    }
    #[test]
    fn cross_store_links_keep_reverse_status_and_reject_cycles() -> Result<()> {
        let (_tmp, store) = fixture()?;
        let other_tmp = tempfile::tempdir()?;
        let other = Store {
            scope: "global".into(),
            root: other_tmp.path().join("other"),
        };
        let coordinator = Store {
            scope: "global".into(),
            root: other_tmp.path().join("coordinator"),
        };
        ensure_store(&other)?;
        let memory = Memory {
            id: "other".into(),
            body: "other".into(),
            strength: 70,
            status: "active".into(),
            ..Default::default()
        };
        write_memory(&working_path(&other, &memory)?, &memory)?;
        let stores = [store, other];
        for _ in 0..2 {
            link_entries_inner(
                &stores,
                "a",
                "other",
                "supersedes",
                false,
                None,
                &coordinator,
            )?;
        }
        let target = &load_memories(&stores[1], true)?[0].1;
        assert_eq!(target.status, "superseded");
        assert_eq!(target.strength, 50);
        assert_eq!(target.extra["invalidated_by"], "a");
        assert!(
            target
                .links
                .iter()
                .any(|link| link.id == "a" && link.rel == "superseded_by")
        );
        assert!(
            link_entries_inner(
                &stores,
                "other",
                "a",
                "supersedes",
                false,
                None,
                &coordinator
            )
            .is_err()
        );
        let result: Value = serde_json::from_str(&graph(&stores, "a", 1, "json")?)?;
        assert_eq!(result["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(fs::read_dir(coordinator.root.join(COMMITS))?.count(), 0);
        Ok(())
    }

    fn test_change(store: &Store) -> Result<Change> {
        let (path, mut memory) = load_memories(store, true)?.remove(0);
        let before = fs::read_to_string(&path)?;
        memory.strength = 42;
        let after = serialize_memory(&memory);
        Ok(Change {
            path: path.strip_prefix(&store.root)?.into(),
            before_hash: digest(&before),
            after_hash: digest(&after),
            before,
            after,
        })
    }

    #[test]
    fn cross_store_interruption_recovers_at_every_durable_stage() -> Result<()> {
        for stage in 1..=5 {
            let (_left_tmp, left) = fixture()?;
            let (_right_tmp, right) = fixture()?;
            let coordinator_tmp = tempfile::tempdir()?;
            let coordinator = Store {
                scope: "global".into(),
                root: coordinator_tmp.path().join("global"),
            };
            let changes = vec![test_change(&left)?, test_change(&right)?];
            {
                let _left_lock = lock_store_read_only(&left)?;
                let _right_lock = lock_store_read_only(&right)?;
                assert!(
                    commit_cross_store([&left, &right], changes.clone(), &coordinator, |point| {
                        ensure!(point != stage, "injected crash at stage {stage}");
                        Ok(())
                    })
                    .is_err()
                );
            }
            // A new caller has only its own store and trusted global config.
            // Recover peers in reverse order to exercise delayed participants.
            for (store, change) in [(&right, &changes[1]), (&left, &changes[0])] {
                let _guard = lock_store_read_only(store)?;
                recover_with_coordinator(store, &coordinator)?;
                assert_eq!(
                    fs::read_to_string(store.root.join(&change.path))?,
                    if stage >= 3 {
                        &change.after
                    } else {
                        &change.before
                    }
                    .to_owned()
                );
                assert!(!store.root.join(JOURNAL).exists());
            }
        }
        Ok(())
    }

    #[test]
    fn cross_store_recovery_preserves_external_edits_and_rejects_journal_tampering() -> Result<()> {
        let (_left_tmp, left) = fixture()?;
        let (_right_tmp, right) = fixture()?;
        let coordinator_tmp = tempfile::tempdir()?;
        let coordinator = Store {
            scope: "global".into(),
            root: coordinator_tmp.path().join("global"),
        };
        let changes = vec![test_change(&left)?, test_change(&right)?];
        assert!(
            commit_cross_store([&left, &right], changes.clone(), &coordinator, |point| {
                ensure!(point != 3, "injected crash after decision");
                Ok(())
            })
            .is_err()
        );
        let _guard = lock_store_read_only(&left)?;
        let target = left.root.join(&changes[0].path);
        atomic_write(&target, "external edit")?;
        assert!(recover_with_coordinator(&left, &coordinator).is_err());
        assert_eq!(fs::read_to_string(&target)?, "external edit");
        assert!(left.root.join(JOURNAL).exists());
        atomic_write(&target, &changes[0].before)?;
        let original = fs::read_to_string(left.root.join(JOURNAL))?;
        let mut tampered: Journal = serde_json::from_str(&original)?;
        tampered.operation_id = Some("../../outside".into());
        atomic_write(&left.root.join(JOURNAL), &serde_json::to_string(&tampered)?)?;
        assert!(recover_with_coordinator(&left, &coordinator).is_err());
        assert_eq!(fs::read_to_string(&target)?, changes[0].before);
        tampered = serde_json::from_str(&original)?;
        tampered.changes[0].path = PathBuf::from("../outside.md");
        atomic_write(&left.root.join(JOURNAL), &serde_json::to_string(&tampered)?)?;
        assert!(recover_with_coordinator(&left, &coordinator).is_err());
        assert!(left.root.join(JOURNAL).exists());
        atomic_write(&left.root.join(JOURNAL), &original)?;
        recover_with_coordinator(&left, &coordinator)?;
        assert_eq!(fs::read_to_string(target)?, changes[0].after);
        Ok(())
    }
}
