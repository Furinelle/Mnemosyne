//! Reviewed same-store changes, committed with their audit state and semantic history.
use crate::{
    provenance::{Clock, MemoryRef, read_manifest},
    relations::{MutationChange, MutationPlan, execute_mutation},
    revisions::{self, RevisionUpdate, Snapshot},
    schema::{Link, Memory},
    store::{Store, load_memories_unlocked, lock_store, lock_store_read_only},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};

const MAX_PROPOSAL_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub memory_ref: MemoryRef,
    pub expected_rev: u64,
    pub expected_hash: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub decision: String,
    pub reason: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub targets: Vec<Target>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proposal {
    pub version: u32,
    pub id: String,
    pub store_id: String,
    pub state: String,
    pub request: Request,
    pub summary_hash: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_summary_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_origin_summary_hash: Option<String>,
    pub before: Vec<Memory>,
    pub applied: Vec<Snapshot>,
    #[serde(default)]
    pub dependents: Vec<Dependent>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dependent {
    pub memory_id: String,
    pub semantic_hash: String,
}

fn relation(request: &Request) -> Option<&'static str> {
    match (request.decision.as_str(), request.targets.len()) {
        ("REFINE", 2) => Some("refines"),
        ("CAUSED_BY", 2) => Some("caused_by"),
        ("CONTRADICT", 2) => Some("contradicts"),
        ("SUPERSEDE", 2) => Some("supersedes"),
        _ => None,
    }
}

fn add_link(memory: &mut Memory, other: &str, rel: &str) -> bool {
    if memory
        .links
        .iter()
        .any(|link| link.id == other && link.rel == rel)
    {
        return false;
    }
    memory.links.push(Link {
        id: other.into(),
        rel: rel.into(),
    });
    true
}

fn check_supersede_cycle(memories: &[(PathBuf, Memory)], source: &str, target: &str) -> Result<()> {
    let mut queue = vec![target.to_owned()];
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = queue.pop() {
        ensure!(id != source, "Supersedes would create a cycle");
        if seen.insert(id.clone())
            && let Some((_, memory)) = memories.iter().find(|(_, memory)| memory.id == id)
        {
            queue.extend(
                memory
                    .links
                    .iter()
                    .filter(|link| link.rel == "supersedes")
                    .map(|link| link.id.clone()),
            );
        }
    }
    Ok(())
}
fn digest(v: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(v)?)))
}
fn path(store: &Store, id: &str) -> Result<PathBuf> {
    ensure!(
        !id.is_empty()
            && id.len() <= 64
            && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "Invalid proposal ID"
    );
    ensure!(
        !store.root.is_symlink() && !store.root.join("proposals").is_symlink(),
        "Symlink proposal directory"
    );
    let p = store.root.join("proposals").join(format!("{id}.json"));
    ensure!(!p.is_symlink(), "Symlink proposal");
    Ok(p)
}
fn read(store: &Store, id: &str) -> Result<(String, Proposal)> {
    let p = path(store, id)?;
    let raw = String::from_utf8(crate::input::read_bytes(
        fs::File::open(p)?,
        MAX_PROPOSAL_BYTES,
    )?)?;
    let value: Proposal = serde_json::from_str(&raw)?;
    ensure!(
        value.version == 1
            && value.id == id
            && value.id == value.summary_hash
            && digest(&value.request)? == value.summary_hash,
        "Corrupt proposal"
    );
    ensure!(
        read_manifest(store)?.context("Upgrade required")?.store_id == value.store_id,
        "STORE_ID_MISMATCH"
    );
    ensure!(
        value
            .approved_summary_hash
            .as_deref()
            .is_none_or(|hash| hash == value.summary_hash),
        "Corrupt approval audit"
    );
    Ok((raw, value))
}
fn append(plan: &mut MutationPlan, p: &Proposal, before: Option<String>) -> Result<()> {
    let after = serde_json::to_string_pretty(p)? + "\n";
    ensure!(
        after.len() <= MAX_PROPOSAL_BYTES,
        "Proposal record too large"
    );
    plan.changes.push(MutationChange {
        path: PathBuf::from("proposals").join(format!("{}.json", p.id)),
        before,
        after: Some(after),
    });
    Ok(())
}
pub fn propose(store: &Store, request: Request, clock: &impl Clock) -> Result<Proposal> {
    let _lock = lock_store_read_only(store)?;
    crate::relations::recover_pending(store)?;
    let mut plan = MutationPlan { changes: vec![] };
    let proposal = plan_proposal(store, request, clock, &mut plan)?;
    if !plan.changes.is_empty() {
        execute_mutation(store, plan)?;
    }
    Ok(proposal)
}

