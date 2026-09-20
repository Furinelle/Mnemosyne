//! Deterministic, non-semantic source indexes. Views are never memories.
use crate::{
    provenance::{Clock, MemoryRef, read_manifest},
    revisions,
    schema::{Memory, is_expired},
    store::{Store, load_memories_unlocked, lock_store},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
};

const VERSION: u32 = 1;
const MAX_VIEW_BYTES: u64 = 1024 * 1024;
const MARKER: &str = "<!-- mnemosyne-view\n";
const END_MARKER: &str = "\n-->\n";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ViewSourceRef {
    pub memory_ref: MemoryRef,
    pub revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewRequest {
    pub name: String,
    pub references: Vec<ViewSourceRef>,
    pub allow_partial: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViewMetadata {
    pub version: u32,
    pub name: String,
    pub generated_at: String,
    pub generation: String,
    pub coverage: String,
    pub derived_from: Vec<ViewSourceRef>,
    pub known_gaps: Vec<String>,
    pub managed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ViewOutcome {
    pub status: String,
    pub path: PathBuf,
    pub metadata: ViewMetadata,
    pub stale: bool,
    /// A view is an index over existing evidence, never an additional source.
    pub evidence_count: usize,
}

#[derive(Clone)]
struct Source {
    reference: ViewSourceRef,
    title: String,
}

fn digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn safe_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 80
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "invalid view name"
    );
    Ok(())
}

fn views_dir(store: &Store) -> Result<PathBuf> {
    ensure!(
        !store.root.exists() || !fs::symlink_metadata(&store.root)?.file_type().is_symlink(),
        "symlink store root"
    );
    let path = store.root.join("views");
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "invalid views directory"
        );
    }
    Ok(path)
}

fn view_path(store: &Store, name: &str) -> Result<PathBuf> {
    safe_name(name)?;
    let path = views_dir(store)?.join(format!("{name}.md"));
    ensure!(
        path.strip_prefix(&store.root)?
            .components()
            .all(|part| matches!(part, Component::Normal(_))),
        "unsafe view path"
    );
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "invalid view file"
        );
    }
    Ok(path)
}

