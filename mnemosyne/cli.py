"""Argparse command line interface for Mnemosyne."""

from __future__ import annotations

import argparse
from contextlib import suppress
import importlib.util
import json
import os
import uuid
import sys
from datetime import date
from pathlib import Path

import portalocker

from mnemosyne.lifecycle import MaintainSummary, is_date_expiry, maintain_memory
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
from mnemosyne.relations import PREDEFINED, is_demoting, reverse
from mnemosyne.schema import Memory, serialize_memory
from mnemosyne.search import BM25, SearchDocument, memory_search_text
from mnemosyne.store import (
    bump_memory_access,
    lock_store,
    lock_stores,
    Store,
    corrupt_memory_paths,
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
    write_parser.add_argument("--allow-duplicate", action="store_true",
        help="skip duplicate detection and write anyway")
    write_parser.set_defaults(func=cmd_write)

    search_parser = subparsers.add_parser("search", help="Search memories")
    search_parser.add_argument("query")
    search_parser.add_argument("--scope", choices=["global", "project", "all"], default="all")
    search_parser.add_argument("--type", default="")
    search_parser.add_argument("--limit", type=int, default=5)
    search_parser.add_argument("--format", choices=["text", "json"], default="text")
    search_parser.add_argument("--archive", action="store_true", help="include archive")
    search_parser.add_argument("--include-superseded", action="store_true",
        help="include memories superseded by newer ones")
    search_parser.set_defaults(func=cmd_search)

    maintain_parser = subparsers.add_parser("maintain", help="Decay and archive memories")
    maintain_parser.add_argument("--scope", choices=["global", "project", "all"], default="all")
    maintain_parser.add_argument("--dry-run", action="store_true")
    maintain_parser.set_defaults(func=cmd_maintain)

    consolidate_parser = subparsers.add_parser(
        "consolidate", help="Merge near-duplicate working memories (dry-run by default)")
    consolidate_parser.add_argument("--scope", choices=["global", "project", "all"], default="project")
    consolidate_parser.add_argument("--threshold", type=float, default=0.8,
        help="Jaccard similarity above which two same-type memories merge")
    consolidate_parser.add_argument("--commit", action="store_true",
        help="actually merge (default: preview only)")
    consolidate_parser.set_defaults(func=cmd_consolidate)

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

    mcp_parser = subparsers.add_parser("mcp", help="Run the optional MCP server")
    mcp_subparsers = mcp_parser.add_subparsers(dest="mcp_command")
    mcp_serve_parser = mcp_subparsers.add_parser("serve", help="Serve MCP over stdio")
    mcp_serve_parser.add_argument("--sse", action="store_true", help="serve optional SSE transport")
    mcp_serve_parser.set_defaults(func=cmd_mcp_serve)

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

    install_hermes_parser = subparsers.add_parser(
        "install-hermes", help="Install the Mnemosyne memory provider into Hermes")
    install_hermes_parser.add_argument("--python", default=None,
                                       help="bridge python that can import mnemosyne")
    install_hermes_parser.add_argument("--hermes-home", default=None,
                                       help="HERMES_HOME (default: $HERMES_HOME or ~/.hermes)")
    install_hermes_parser.add_argument("--force", action="store_true",
                                       help="overwrite an existing plugin dir")
    install_hermes_parser.add_argument("--no-config", action="store_true",
                                       help="do not edit config.yaml")
    install_hermes_parser.add_argument("--dry-run", action="store_true",
                                       help="preview without writing")
    install_hermes_parser.set_defaults(func=cmd_install_hermes)

    distill_parser = subparsers.add_parser("distill", help="Extract memories from a conversation transcript")
    distill_group = distill_parser.add_mutually_exclusive_group(required=True)
    distill_group.add_argument("--transcript", type=Path, help="Path to a Claude Code JSONL transcript")
    distill_group.add_argument("--stdin", action="store_true", help="Read plain transcript text from stdin")
    distill_parser.add_argument("--source", default="claude-code")
    distill_parser.add_argument("--commit", action="store_true", help="Persist findings (default: dry-run)")
    distill_parser.set_defaults(func=cmd_distill)

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
        strength=max(0, min(100, args.importance)),
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

    if args.expires and not is_date_expiry(args.expires):
        print(
            "Warning: --expires is not an ISO date (YYYY-MM-DD); kept as a note, will not auto-archive.",
            file=sys.stderr,
        )

    # Dedup applies to every write path, including --force: agents write with
    # --force by default, which used to bypass duplicate detection entirely
    # and left "search before write" as a convention instead of a mechanism.
    from mnemosyne.codex import Finding
    from mnemosyne.distill import classify_against_store

    distill_cfg = config.get("distill", {})
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

    finding = Finding(type=args.type, importance=memory.strength, title=title, tags=tags, content=content)
    with lock_store(store):
        # Classification and creation are one transaction, so simultaneous
        # writers cannot both observe an empty destination and create copies.
        verdict, target = classify_against_store(
            finding,
            store=store,
            dedup_threshold=float(distill_cfg.get("dedup_threshold", 0.85)),
            subject_threshold=float(distill_cfg.get("subject_threshold", 0.5)),
        )
        if verdict == "duplicate" and target and not args.allow_duplicate:
            print(f"Duplicate of {target}; skipped (use --allow-duplicate to write anyway).")
            return 0
        path = working_path(store, memory)
        write_memory(path, memory)
        update_memory_index_file(store, memory)
        update_search_index(store, path, memory)
    if verdict == "supersede" and target:
        from mnemosyne.distill import _apply_supersedes

        _apply_supersedes(memory.id, target)
        print(f"Supersedes {target} (linked, old memory demoted).")
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
        include_superseded=args.include_superseded,
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
    stores = getattr(args, "stores", None)
    if stores is None:
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
        if not args.dry_run:
            rewrite_memory_index_file(store)
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


def _consolidation_tokens(memory: Memory) -> set[str]:
    from mnemosyne.search import tokenize as _tokenize

    return set(_tokenize(f"{memory.canonical_summary} {memory.body}"))


def cmd_consolidate(args: argparse.Namespace) -> int:
    """Sleep-time style maintenance, conservative v1: merge near-duplicate
    working memories of the same type. Heuristic-only (no LLM), dry-run by
    default; the weaker memory is folded into the stronger via merge_memory."""
    from mnemosyne.distill import jaccard  # same similarity the dedup path uses
    from mnemosyne.index import remove_memory_index

    merged_total = 0
    for store in stores_for_scope(args.scope):
        with lock_store(store):
            entries = load_memories(store)
            entries.sort(key=lambda pair: pair[1].strength, reverse=True)
            consumed: set[str] = set()
            for i, (strong_path, strong) in enumerate(entries):
                if strong.id in consumed:
                    continue
                strong_tokens = _consolidation_tokens(strong)
                for weak_path, weak in entries[i + 1:]:
                    if weak.id in consumed or weak.type != strong.type:
                        continue
                    similarity = jaccard(sorted(strong_tokens), sorted(_consolidation_tokens(weak)))
                    if similarity < args.threshold:
                        continue
                    if not args.commit:
                        print(f"would merge {weak.id} -> {strong.id} (similarity {similarity:.2f})")
                        consumed.add(weak.id)
                        continue
                    merge_memory(strong, weak)
                    write_memory(strong_path, strong)
                    update_search_index(store, strong_path, strong)
                    weak_path.unlink(missing_ok=True)
                    remove_memory_index(store, weak.id)
                    consumed.add(weak.id)
                    merged_total += 1
                    print(f"merged {weak.id} -> {strong.id} (similarity {similarity:.2f})")
            if args.commit and consumed:
                rewrite_memory_index_file(store)
    if not args.commit:
        print("(dry-run) Add --commit to merge.")
    else:
        print(f"merged: {merged_total}")
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
            bad = corrupt_memory_paths(store)
            if bad:
                names = ", ".join(path.name for path in bad[:3])
                more = f" (+{len(bad) - 3} more)" if len(bad) > 3 else ""
                checks.append((f"{store.scope} corrupt files", False, f"{names}{more}; fix or delete, they are skipped", False))
            else:
                checks.append((f"{store.scope} corrupt files", True, "none", False))
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


# Strength penalty applied to the target of a demoting relation (e.g. the
# memory that gets superseded). Activates the demote_target relation semantics.
DEMOTE_ON_SUPERSEDE = 20


def cmd_link(args: argparse.Namespace) -> int:
    requested_stores = getattr(args, "stores", None)
    if requested_stores is None:
        requested_stores = stores_for_scope("all")
    stores = [store for store in requested_stores if store.root.exists()]
    with lock_stores(stores):
        # Resolve after locking: callers may have loaded either endpoint before
        # a concurrent access update or supersedence write completed.
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
        if is_demoting(args.rel):
            second_memory.strength = max(0, second_memory.strength - DEMOTE_ON_SUPERSEDE)
            second_memory.status = "superseded"
            second_memory.extra["invalidated_by"] = first_memory.id
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


def cmd_mcp_serve(args: argparse.Namespace) -> int:
    from mnemosyne.mcp.server import MissingMCPDependency, serve

    try:
        return serve(sse=args.sse)
    except MissingMCPDependency:
        print("MCP support is optional; please pip install mnemosyne[mcp].", file=sys.stderr)
        return 1


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


def print_search_results(indexed_results, output_format: str, config: dict) -> int:
    threshold_bonus = int(config["thresholds"].get("bonus_access", 5))
    recall_bonus = int(config["thresholds"].get("bonus_recall", 20))
    output = []
    today = date.today().isoformat()
    for result in indexed_results:
        memory = result.memory
        path = result.path
        is_archive = result.store.archive_dir in path.parents
        bonus = recall_bonus if is_archive else threshold_bonus
        with suppress(portalocker.exceptions.LockException):
            memory = bump_memory_access(
                result.store,
                path,
                bonus,
                today=today,
                lock_timeout=0,
                sync_index=True,
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


def cmd_install_hermes(args: argparse.Namespace) -> int:
    import os
    from pathlib import Path
    from mnemosyne.integrations.hermes import _install

    home = args.hermes_home or os.environ.get("HERMES_HOME") or str(Path.home() / ".hermes")
    try:
        result = _install.install_hermes(
            hermes_home=Path(home),
            python_path=args.python,
            force=args.force,
            write_config=not args.no_config,
            dry_run=args.dry_run,
        )
    except FileExistsError as exc:
        print(f"error: {exc}")
        return 1
    if args.dry_run:
        print("[dry-run] would install to", result["plugin_dir"])
        print("[dry-run] bridge python:", result["python"])
        if "config_preview" in result:
            print("[dry-run] config.yaml after edit:\n")
            print(result["config_preview"])
    else:
        print("Installed Mnemosyne provider to", result["plugin_dir"])
        print("Bridge python:", result["python"])
        if result["config_written"]:
            print("Updated config.yaml (backup:", result["backup"], ")")
            print("Restart Hermes to activate (memory.provider: mnemosyne).")
        else:
            print("Skipped config.yaml — set memory.provider: mnemosyne manually.")
    return 0


def cmd_distill(args: argparse.Namespace) -> int:
    from mnemosyne.distill import distill_text, parse_claude_transcript, turns_to_text

    if args.stdin:
        text = sys.stdin.read()
    else:
        text = turns_to_text(parse_claude_transcript(args.transcript))
    actions = distill_text(text, source=args.source, commit=args.commit)
    if not actions:
        print("No findings extracted.")
        return 0
    for action in actions:
        marker = action.get("id", "(dry-run)")
        print(f"[{action['verdict']}] {action['type']}: {action['title']} -> {marker}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
