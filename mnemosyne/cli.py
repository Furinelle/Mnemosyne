"""Argparse command line interface for Mnemosyne."""

from __future__ import annotations

import argparse
from contextlib import suppress
import json
import os
import random
import sys
from datetime import date
from pathlib import Path

import portalocker

from mnemosyne.lifecycle import MaintainSummary, maintain_memory
from mnemosyne.schema import Memory, serialize_memory
from mnemosyne.search import BM25, SearchDocument, memory_search_text
from mnemosyne.store import (
    lock_store,
    Store,
    ensure_store,
    find_memory,
    global_store,
    load_config,
    load_memories,
    project_store,
    read_core,
    stores_for_scope,
    template_text,
    working_path,
    write_memory,
)


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if not hasattr(args, "func"):
        parser.print_help()
        return 0
    try:
        return args.func(args)
    except KeyboardInterrupt:
        print("Interrupted.", file=sys.stderr)
        return 130


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="mnemosyne")
    subparsers = parser.add_subparsers(dest="command")

    init_parser = subparsers.add_parser("init", help="Create .mnemosyne/ in the current directory")
    init_parser.set_defaults(func=cmd_init)

    read_parser = subparsers.add_parser("read", help="Print core memory for prompt injection")
    read_parser.add_argument("--scope", choices=["global", "project", "all"], default="project")
    read_parser.set_defaults(func=cmd_read)

    write_parser = subparsers.add_parser("write", help="Write a memory")
    write_parser.add_argument("--type", required=True)
    write_parser.add_argument("--importance", type=int, required=True)
    write_parser.add_argument("--scope", choices=["global", "project"], default="project")
    write_parser.add_argument("--source", default="agent")
    write_parser.add_argument("--tags", default="")
    write_parser.add_argument("--title", default="")
    write_parser.add_argument("--content", default="")
    write_parser.add_argument("--expires", default="")
    write_parser.add_argument("--force", action="store_true")
    write_parser.set_defaults(func=cmd_write)

    search_parser = subparsers.add_parser("search", help="Search memories")
    search_parser.add_argument("query")
    search_parser.add_argument("--scope", choices=["global", "project", "all"], default="all")
    search_parser.add_argument("--type", default="")
    search_parser.add_argument("--limit", type=int, default=5)
    search_parser.add_argument("--format", choices=["text", "json"], default="text")
    search_parser.add_argument("--archive", action="store_true", help="include archive")
    search_parser.set_defaults(func=cmd_search)

    maintain_parser = subparsers.add_parser("maintain", help="Decay and archive memories")
    maintain_parser.add_argument("--scope", choices=["global", "project", "all"], default="all")
    maintain_parser.add_argument("--dry-run", action="store_true")
    maintain_parser.set_defaults(func=cmd_maintain)

    show_parser = subparsers.add_parser("show", help="Show a memory by ID")
    show_parser.add_argument("id")
    show_parser.set_defaults(func=cmd_show)

    link_parser = subparsers.add_parser("link", help="Link two memories")
    link_parser.add_argument("id1")
    link_parser.add_argument("id2")
    link_parser.add_argument("--rel", default="related")
    link_parser.set_defaults(func=cmd_link)

    codex_prep_parser = subparsers.add_parser('codex-prep',
        help='Generate a prompt prefix for handoff to a non-Claude agent')
    codex_prep_parser.add_argument('task')
    codex_prep_parser.add_argument('--limit', type=int, default=5)
    codex_prep_parser.set_defaults(func=cmd_codex_prep)

    codex_ingest_parser = subparsers.add_parser('codex-ingest',
        help='Parse Findings blocks from stdin and write them as memories')
    codex_ingest_parser.add_argument('--source', default='codex')
    codex_ingest_parser.add_argument('--commit', action='store_true',
        help='actually write (default: dry-run preview)')
    codex_ingest_parser.set_defaults(func=cmd_codex_ingest)

    return parser


def cmd_init(args: argparse.Namespace) -> int:
    store = Store("project", Path.cwd() / ".mnemosyne")
    ensure_store(store, template_text("core_project.md"))
    print("Mnemosyne initialized. Add .mnemosyne/ to .gitignore or commit it.")
    agents_path = Path.cwd() / "AGENTS.md"
    if not agents_path.exists():
        try:
            agents_path.write_text(template_text("AGENTS.md"), encoding="utf-8")
            print(f"Wrote {agents_path}")
        except FileNotFoundError:
            pass
    templates_dir = Path(__file__).resolve().parent.parent / "templates"
    settings_path = templates_dir / "settings.json"
    print()
    print("Next steps:")
    print("  1. Ensure mnemosyne is importable from any cwd (recommended):")
    print("       pip install -e .")
    print("  2. To enable Claude Code auto-injection, merge this hooks config")
    print(f"     into ~/.claude/settings.json or .claude/settings.json:")
    print(f"       {settings_path}")
    return 0