/// Caller holds the store lock; append publication to its existing transaction.
pub(crate) fn plan_proposal(
    store: &Store,
    request: Request,
    clock: &impl Clock,
    plan: &mut MutationPlan,
) -> Result<Proposal> {
    ensure!(
        [
            "REFINE",
            "CONTEXTUALIZE",
            "CONTRADICT",
            "SUPERSEDE",
            "CAUSED_BY"
        ]
        .contains(&request.decision.as_str()),
        "Invalid proposal decision"
    );
    ensure!(
        !request.reason.trim().is_empty()
            && request.reason.len() <= 4096
            && request.evidence.len() <= 32
            && request.evidence.iter().all(|x| x.len() <= 4096),
        "Invalid proposal evidence/reason"
    );
    ensure!(
        !request.targets.is_empty() && request.targets.len() <= 32,
        "Invalid target count"
    );
    let rel = relation(&request);
    ensure!(
        rel.is_some()
            || (request.targets.len() == 1
                && matches!(request.decision.as_str(), "REFINE" | "CONTEXTUALIZE")),
        "Decision requires one content target or two relation endpoints"
    );
    let mut seen = std::collections::HashSet::new();
    for t in &request.targets {
        ensure!(
            seen.insert(t.memory_ref.memory_id.clone()),
            "Duplicate target"
        );
        ensure!(
            t.body
                .as_ref()
                .is_none_or(|b| !b.trim().is_empty() && b.len() <= 1024 * 1024),
            "Invalid proposal body"
        );
        ensure!(
            t.status.is_none(),
            "Status changes require a structured relation"
        );
        ensure!(
            rel.is_none() || t.body.is_none(),
            "Relation proposal cannot rewrite content"
        );
    }
    ensure!(
        rel.is_some() || request.targets[0].body.is_some(),
        "Content proposal needs a body"
    );
    // A pending proposal does not read or change its target memories.
    let manifest = read_manifest(store)?.context("Upgrade required")?;
    ensure!(
        manifest.min_writer_version == crate::provenance::WRITER_VERSION,
        "Upgrade required"
    );
    ensure!(
        request
            .targets
            .iter()
            .all(|t| t.memory_ref.store_id == manifest.store_id),
        "Cross-store proposal requires separate review"
    );
    let hash = digest(&request)?;
    let id = hash.clone();
    if let Some(change) = plan
        .changes
        .iter()
        .find(|change| change.path == PathBuf::from("proposals").join(format!("{id}.json")))
    {
        return Ok(serde_json::from_str(
            change.after.as_deref().context("Missing proposal")?,
        )?);
    }
    if path(store, &id)?.exists() {
        return Ok(read(store, &id)?.1);
    }
    let p = Proposal {
        version: 1,
        id,
        store_id: manifest.store_id,
        state: "pending".into(),
        request,
        summary_hash: hash,
        created_at: clock.now().to_rfc3339(),
        approved_at: None,
        approved_summary_hash: None,
        fork_origin_summary_hash: None,
        before: vec![],
        applied: vec![],
        dependents: vec![],
    };
    append(plan, &p, None)?;
    Ok(p)
}
pub fn show(store: &Store, id: &str) -> Result<Proposal> {
    let _lock = lock_store(store)?;
    Ok(read(store, id)?.1)
}
/// This capability is deliberately absent from MCP. CLI must explicitly confirm summary_hash.
pub fn review(
    store: &Store,
    id: &str,
    action: &str,
    confirmed_hash: &str,
    clock: &impl Clock,
) -> Result<Proposal> {
    ensure!(
        ["approve", "reject", "undo"].contains(&action),
        "Invalid review action"
    );
    let _lock = lock_store(store)?;
    let (raw, mut p) = read(store, id)?;
    ensure!(confirmed_hash == p.summary_hash, "APPROVAL_HASH_REQUIRED");
    if (action == "approve" && p.state == "applied")
        || (action == "reject" && p.state == "rejected")
        || (action == "undo" && p.state == "undone")
    {
        return Ok(p);
    }
    if action == "reject" {
        ensure!(p.state == "pending", "Proposal not pending");
        p.state = "rejected".into();
        let mut plan = MutationPlan { changes: vec![] };
        append(&mut plan, &p, Some(raw))?;
        execute_mutation(store, plan)?;
        return Ok(p);
    }
    let undo = action == "undo";
    ensure!(
        if undo {
            p.state == "applied"
        } else {
            p.state == "pending"
        },
        "Proposal not applicable"
    );
    let memories = load_memories_unlocked(store, true)?;
    let mut ids = std::collections::HashSet::new();
    ensure!(
        memories.iter().all(|(_, memory)| ids.insert(&memory.id)),
        "Duplicate memory ID"
    );
    let target_ids: std::collections::HashSet<_> = p
        .request
        .targets
        .iter()
        .map(|target| target.memory_ref.memory_id.as_str())
        .collect();
    let dependents = memories
        .iter()
        .filter(|(_, memory)| {
            !target_ids.contains(memory.id.as_str())
                && memory
                    .links
                    .iter()
                    .any(|link| target_ids.contains(link.id.as_str()))
        })
        .map(|(_, memory)| Dependent {
            memory_id: memory.id.clone(),
            semantic_hash: revisions::semantic_digest(memory),
        })
        .collect::<Vec<_>>();
    if undo {
        ensure!(
            dependents.len() == p.dependents.len()
                && dependents.iter().all(|now| p
                    .dependents
                    .iter()
                    .any(|old| old.memory_id == now.memory_id
                        && old.semantic_hash == now.semantic_hash)),
            "UNDO_CONFLICT: external dependency changed"
        );
    }
    let mut updates = vec![];
    let mut before = vec![];
    for (i, t) in p.request.targets.iter().enumerate() {
        let current = memories
            .iter()
            .find(|(_, m)| m.id == t.memory_ref.memory_id);
        let expected = if undo {
            p.applied
                .get(i)
                .map(|s| (s.semantic_rev, s.semantic_hash.as_str()))
        } else {
            Some((t.expected_rev, t.expected_hash.as_str()))
        };
        let found = current
            .map(|(path, m)| -> Result<_> { Ok((path, m, revisions::snapshot(store, path)?)) })
            .transpose()?;
        if found
            .as_ref()
            .is_none_or(|(_, _, s)| expected != Some((s.semantic_rev, s.semantic_hash.as_str())))
        {
            if undo {
                bail!("UNDO_CONFLICT: target changed")
            }
            p.state = "stale".into();
            let mut plan = MutationPlan { changes: vec![] };
            append(&mut plan, &p, Some(raw))?;
            execute_mutation(store, plan)?;
            return Ok(p);
        }
        let (path, m, s) = found.unwrap();
        let mut desired = if undo {
            p.before
                .get(i)
                .context("Missing compensation image")?
                .clone()
        } else {
            m.clone()
        };
        if !undo && let Some(body) = &t.body {
            desired.body = body.clone();
            let summary = body
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(220)
                .collect::<String>();
            desired.canonical_summary = summary.clone();
            desired.injection_summary = summary;
        }
        // Compensation preserves current usage and never removes source evidence.
        desired.strength = m.strength;
        desired.access_count = m.access_count;
        desired.last_accessed = m.last_accessed.clone();
        before.push(m.clone());
        updates.push(RevisionUpdate {
            path: path.clone(),
            memory: desired,
            expected: Some(s),
        });
    }
    if !undo && let Some(rel) = relation(&p.request) {
        let source = p.request.targets[0].memory_ref.memory_id.as_str();
        let target = p.request.targets[1].memory_ref.memory_id.as_str();
        if rel == "supersedes" {
            check_supersede_cycle(&memories, source, target)?;
        }
        let added = add_link(&mut updates[0].memory, target, rel);
        add_link(
            &mut updates[1].memory,
            source,
            crate::relations::reverse(rel).unwrap(),
        );
        if rel == "supersedes" && added {
            let replaced = &mut updates[1].memory;
            replaced.strength = (replaced.strength - 20).max(0);
            replaced.status = "superseded".into();
            replaced
                .extra
                .insert("invalidated_by".into(), serde_json::json!(source));
        }
    }
    let mut plan = revisions::plan_updates(
        store,
        &updates,
        if undo {
            "proposal_undo"
        } else {
            "proposal_apply"
        },
        clock,
    )?;
    if !undo {
        p.before = before;
        p.dependents = dependents;
        p.approved_at = Some(clock.now().to_rfc3339());
        p.approved_summary_hash = Some(confirmed_hash.to_owned());
        p.applied = updates
            .iter()
            .map(|u| {
                let mut s = u.expected.clone().unwrap();
                let h = revisions::semantic_digest(&u.memory);
                if h != s.semantic_hash {
                    s.semantic_rev += 1;
                }
                s.semantic_hash = h;
                s.raw_hash = None;
                s
            })
            .collect();
    }
    p.state = if undo { "undone" } else { "applied" }.into();
    append(&mut plan, &p, Some(raw))?;
    execute_mutation(store, plan)?;
    crate::api::update_markdown_index(store, None)?;
    Ok(p)
}
