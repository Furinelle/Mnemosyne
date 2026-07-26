"""Stable structured API consumed by the CLI, MCP server, and adapters.

Functions here return dataclasses/dicts instead of printing; the CLI turns
them into human-readable text and the MCP server into JSON-RPC payloads.
Symbols exported here (plus their `mnemosyne.cli` re-exports) are the public
Python surface with a compatibility promise.
"""

from __future__ import annotations

import uuid
from contextlib import suppress
from dataclasses import dataclass
from datetime import date

import portalocker

from mnemosyne.fusion import search as fusion_search
from mnemosyne.index import (
    index_enabled,
    index_path,
    reindex_store,
    update_memory_index as update_search_index,
)
from mnemosyne.findings import Finding
from mnemosyne.lifecycle import MaintainSummary, maintain_memory
from mnemosyne.relations import PREDEFINED, is_demoting, reverse
from mnemosyne.schema import Memory
from mnemosyne.store import (
    Store,
    bump_memory_access,
    ensure_store,
    find_memory,
    global_store,
    load_config,
    load_memories,
    lock_store,
    lock_stores,
    project_store,
    stores_for_scope,
    template_text,
    working_path,
    write_memory,
)


class MnemosyneError(RuntimeError):
    """Typed failure for API consumers; exit_code maps to the CLI convention."""

    def __init__(self, message: str, exit_code: int = 1):
        super().__init__(message)
        self.exit_code = exit_code


@dataclass
class WriteResult:
    status: str                    # "created" | "duplicate"
    id: str                        # new memory id, or the duplicate target's id
    duplicate_of: str | None = None
    superseded: str | None = None  # id of the memory the new one supersedes
    path: str = ""


# Strength penalty applied to the target of a demoting relation (e.g. the
# memory that gets superseded). Activates the demote_target relation semantics.
DEMOTE_ON_SUPERSEDE = 20


def make_memory_id(memory_type: str, day: str) -> str:
    # 8 hex chars (32 bits) instead of 6 (24 bits): the old space made silent
    # filename collisions (and os.replace overwrites) plausible across many
    # same-day writes. uuid4 is a stronger entropy source than random.choice.
    suffix = uuid.uuid4().hex[:8]
    return f"{memory_type}-{day}-{suffix}"


def summarize(title: str, content: str) -> str:
    text = " ".join(content.split())
    if len(text) > 220:
        text = text[:217].rstrip() + "..."
    return f"{title}: {text}" if title else text


def first_line_title(content: str) -> str:
    for line in content.splitlines():
        stripped = line.strip().lstrip("#").strip()
        if stripped:
            return stripped[:80]
    return "Untitled"


def add_link(memory: Memory, link_id: str, rel: str) -> None:
    if any(link.get("id") == link_id and link.get("rel") == rel for link in memory.links):
        return
    memory.links.append({"id": link_id, "rel": rel})


def update_memory_index_file(store: Store, memory: Memory) -> None:
    index_file = store.root / "MEMORY.md"
    if not index_file.exists():
        try:
            index_file.write_text(template_text("MEMORY.md"), encoding="utf-8")
        except FileNotFoundError:
            index_file.write_text("# Memory Index\n", encoding="utf-8")
    with index_file.open("a", encoding="utf-8") as handle:
        handle.write(f"\n- `{memory.id}` ({memory.type}, strength {memory.strength}): {memory.injection_summary}\n")


def rewrite_memory_index_file(store: Store) -> None:
    """Regenerate MEMORY.md from active working memories.

    The write path appends for cheapness; maintain reconciles so entries for
    archived or superseded memories do not accumulate forever.
    """
    try:
        header = template_text("MEMORY.md").rstrip()
    except (FileNotFoundError, OSError):
        header = "# Memory Index"
    lines = [header, ""]
    memories = sorted(load_memories(store), key=lambda pair: pair[1].strength, reverse=True)
    for _path, memory in memories:
        lines.append(f"- `{memory.id}` ({memory.type}, strength {memory.strength}): {memory.injection_summary}")
    (store.root / "MEMORY.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def write_entry(
    *,
    type: str,
    importance: int,
    content: str,
    title: str = "",
    tags: list[str] | None = None,
    scope: str = "project",
    source: str = "agent",
    expires: str = "",
    allow_duplicate: bool = False,
) -> WriteResult:
    """Classify-and-write one memory as a single locked transaction."""
    store = global_store() if scope == "global" else project_store()
    core_template = template_text("core_global.md") if scope == "global" else template_text("core_project.md")
    ensure_store(store, core_template)
    config = load_config(store)

    content = content.strip()
    if not content:
        raise MnemosyneError("No content provided.", exit_code=2)
    title = (title or "").strip() or first_line_title(content)
    today = date.today().isoformat()
    tag_list = [str(tag) for tag in (tags or [])]
    summary = summarize(title, content)
    memory = Memory(
        id=make_memory_id(type, today),
        type=type,
        source=source,
        strength=max(0, min(100, importance)),
        created=today,
        last_accessed=today,
        access_count=0,
        tags=tag_list,
        links=[],
        canonical_summary=summary,
        injection_summary=summary,
        status="active",
        body=f"## {title}\n\n{content}",
        expires=expires,
    )

    from mnemosyne.distill import _apply_supersedes, classify_against_store

    finding = Finding(type=type, importance=memory.strength, title=title, tags=tag_list, content=content)
    distill_cfg = config.get("distill", {})
    with lock_store(store):
        # Classification and creation are one transaction, so simultaneous
        # writers cannot both observe an empty destination and create copies.
        verdict, target = classify_against_store(
            finding,
            store=store,
            dedup_threshold=float(distill_cfg.get("dedup_threshold", 0.85)),
            subject_threshold=float(distill_cfg.get("subject_threshold", 0.5)),
        )
        if verdict == "duplicate" and target and not allow_duplicate:
            return WriteResult(status="duplicate", id=target, duplicate_of=target)
        path = working_path(store, memory)
        write_memory(path, memory)
        update_memory_index_file(store, memory)
        update_search_index(store, path, memory)
        if verdict == "supersede" and target:
            _apply_supersedes(memory.id, target, stores=[store], _locked=True)
    return WriteResult(
        status="created",
        id=memory.id,
        superseded=target if verdict == "supersede" else None,
        path=str(path),
    )