def cmd_read(args: argparse.Namespace) -> int:
    for store in stores_for_scope(args.scope):
        label = "Global Core Memory" if store.scope == "global" else "Project Core Memory"
        print(f"===== {label}: {store.root} =====")
        content = read_core(store)
        print(content.rstrip() if content else "(empty)")
        print()
    return 0


def cmd_write(args: argparse.Namespace) -> int:
    store = global_store() if args.scope == "global" else project_store()
    core_template = template_text("core_global.md") if args.scope == "global" else template_text("core_project.md")
    ensure_store(store, core_template)

    config = load_config(store)
    allowed_types = config["memory"].get("types", [])
    if allowed_types and args.type not in allowed_types:
        print(f"Warning: type '{args.type}' is not in configured memory.types.", file=sys.stderr)

    content = args.content if args.content else sys.stdin.read()
    content = content.strip()
    if not content:
        print("No content provided.", file=sys.stderr)
        return 2

    title = args.title.strip() or first_line_title(content)
    today = date.today().isoformat()
    memory_id = make_memory_id(args.type, today)
    tags = [tag.strip() for tag in args.tags.split(",") if tag.strip()]
    summary = summarize(title, content)
    memory = Memory(
        id=memory_id,
        type=args.type,
        source=args.source,
        strength=min(100, args.importance),
        created=today,
        last_accessed=today,
        access_count=0,
        tags=tags,
        links=[],
        canonical_summary=summary,
        injection_summary=summary,
        status="active",
        body=f"## {title}\n\n{content}",
        expires=args.expires,
    )

    if not args.force and sys.stdin.isatty():
        duplicate = duplicate_prompt(store, memory)
        if duplicate == "cancel":
            print("Cancelled.")
            return 1
        if duplicate and duplicate != "none":
            duplicate_path, duplicate_memory = duplicate
            merge_memory(duplicate_memory, memory)
            write_memory(duplicate_path, duplicate_memory)
            print(f"Merged into {duplicate_memory.id}")
            return 0

    path = working_path(store, memory)
    write_memory(path, memory)
    update_memory_index(store, memory)
    print(f"Wrote {memory.id}")
    return 0


def cmd_search(args: argparse.Namespace) -> int:
    stores = stores_for_scope(args.scope)
    config = load_config(stores[-1] if stores else None)
    documents: list[SearchDocument] = []
    path_lookup: dict[str, tuple[Store, Path, Memory]] = {}
    for store in stores:
        for path, memory in load_memories(store, include_archive=args.archive):
            if args.type and memory.type != args.type:
                continue
            document_id = f"{store.scope}:{memory.id}"
            documents.append(SearchDocument(document_id, memory_search_text(memory), memory))
            path_lookup[document_id] = (store, path, memory)

    results = BM25(documents).search(args.query, args.limit)
    if not results:
        if args.format == "json":
            print("[]")
        else:
            print("no results")
        return 0

    threshold_bonus = int(config["thresholds"].get("bonus_access", 5))
    recall_bonus = int(config["thresholds"].get("bonus_recall", 20))
    output = []
    for result in results:
        store, path, memory = path_lookup[result.document.id]
        memory.access_count += 1
        memory.last_accessed = date.today().isoformat()
        is_archive = "archive" in str(path)
        bonus = recall_bonus if is_archive else threshold_bonus
        memory.strength = min(100, memory.strength + bonus)
        with suppress(portalocker.exceptions.LockException):
            write_memory(path, memory, lock_timeout=0)
        output.append(
            {
                "id": memory.id,
                "scope": store.scope,
                "type": memory.type,
                "score": round(result.score, 4),
                "strength": memory.strength,
                "tags": memory.tags,
                "links": memory.links,
                "summary": memory.injection_summary,
                "path": str(path),
            }
        )

    if args.format == "json":
        print(json.dumps(output, ensure_ascii=False, indent=2))
    else:
        for item in output:
            print(f"[{item['score']:.4f}] {item['id']} ({item['scope']}/{item['type']})")
            print(item["summary"])
            print(f"path: {item['path']}")
            print()
    return 0


def cmd_maintain(args: argparse.Namespace) -> int:
    summary = MaintainSummary()
    stores = stores_for_scope(args.scope)
    for store in stores:
        with lock_store(store):
            config = load_config(store)
            for path, memory in load_memories(store, include_archive=False):
                summary.processed += 1
                result, candidate = maintain_memory(
                    store,
                    path,
                    memory,
                    config["thresholds"],
                    dry_run=args.dry_run,
                )
                summary.decayed += 1
                if result == "deprecated":
                    summary.deprecated += 1
                elif result == "archived":
                    summary.archived += 1
                elif result == "core_candidate" and candidate is not None:
                    summary.core_candidates.append(candidate)

    print(f"processed: {summary.processed}")
    print(f"decayed: {summary.decayed}")
    print(f"deprecated: {summary.deprecated}")
    print(f"archived: {summary.archived}")
    if summary.core_candidates:
        print("Core candidates:")
        for memory in summary.core_candidates:
            print(f"- {memory.id}: {memory.injection_summary}")
    return 0