fn read_view(path: &Path) -> Result<Option<String>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "invalid view file"
            );
            ensure!(metadata.len() <= MAX_VIEW_BYTES, "VIEW_TOO_LARGE");
            Ok(Some(String::from_utf8(crate::input::read_bytes(
                File::open(path)?,
                MAX_VIEW_BYTES as usize,
            )?)?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn parse_view(text: &str) -> Result<(ViewMetadata, &str)> {
    ensure!(
        text.starts_with(MARKER),
        "VIEW_UNMANAGED: missing view metadata"
    );
    let rest = &text[MARKER.len()..];
    let (raw, body) = rest
        .split_once(END_MARKER)
        .context("invalid view metadata")?;
    let metadata: ViewMetadata = serde_json::from_str(raw)?;
    ensure!(metadata.version == VERSION, "unsupported view version");
    Ok((metadata, body))
}

fn valid_image(metadata: &ViewMetadata, body: &str) -> Result<bool> {
    let Some(actual) = &metadata.integrity else {
        return Ok(false);
    };
    let mut expected = metadata.clone();
    expected.integrity = None;
    Ok(actual == &digest(&[serde_json::to_vec(&expected)?, body.as_bytes().to_vec()].concat()))
}

fn checked_existing(path: &Path) -> Result<()> {
    let Some(text) = read_view(path)? else {
        return Ok(());
    };
    let (metadata, body) = parse_view(&text)?;
    ensure!(
        metadata.managed,
        "VIEW_UNMANAGED: refusing to overwrite user-owned view"
    );
    ensure!(
        valid_image(&metadata, body)?,
        "VIEW_CONFLICT: managed view changed outside Mnemosyne"
    );
    Ok(())
}

fn active(memory: &Memory) -> bool {
    (memory.status.is_empty() || memory.status == "active")
        && !is_expired(&memory.expires)
        && memory
            .extra
            .get("invalidated_by")
            .is_none_or(|value| value.as_str().is_none_or(str::is_empty))
}

fn source(store: &Store, reference: &ViewSourceRef) -> Result<Source> {
    let manifest = read_manifest(store)?.context("views require an upgraded store")?;
    ensure!(
        reference.memory_ref.store_id == manifest.store_id,
        "VIEW_SOURCE_STORE_MISMATCH"
    );
    let (path, memory) = load_memories_unlocked(store, true)?
        .into_iter()
        .find(|(_, memory)| memory.id == reference.memory_ref.memory_id)
        .context("VIEW_SOURCE_MISSING")?;
    ensure!(active(&memory), "VIEW_SOURCE_INACTIVE");
    let snapshot = revisions::snapshot(store, &path)?;
    ensure!(
        snapshot.semantic_rev == reference.revision,
        "VIEW_SOURCE_STALE: requested source revision is not current"
    );
    Ok(Source {
        reference: reference.clone(),
        title: label(&memory.title()),
    })
}

fn label(value: &str) -> String {
    let one_line = value.split_whitespace().collect::<Vec<_>>().join(" ");
    one_line.chars().take(200).collect()
}

fn title(name: &str) -> String {
    let mut title = String::new();
    let mut upper = true;
    for character in name.chars() {
        if character == '-' || character == '_' {
            title.push(' ');
            upper = true;
        } else if upper {
            title.extend(character.to_uppercase());
            upper = false;
        } else {
            title.push(character);
        }
    }
    title
}

fn render(metadata: &ViewMetadata, sources: &[Source]) -> Result<String> {
    let mut body = format!(
        "# {} index\n\nGenerated offline from explicit current source revisions. This is an index, not a semantic synthesis and not independent evidence.\n\n## Sources\n",
        title(&metadata.name)
    );
    if sources.is_empty() {
        body.push_str("\n_No current sources were available._\n");
    } else {
        for source in sources {
            body.push_str(&format!(
                "\n- {} (`{}/{}` revision {})",
                source.title,
                source.reference.memory_ref.store_id,
                source.reference.memory_ref.memory_id,
                source.reference.revision
            ));
        }
        body.push('\n');
    }
    if !metadata.known_gaps.is_empty() {
        body.push_str("\n## Known gaps\n");
        for gap in &metadata.known_gaps {
            body.push_str(&format!("\n- {}", label(gap)));
        }
        body.push('\n');
    }
    image(metadata, &body)
}

fn image(metadata: &ViewMetadata, body: &str) -> Result<String> {
    let mut metadata = metadata.clone();
    metadata.integrity = None;
    metadata.integrity = Some(digest(
        &[serde_json::to_vec(&metadata)?, body.as_bytes().to_vec()].concat(),
    ));
    let text = format!(
        "{MARKER}{}{}{}",
        serde_json::to_string(&metadata)?,
        END_MARKER,
        body
    );
    ensure!(text.len() as u64 <= MAX_VIEW_BYTES, "VIEW_TOO_LARGE");
    Ok(text)
}

fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let parent = path.parent().context("view path has no parent")?;
    if !parent.exists() {
        fs::create_dir(parent)?;
    }
    ensure!(
        !fs::symlink_metadata(parent)?.file_type().is_symlink(),
        "symlink views directory"
    );
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap().to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        file.write_all(text.as_bytes())?;
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

/// Generate a non-semantic index from current, explicit source revisions.
pub fn generate(store: &Store, request: &ViewRequest, clock: &impl Clock) -> Result<ViewOutcome> {
    let _lock = lock_store(store)?;
    let path = view_path(store, &request.name)?;
    checked_existing(&path)?;
    let mut sources = Vec::new();
    let mut gaps = Vec::new();
    let mut seen = HashSet::new();
    for reference in &request.references {
        if !seen.insert((
            reference.memory_ref.store_id.clone(),
            reference.memory_ref.memory_id.clone(),
            reference.revision,
        )) {
            continue;
        }
        match source(store, reference) {
            Ok(source) => sources.push(source),
            Err(error) if request.allow_partial => gaps.push(format!(
                "{}/{} revision {}: {}",
                reference.memory_ref.store_id,
                reference.memory_ref.memory_id,
                reference.revision,
                error
            )),
            Err(error) => return Err(error),
        }
    }
    let metadata = ViewMetadata {
        version: VERSION,
        name: request.name.clone(),
        generated_at: clock.now().to_rfc3339(),
        generation: "offline_template_index".into(),
        coverage: "explicit_current_revisions".into(),
        derived_from: sources
            .iter()
            .map(|source| source.reference.clone())
            .collect(),
        known_gaps: gaps,
        managed: true,
        integrity: None,
    };
    let text = render(&metadata, &sources)?;
    let (metadata, _) = parse_view(&text)?;
    write_atomic(&path, &text)?;
    let stale = !metadata.known_gaps.is_empty();
    Ok(ViewOutcome {
        status: if stale { "partial" } else { "current" }.into(),
        path,
        metadata,
        stale,
        evidence_count: 0,
    })
}

/// Preserve a checked view as user-owned content. It will never be regenerated.
pub fn unmanage(store: &Store, name: &str) -> Result<ViewOutcome> {
    let _lock = lock_store(store)?;
    let path = view_path(store, name)?;
    let text = read_view(&path)?.context("VIEW_NOT_FOUND")?;
    let (mut metadata, body) = parse_view(&text)?;
    ensure!(
        valid_image(&metadata, body)?,
        "VIEW_CONFLICT: managed view changed outside Mnemosyne"
    );
    metadata.managed = false;
    let text = image(&metadata, body)?;
    let (metadata, _) = parse_view(&text)?;
    write_atomic(&path, &text)?;
    Ok(ViewOutcome {
        status: "unmanaged".into(),
        path,
        metadata,
        stale: true,
        evidence_count: 0,
    })
}

/// Read a view and determine whether its recorded source revisions remain current.
pub fn inspect(store: &Store, name: &str) -> Result<ViewOutcome> {
    let _lock = lock_store(store)?;
    let path = view_path(store, name)?;
    let text = read_view(&path)?.context("VIEW_NOT_FOUND")?;
    let (metadata, body) = parse_view(&text)?;
    ensure!(
        valid_image(&metadata, body)?,
        "VIEW_CONFLICT: managed view changed outside Mnemosyne"
    );
    let stale = stale_unlocked(store, &metadata)?;
    Ok(ViewOutcome {
        status: if stale {
            "stale"
        } else if metadata.known_gaps.is_empty() {
            "current"
        } else {
            "partial"
        }
        .into(),
        path,
        metadata,
        stale,
        evidence_count: 0,
    })
}

/// Stale views must not be treated as current knowledge or independent evidence.
pub fn is_stale(store: &Store, metadata: &ViewMetadata) -> Result<bool> {
    let _lock = lock_store(store)?;
    stale_unlocked(store, metadata)
}

fn stale_unlocked(store: &Store, metadata: &ViewMetadata) -> Result<bool> {
    if !metadata.managed || metadata.generation != "offline_template_index" {
        return Ok(true);
    }
    for reference in &metadata.derived_from {
        if source(store, reference).is_err() {
            return Ok(true);
        }
    }
    Ok(false)
}
