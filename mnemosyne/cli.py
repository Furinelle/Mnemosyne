"""Argparse command line interface for Mnemosyne."""

from __future__ import annotations

import argparse
from contextlib import suppress
import importlib.util
import json
import os
import random
import sys
from datetime import date
from pathlib import Path

import portalocker

from mnemosyne.lifecycle import MaintainSummary, maintain_memory
from mnemosyne.index import (
    backfill_embeddings,
    fts_available,
    index_enabled,
    index_path,
    reindex_store,
    search_index,
    update_memory_index as update_search_index,
)
from mnemosyne.embedding import get_embedder
from mnemosyne.fusion import search as fusion_search
from mnemosyne.relations import PREDEFINED, reverse
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

    reindex_parser = subparsers.add_parser("reindex", help="Rebuild persistent search indexes")
    reindex_parser.add_argument("--scope", choices=["global", "project", "all"], default="all")
    reindex_parser.add_argument("--no-archive", action="store_true", help="exclude archive memories")
    reindex_parser.set_defaults(func=cmd_reindex)

    embed_parser = subparsers.add_parser("embed-backfill", help="Compute embeddings for existing memories")
    embed_parser.add_argument("--scope", choices=["global", "project", "all"], default="all")
    embed_parser.add_argument("--no-archive", action="store_true", help="exclude archive memories")
    embed_parser.set_defaults(func=cmd_embed_backfill)

    eval_parser = subparsers.add_parser("eval", help="Run retrieval quality evaluations")
    eval_parser.add_argument("eval_args", nargs=argparse.REMAINDER)
    eval_parser.set_defaults(func=cmd_eval)

    doctor_parser = subparsers.add_parser("doctor", help="Check Mnemosyne installation health")
    doctor_parser.add_argument("--scope", choices=["global", "project", "all"], default="all")
    doctor_parser.set_defaults(func=cmd_doctor)

    show_parser = subparsers.add_parser("show", help="Show a memory by ID")
    show_parser.add_argument("id")
    show_parser.set_defaults(func=cmd_show)

    link_parser = subparsers.add_parser("link", help="Link two memories")
    link_parser.add_argument("id1")
    link_parser.add_argument("id2")
    link_parser.add_argument("--rel", default="related")
    link_parser.add_argument("--allow-custom", action="store_true")
    link_parser.set_defaults(func=cmd_link)

    graph_parser = subparsers.add_parser("graph", help="Render linked memories as a graph")
    graph_parser.add_argument("id")
    graph_parser.add_argument("--depth", type=int, default=1)
    graph_parser.add_argument("--format", choices=["mermaid", "ascii", "json"], default="mermaid")
    graph_parser.set_defaults(func=cmd_graph)

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
    update_memory_index_file(store, memory)
    update_search_index(store, path, memory)
    print(f"Wrote {memory.id}")
    return 0


def cmd_search(args: argparse.Namespace) -> int:
    stores = stores_for_scope(args.scope)
    config = load_config(stores[-1] if stores else None)
    results = fusion_search(
        stores,
        args.query,
        limit=args.limit,
        type_filter=args.type,
        include_archive=args.archive,
        config=config,
    )
    if not results:
        if args.format == "json":
            print("[]")
        else:
            print("no results")
        return 0

    return print_search_results(results, args.format, config)


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
                if result == "decayed":
                    summary.decayed += 1
                elif result == "deprecated":
                    summary.deprecated += 1
                elif result == "archived":
                    summary.archived += 1
                elif result == "core_candidate" and candidate is not None:
                    summary.core_candidates.append(candidate)
        if index_enabled(store) and index_path(store).exists():
            reindex_store(store)

    print(f"processed: {summary.processed}")
    print(f"decayed: {summary.decayed}")
    print(f"deprecated: {summary.deprecated}")
    print(f"archived: {summary.archived}")
    if summary.core_candidates:
        print("Core candidates:")
        for memory in summary.core_candidates:
            print(f"- {memory.id}: {memory.injection_summary}")
    return 0


def cmd_reindex(args: argparse.Namespace) -> int:
    if not fts_available():
        print("SQLite FTS5 is not available in this Python build.", file=sys.stderr)
        return 1
    total = 0
    for store in stores_for_scope(args.scope):
        count = reindex_store(store, include_archive=not args.no_archive)
        total += count
        print(f"{store.scope}: indexed {count} memories at {index_path(store)}")
        embedder = get_embedder(load_config(store))
        if embedder.model_id != "none":
            embedded = backfill_embeddings(store, embedder, include_archive=not args.no_archive)
            print(f"{store.scope}: embedded {embedded} memories with {embedder.model_id}")
    print(f"total: {total}")
    return 0


def cmd_embed_backfill(args: argparse.Namespace) -> int:
    total = 0
    enabled = False
    for store in stores_for_scope(args.scope):
        embedder = get_embedder(load_config(store))
        if embedder.model_id == "none":
            print(f"{store.scope}: embedding is disabled", file=sys.stderr)
            continue
        enabled = True
        count = backfill_embeddings(store, embedder, include_archive=not args.no_archive)
        total += count
        print(f"{store.scope}: embedded {count} memories with {embedder.model_id}")
    print(f"total: {total}")
    return 0 if enabled else 1


def cmd_eval(args: argparse.Namespace) -> int:
    from mnemosyne.eval.__main__ import main as eval_main

    return eval_main(args.eval_args)