def results_to_dicts(indexed_results, config: dict, *, update_access: bool = True) -> list[dict]:
    """Turn fusion search results into the stable search-result dict shape."""
    threshold_bonus = int(config["thresholds"].get("bonus_access", 5))
    recall_bonus = int(config["thresholds"].get("bonus_recall", 20))
    output: list[dict] = []
    today = date.today().isoformat()
    for result in indexed_results:
        memory = result.memory
        path = result.path
        is_archive = result.store.archive_dir in path.parents
        bonus = recall_bonus if is_archive else threshold_bonus
        if update_access:
            # OSError too: a concurrent `maintain` can archive (move) the file
            # between retrieval and the access bump. Failing to bump must not
            # take the whole search down with it.
            with suppress(portalocker.exceptions.LockException, OSError):
                memory = bump_memory_access(
                    result.store, path, bonus, today=today, lock_timeout=0, sync_index=True
                )
        output.append(
            {
                "id": memory.id,
                "scope": result.store.scope,
                "type": memory.type,
                "score": round(result.score, 4),
                "strength": memory.strength,
                "tags": memory.tags,
                "links": memory.links,
                "summary": memory.injection_summary,
                "path": str(path),
                "why_matched": result.why_matched,
                "score_breakdown": result.score_breakdown,
            }
        )
    return output


def search_entries(
    query: str,
    *,
    scope: str = "all",
    type_filter: str = "",
    limit: int = 5,
    include_archive: bool = False,
    include_superseded: bool = False,
    update_access: bool = True,
    stores: list[Store] | None = None,
) -> list[dict]:
    selected = stores_for_scope(scope) if stores is None else list(stores)
    config = load_config(selected[-1] if selected else None)
    results = fusion_search(
        selected,
        query,
        limit=limit,
        type_filter=type_filter,
        include_archive=include_archive,
        include_superseded=include_superseded,
        config=config,
    )
    return results_to_dicts(results, config, update_access=update_access)


def maintain(*, scope: str = "all", dry_run: bool = False, stores: list[Store] | None = None) -> dict:
    summary = MaintainSummary()
    selected = stores_for_scope(scope) if stores is None else list(stores)
    for store in selected:
        with lock_store(store):
            config = load_config(store)
            for path, memory in load_memories(store, include_archive=False):
                summary.processed += 1
                result, candidate = maintain_memory(
                    store, path, memory, config["thresholds"], dry_run=dry_run
                )
                if result == "decayed":
                    summary.decayed += 1
                elif result == "deprecated":
                    summary.deprecated += 1
                elif result == "archived":
                    summary.archived += 1
                elif result == "core_candidate" and candidate is not None:
                    summary.core_candidates.append(candidate)
        if not dry_run:
            rewrite_memory_index_file(store)
        if index_enabled(store) and index_path(store).exists():
            reindex_store(store)
    return {
        "processed": summary.processed,
        "decayed": summary.decayed,
        "deprecated": summary.deprecated,
        "archived": summary.archived,
        "core_candidates": [
            {"id": memory.id, "summary": memory.injection_summary}
            for memory in summary.core_candidates
        ],
    }


def link_entries(
    id1: str,
    id2: str,
    rel: str = "related",
    allow_custom: bool = False,
    stores: list[Store] | None = None,
) -> dict:
    requested = stores_for_scope("all") if stores is None else list(stores)
    selected = [store for store in requested if store.root.exists()]
    with lock_stores(selected):
        # Resolve after locking: callers may have loaded either endpoint before
        # a concurrent access update or supersedence write completed.
        first = find_memory(id1, selected, include_archive=True)
        second = find_memory(id2, selected, include_archive=True)
        if first is None:
            raise MnemosyneError(f"Memory not found: {id1}")
        if second is None:
            raise MnemosyneError(f"Memory not found: {id2}")
        _, first_path, first_memory = first
        _, second_path, second_memory = second
        allow = bool(allow_custom or load_config(first[0]).get("relations", {}).get("allow_custom"))
        if rel not in PREDEFINED and not allow:
            choices = ", ".join(sorted(PREDEFINED))
            raise MnemosyneError(
                f"Unknown relation: {rel}. Choose one of: {choices}; use --allow-custom to opt in.",
                exit_code=2,
            )
        add_link(first_memory, second_memory.id, rel)
        add_link(second_memory, first_memory.id, reverse(rel) or rel)
        if is_demoting(rel):
            second_memory.strength = max(0, second_memory.strength - DEMOTE_ON_SUPERSEDE)
            second_memory.status = "superseded"
            second_memory.extra["invalidated_by"] = first_memory.id
        write_memory(first_path, first_memory)
        write_memory(second_path, second_memory)
        update_search_index(first[0], first_path, first_memory)
        update_search_index(second[0], second_path, second_memory)
    return {"ok": True, "rel": rel, "id1": first_memory.id, "id2": second_memory.id}