def cmd_show(args: argparse.Namespace) -> int:
    found = find_memory(args.id, stores_for_scope("all"), include_archive=True)
    if found is None:
        print(f"Memory not found: {args.id}", file=sys.stderr)
        return 1
    _, _, memory = found
    print(serialize_memory(memory).rstrip())
    return 0


def cmd_link(args: argparse.Namespace) -> int:
    stores = stores_for_scope("all")
    first = find_memory(args.id1, stores, include_archive=True)
    second = find_memory(args.id2, stores, include_archive=True)
    if first is None:
        print(f"Memory not found: {args.id1}", file=sys.stderr)
        return 1
    if second is None:
        print(f"Memory not found: {args.id2}", file=sys.stderr)
        return 1
    _, first_path, first_memory = first
    _, second_path, second_memory = second

    add_link(first_memory, second_memory.id, args.rel)
    add_link(second_memory, first_memory.id, args.rel)
    write_memory(first_path, first_memory)
    write_memory(second_path, second_memory)
    print(f"Linked {first_memory.id} <-> {second_memory.id} ({args.rel})")
    return 0


def cmd_codex_prep(args: argparse.Namespace) -> int:
    from mnemosyne.codex import prep
    print(prep(args.task, max_memories=args.limit))
    return 0


def cmd_codex_ingest(args: argparse.Namespace) -> int:
    from mnemosyne.codex import ingest
    text = sys.stdin.read()
    if not text.strip():
        print('No input on stdin.', file=sys.stderr)
        return 2
    actions = ingest(text, source=args.source, commit=args.commit)
    if not actions:
        print('No findings parsed.')
        return 0
    verb = 'wrote' if args.commit else 'would write'
    for action in actions:
        ident = action.get('id', '<dry-run>')
        print(f"{verb} {ident} ({action['type']}, importance {action['importance']}): {action['title']}")
        if action['tags']:
            print(f"  tags: {', '.join(action['tags'])}")
        print(f"  preview: {action['content_preview']}")
    if not args.commit:
        print()
        print('(dry-run) Add --commit to actually write.')
    return 0


def make_memory_id(memory_type: str, day: str) -> str:
    suffix = "".join(random.choice("0123456789abcdef") for _ in range(6))
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


def duplicate_prompt(store: Store, memory: Memory) -> str | tuple[Path, Memory]:
    path_lookup: dict[str, tuple[Path, Memory]] = {}
    documents = []
    for path, existing in load_memories(store, include_archive=False):
        documents.append(SearchDocument(existing.id, memory_search_text(existing), existing))
        path_lookup[existing.id] = (path, existing)
    results = BM25(documents).search(memory_search_text(memory), 3)
    matches = [result for result in results if result.score > 4.0]
    if not matches:
        return "none"
    print("Possible duplicate memories:")
    for result in matches:
        existing = result.document.payload
        print(f"- {existing.id}: {getattr(existing, 'injection_summary', '')} ({result.score:.4f})")
    answer = input("Merge into top match? [y/N/c] ").strip().lower()
    if answer in {"y", "yes", "m", "merge"}:
        return path_lookup[matches[0].document.id]
    if answer in {"c", "cancel"}:
        return "cancel"
    return "none"


def update_memory_index(store: Store, memory: Memory) -> None:
    index_path = store.root / "MEMORY.md"
    if not index_path.exists():
        try:
            index_path.write_text(template_text("MEMORY.md"), encoding="utf-8")
        except FileNotFoundError:
            index_path.write_text("# Memory Index\n", encoding="utf-8")
    with index_path.open("a", encoding="utf-8") as handle:
        handle.write(f"\n- `{memory.id}` ({memory.type}, strength {memory.strength}): {memory.injection_summary}\n")


def add_link(memory: Memory, link_id: str, rel: str) -> None:
    if any(link.get("id") == link_id and link.get("rel") == rel for link in memory.links):
        return
    memory.links.append({"id": link_id, "rel": rel})


def merge_memory(existing: Memory, incoming: Memory) -> None:
    existing.strength = max(existing.strength, incoming.strength)
    existing.last_accessed = date.today().isoformat()
    existing.access_count += 1
    for tag in incoming.tags:
        if tag not in existing.tags:
            existing.tags.append(tag)
    if incoming.body and incoming.body not in existing.body:
        existing.body = existing.body.rstrip() + "\n\n" + incoming.body.strip()
    if incoming.canonical_summary and incoming.canonical_summary not in existing.canonical_summary:
        existing.canonical_summary = (existing.canonical_summary + " " + incoming.canonical_summary).strip()
    if incoming.injection_summary and incoming.injection_summary not in existing.injection_summary:
        existing.injection_summary = (existing.injection_summary + " " + incoming.injection_summary).strip()


if __name__ == "__main__":
    raise SystemExit(main())