def cmd_doctor(args: argparse.Namespace) -> int:
    # Read-only health check: never creates stores or files as a side effect.
    # Tuple is (name, ok, detail, hard) where hard marks installation-level
    # checks that count toward the non-zero exit code.
    checks: list[tuple[str, bool, str, bool]] = []
    checks.append((
        "portalocker",
        importlib.util.find_spec("portalocker") is not None,
        "required for concurrent file locks",
        True,
    ))
    config = load_config()
    embedding = config.get("embedding", {})
    embedding_enabled = bool(embedding.get("enabled"))
    embedding_backend = str(embedding.get("backend", "onnx"))
    if not embedding_enabled:
        embedding_detail = "disabled"
    elif embedding_backend == "onnx":
        embedding_detail = "configured" if importlib.util.find_spec("onnxruntime") else "configured; install mnemosyne[vector]"
    else:
        embedding_detail = "configured" if importlib.util.find_spec("httpx") else "configured; install httpx"
    checks.append(("embedder", True, embedding_detail, False))
    rerank = config.get("rerank", {})
    rerank_enabled = bool(rerank.get("enabled"))
    rerank_detail = "disabled" if not rerank_enabled else (
        "configured" if importlib.util.find_spec("onnxruntime") else "configured; install mnemosyne[rerank]"
    )
    checks.append(("reranker", True, rerank_detail, False))
    mcp_available = importlib.util.find_spec("mcp") is not None
    checks.append(("mcp", True, "available" if mcp_available else "install mnemosyne[mcp] to enable", False))
    try:
        template_text("core_project.md")
        template_text("AGENTS.md")
        templates_ok = True
    except OSError:
        templates_ok = False
    checks.append(("templates", templates_ok, "required by init and hooks", True))
    fts_ok = fts_available()
    checks.append((
        "fts5",
        fts_ok,
        "indexed search enabled" if fts_ok else "unavailable; falls back to in-memory BM25",
        False,
    ))
    for store in stores_for_scope(args.scope):
        initialized = store.core_path.exists()
        store_detail = str(store.root) if initialized else f"{store.root} (not initialized; run init)"
        checks.append((f"{store.scope} store", initialized, store_detail, False))
        if initialized:
            checks.append((f"{store.scope} working", store.working_dir.exists(), str(store.working_dir), True))
            if index_path(store).exists():
                index_detail = str(index_path(store))
            else:
                index_detail = f"{index_path(store)} (not built yet; run reindex)"
            checks.append((f"{store.scope} index", True, index_detail, False))
    failed = 0
    for name, ok, detail, hard in checks:
        status = "ok" if ok else ("missing" if hard else "info")
        if not ok and hard:
            failed += 1
        print(f"{status:7} {name}: {detail}")
    return 1 if failed else 0


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
    allow_custom = bool(args.allow_custom or load_config(first[0]).get("relations", {}).get("allow_custom"))
    if args.rel not in PREDEFINED and not allow_custom:
        choices = ", ".join(sorted(PREDEFINED))
        print(
            f"Unknown relation: {args.rel}. Choose one of: {choices}; use --allow-custom to opt in.",
            file=sys.stderr,
        )
        return 2

    add_link(first_memory, second_memory.id, args.rel)
    add_link(second_memory, first_memory.id, reverse(args.rel) or args.rel)
    write_memory(first_path, first_memory)
    write_memory(second_path, second_memory)
    update_search_index(first[0], first_path, first_memory)
    update_search_index(second[0], second_path, second_memory)
    print(f"Linked {first_memory.id} <-> {second_memory.id} ({args.rel})")
    return 0


def cmd_graph(args: argparse.Namespace) -> int:
    from mnemosyne.graph import build_graph, render_graph

    try:
        graph = build_graph(args.id, stores_for_scope("all"), depth=args.depth)
    except ValueError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    print(render_graph(graph, args.format))
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


def update_memory_index_file(store: Store, memory: Memory) -> None:
    index_path = store.root / "MEMORY.md"
    if not index_path.exists():
        try:
            index_path.write_text(template_text("MEMORY.md"), encoding="utf-8")
        except FileNotFoundError:
            index_path.write_text("# Memory Index\n", encoding="utf-8")
    with index_path.open("a", encoding="utf-8") as handle:
        handle.write(f"\n- `{memory.id}` ({memory.type}, strength {memory.strength}): {memory.injection_summary}\n")


def print_search_results(indexed_results, output_format: str, config: dict) -> int:
    threshold_bonus = int(config["thresholds"].get("bonus_access", 5))
    recall_bonus = int(config["thresholds"].get("bonus_recall", 20))
    output = []
    today = date.today().isoformat()
    for result in indexed_results:
        memory = result.memory
        path = result.path
        memory.access_count += 1
        memory.last_accessed = today
        is_archive = "archive" in str(path)
        bonus = recall_bonus if is_archive else threshold_bonus
        memory.strength = min(100, memory.strength + bonus)
        with suppress(portalocker.exceptions.LockException):
            write_memory(path, memory, lock_timeout=0)
            update_search_index(result.store, path, memory)
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
    if output_format == "json":
        print(json.dumps(output, ensure_ascii=False, indent=2))
    else:
        for item in output:
            print(f"[{item['score']:.4f}] {item['id']} ({item['scope']}/{item['type']})")
            print(item["summary"])
            if item.get("why_matched"):
                print(f"match: {item['why_matched']}")
            print(f"path: {item['path']}")
            print()
    return 0


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
