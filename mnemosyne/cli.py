"""Argparse command line interface for Mnemosyne."""

from __future__ import annotations

import argparse
from copy import deepcopy
from dataclasses import fields
import importlib.util
import json
import sys
from datetime import date
from pathlib import Path


from mnemosyne import api
from mnemosyne.api import (  # noqa: F401  (re-exported for backward compat)
    DEMOTE_ON_SUPERSEDE,
    MnemosyneError,
    add_link,
    first_line_title,
    make_memory_id,
    rewrite_memory_index_file,
    summarize,
    update_memory_index_file,
)
from mnemosyne.lifecycle import is_date_expiry
from mnemosyne.index import (
    backfill_embeddings,
    fts_available,
    index_enabled,
    index_path,
    reindex_store,
    update_memory_index as update_search_index,
)
from mnemosyne.embedding import get_embedder
from mnemosyne.fusion import search as fusion_search
from mnemosyne.schema import Memory, serialize_memory
from mnemosyne.search import BM25, SearchDocument, memory_search_text
from mnemosyne.store import (
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
    templates_dir,
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
    init_parser.add_argument("--agent", choices=["generic", "codex", "claude-code", "hermes"],
        default="generic", help="which agent's guidance files to write (default: generic)")
    init_parser.add_argument("--no-agent-files", action="store_true",
        help="do not write AGENTS.md or other guidance files")
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

    for prep_name, prep_help in (
        ("prep", "Generate a context prompt prefix for any agent task"),
        ("codex-prep", "Alias of prep (kept for backward compatibility)"),
    ):
        prep_parser = subparsers.add_parser(prep_name, help=prep_help)
        prep_parser.add_argument('task')
        prep_parser.add_argument('--limit', type=int, default=5)
        prep_parser.set_defaults(func=cmd_prep)

    for ingest_name, ingest_help in (
        ("ingest", "Parse findings blocks from stdin and write them as memories"),
        ("codex-ingest", "Alias of ingest (kept for backward compatibility)"),
    ):
        ingest_parser = subparsers.add_parser(ingest_name, help=ingest_help)
        ingest_parser.add_argument('--source', default='codex')
        ingest_parser.add_argument('--commit', action='store_true',
            help='actually write (default: dry-run preview)')
        ingest_parser.add_argument('--format', dest='fmt',
            choices=['auto', 'markdown', 'json'], default='auto',
            help='findings block format (default: auto-detect)')
        ingest_parser.set_defaults(func=cmd_ingest)

    from mnemosyne.integrations._registry import INSTALLERS

    install_parser = subparsers.add_parser(
        "install", help="Install a Mnemosyne adapter into an agent host")
    install_parser.add_argument("agent", choices=sorted(INSTALLERS))
    _add_install_options(install_parser)
    install_parser.set_defaults(func=cmd_install)

    install_hermes_parser = subparsers.add_parser(
        "install-hermes", help="Alias of `install hermes` (kept for backward compatibility)")
    _add_install_options(install_hermes_parser)
    install_hermes_parser.set_defaults(func=cmd_install, agent="hermes")

    inject_parser = subparsers.add_parser(
        "inject", help="Produce injection context for an agent-neutral lifecycle event")
    inject_parser.add_argument("--event", required=True,
        choices=["session_start", "turn_start", "file_touch", "session_end"])
    inject_parser.add_argument("--session", default="",
        help="opaque session id for cross-turn injection dedup")
    inject_parser.add_argument("--channel", choices=["cli", "mcp", "none"], default="cli",
        help="which retrieval hint fits the calling agent")
    inject_parser.add_argument("--format", dest="fmt", choices=["text", "json"], default="text")
    inject_parser.add_argument("--fail-safe", action="store_true",
        help="never fail: on any error print nothing and exit 0")
    inject_parser.set_defaults(func=cmd_inject)

    distill_parser = subparsers.add_parser("distill", help="Extract memories from a conversation transcript")
    distill_group = distill_parser.add_mutually_exclusive_group(required=True)
    distill_group.add_argument("--transcript", type=Path,
        help="Path to a transcript (claude-jsonl, role-jsonl, or plain text)")
    distill_group.add_argument("--stdin", action="store_true", help="Read plain transcript text from stdin")
    distill_parser.add_argument("--format", dest="fmt",
        choices=["auto", "claude-jsonl", "role-jsonl", "text"], default="auto",
        help="transcript format (default: auto-detect)")
    distill_parser.add_argument("--source", default="agent")
    distill_parser.add_argument("--commit", action="store_true", help="Persist findings (default: dry-run)")
    distill_parser.set_defaults(func=cmd_distill)

    return parser


AGENT_GUIDANCE_TEMPLATES = {
    "generic": "agents/generic/AGENTS.md",
    "codex": "agents/codex/AGENTS.md",
    "claude-code": "agents/generic/AGENTS.md",
    "hermes": "agents/generic/AGENTS.md",
}


def cmd_init(args: argparse.Namespace) -> int:
    store = Store("project", Path.cwd() / ".mnemosyne")
    ensure_store(store, template_text("core_project.md"))
    print("Mnemosyne initialized. Add .mnemosyne/ to .gitignore or commit it.")
    agent = getattr(args, "agent", "generic")
    if not getattr(args, "no_agent_files", False):
        agents_path = Path.cwd() / "AGENTS.md"
        if not agents_path.exists():
            try:
                agents_path.write_text(
                    template_text(AGENT_GUIDANCE_TEMPLATES[agent]), encoding="utf-8")
                print(f"Wrote {agents_path}")
            except FileNotFoundError:
                pass
    print()
    print("Next steps:")
    print("  1. Ensure mnemosyne is importable from any cwd (recommended):")
    print("       pip install -e .")
    if agent == "hermes":
        print("  2. Install the Hermes provider plugin:")
        print("       python3 -m mnemosyne install hermes")
    elif agent == "codex":
        print("  2. Codex reads AGENTS.md automatically; hand off with")
        print("     `mnemosyne prep` and report back with findings blocks.")
    else:
        settings_path = templates_dir() / "agents" / "claude_code" / "settings.json"
        print("  2. To enable Claude Code auto-injection, merge this hooks config")
        print("     into ~/.claude/settings.json or .claude/settings.json:")
        print(f"       {settings_path}")
        print("  3. Any MCP client can attach with: python3 -m mnemosyne mcp serve")
    return 0


def cmd_read(args: argparse.Namespace) -> int:
    for store in stores_for_scope(args.scope):
        label = "Global Core Memory" if store.scope == "global" else "Project Core Memory"
        print(f"===== {label}: {store.root} =====")
        content = read_core(store)
        print(content.rstrip() if content else "(empty)")
        print()
    return 0


import re as _re

_SOURCE_RE = _re.compile(r"^[a-z0-9_-]+(:[a-z0-9_.-]+)?$")


def _normalize_source(value: str) -> str:
    """Normalize source names to `<agent>[:<profile>]` (lowercase)."""
    normalized = str(value or "").strip().lower()
    if normalized and not _SOURCE_RE.match(normalized):
        print(f"Warning: source '{normalized}' does not match <agent>[:<profile>].", file=sys.stderr)
    return normalized or "agent"


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
    tags = [tag.strip() for tag in args.tags.split(",") if tag.strip()]

    if args.expires and not is_date_expiry(args.expires):
        print(
            "Warning: --expires is not an ISO date (YYYY-MM-DD); kept as a note, will not auto-archive.",
            file=sys.stderr,
        )

    # Interactive merge stays in the CLI: it needs a TTY prompt. The dedup
    # transaction itself lives in api.write_entry for every caller.
    if not args.force and sys.stdin.isatty():
        today = date.today().isoformat()
        summary = summarize(title, content)
        probe = Memory(
            id=make_memory_id(args.type, today),
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
        duplicate = duplicate_prompt(store, probe)
        if duplicate == "cancel":
            print("Cancelled.")
            return 1
        if duplicate and duplicate != "none":
            duplicate_path, duplicate_memory = duplicate
            if merge_memory(duplicate_memory, probe):
                write_memory(duplicate_path, duplicate_memory)
                print(f"Merged into {duplicate_memory.id}")
                return 0
            print("Selected duplicate has incompatible metadata; writing a separate memory.")

    result = api.write_entry(
        type=args.type,
        importance=args.importance,
        content=content,
        title=title,
        tags=tags,
        scope=args.scope,
        source=_normalize_source(args.source),
        expires=args.expires,
        allow_duplicate=args.allow_duplicate,
    )
    if result.status == "duplicate":
        print(f"Duplicate of {result.duplicate_of}; skipped (use --allow-duplicate to write anyway).")
        return 0
    if result.superseded:
        print(f"Supersedes {result.superseded} (linked, old memory demoted).")
    print(f"Wrote {result.id}")
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
    summary = api.maintain(
        scope=args.scope,
        dry_run=args.dry_run,
        stores=getattr(args, "stores", None),
    )
    print(f"processed: {summary['processed']}")
    print(f"decayed: {summary['decayed']}")
    print(f"deprecated: {summary['deprecated']}")
    print(f"archived: {summary['archived']}")
    if summary["core_candidates"]:
        print("Core candidates:")
        for candidate in summary["core_candidates"]:
            print(f"- {candidate['id']}: {candidate['summary']}")
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
    target_stores = stores_for_scope(args.scope)
    visible_stores = [store for store in stores_for_scope("all") if store.root.exists()]

    def consolidate_loaded_stores() -> None:
        nonlocal merged_total
        for store in target_stores:
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
                    candidate = deepcopy(strong)
                    if not merge_memory(candidate, weak):
                        print(
                            f"skipped {weak.id} -> {strong.id}: incompatible lifecycle or custom metadata"
                        )
                        continue
                    if not args.commit:
                        print(f"would merge {weak.id} -> {strong.id} (similarity {similarity:.2f})")
                        consumed.add(weak.id)
                        continue
                    strong = candidate
                    write_memory(strong_path, strong)
                    update_search_index(store, strong_path, strong)
                    _rewrite_memory_references(visible_stores, weak.id, strong.id)
                    weak_path.unlink(missing_ok=True)
                    remove_memory_index(store, weak.id)
                    consumed.add(weak.id)
                    merged_total += 1
                    print(f"merged {weak.id} -> {strong.id} (similarity {similarity:.2f})")
            if args.commit and consumed:
                rewrite_memory_index_file(store)

    if args.commit:
        with lock_stores(visible_stores):
            consolidate_loaded_stores()
    else:
        consolidate_loaded_stores()
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
    checks.append(("mcp", True, "stdlib server built-in", False))
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


def cmd_link(args: argparse.Namespace) -> int:
    try:
        result = api.link_entries(
            args.id1,
            args.id2,
            rel=args.rel,
            allow_custom=bool(args.allow_custom),
            stores=getattr(args, "stores", None),
        )
    except MnemosyneError as exc:
        print(str(exc), file=sys.stderr)
        return exc.exit_code
    print(f"Linked {result['id1']} <-> {result['id2']} ({result['rel']})")
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
    from mnemosyne.mcp.server import serve

    return serve(sse=args.sse)


def cmd_prep(args: argparse.Namespace) -> int:
    from mnemosyne.handoff import prep
    print(prep(args.task, max_memories=args.limit, channel='cli'))
    return 0


def cmd_ingest(args: argparse.Namespace) -> int:
    from mnemosyne.handoff import ingest
    text = sys.stdin.read()
    if not text.strip():
        print('No input on stdin.', file=sys.stderr)
        return 2
    actions = ingest(text, source=_normalize_source(args.source), commit=args.commit, fmt=getattr(args, 'fmt', 'auto'))
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


def print_search_results(indexed_results, output_format: str, config: dict) -> int:
    output = api.results_to_dicts(indexed_results, config)
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


def _list_values(value) -> list[str]:
    if value in (None, ""):
        return []
    if isinstance(value, list):
        return [str(item) for item in value]
    return [str(value)]


def merge_memory(existing: Memory, incoming: Memory) -> bool:
    """Losslessly fold incoming into existing, or leave existing unchanged."""
    if existing.type != incoming.type or existing.status != incoming.status:
        return False
    if existing.expires and incoming.expires and existing.expires != incoming.expires:
        return False

    candidate = deepcopy(existing)
    candidate.strength = max(existing.strength, incoming.strength)
    candidate.created = min(filter(None, (existing.created, incoming.created)), default="")
    candidate.last_accessed = max(existing.last_accessed, incoming.last_accessed)
    candidate.access_count = existing.access_count + incoming.access_count
    candidate.expires = existing.expires or incoming.expires
    for tag in incoming.tags:
        if tag not in candidate.tags:
            candidate.tags.append(tag)

    links: list[dict[str, str]] = []
    seen_links: set[tuple[str, str]] = set()
    for link in [*existing.links, *incoming.links]:
        target = str(link.get("id", ""))
        relation = str(link.get("rel", ""))
        if not target or target in {existing.id, incoming.id}:
            continue
        key = (target, relation)
        if key not in seen_links:
            links.append({"id": target, "rel": relation})
            seen_links.add(key)
    candidate.links = links

    if incoming.body and incoming.body not in candidate.body:
        candidate.body = candidate.body.rstrip() + "\n\n" + incoming.body.strip()
    if incoming.canonical_summary and incoming.canonical_summary not in candidate.canonical_summary:
        candidate.canonical_summary = (
            candidate.canonical_summary + " " + incoming.canonical_summary
        ).strip()
    if incoming.injection_summary and incoming.injection_summary not in candidate.injection_summary:
        candidate.injection_summary = (
            candidate.injection_summary + " " + incoming.injection_summary
        ).strip()

    evidence_a = candidate.extra.pop("evidence", "")
    evidence_b = incoming.extra.get("evidence", "")
    if evidence_a or evidence_b:
        if not isinstance(evidence_a, str) or not isinstance(evidence_b, str):
            return False
        evidence = [item for item in (evidence_a, evidence_b) if item]
        candidate.extra["evidence"] = "\n\n".join(dict.fromkeys(evidence))

    ignored_extra = {"evidence", "merged_from", "merged_sources"}
    for key, value in incoming.extra.items():
        if key in ignored_extra:
            continue
        if key in candidate.extra and candidate.extra[key] != value:
            return False
        candidate.extra[key] = deepcopy(value)

    merged_from = _list_values(existing.extra.get("merged_from"))
    merged_from.extend(_list_values(incoming.extra.get("merged_from")))
    merged_from.append(incoming.id)
    candidate.extra["merged_from"] = list(dict.fromkeys(merged_from))
    sources = _list_values(existing.extra.get("merged_sources"))
    sources.extend([existing.source, incoming.source])
    sources.extend(_list_values(incoming.extra.get("merged_sources")))
    candidate.extra["merged_sources"] = list(dict.fromkeys(filter(None, sources)))

    for field in fields(Memory):
        setattr(existing, field.name, deepcopy(getattr(candidate, field.name)))
    return True


def _rewrite_memory_references(stores: list[Store], old_id: str, new_id: str) -> None:
    """Rewrite links and invalidation pointers after a consolidation merge."""
    for store in stores:
        for path, memory in load_memories(store, include_archive=True):
            if memory.id in {old_id, new_id}:
                continue
            changed = False
            rewritten: list[dict[str, str]] = []
            seen: set[tuple[str, str]] = set()
            for link in memory.links:
                target = new_id if link.get("id") == old_id else str(link.get("id", ""))
                relation = str(link.get("rel", ""))
                changed = changed or target != link.get("id")
                if not target or target == memory.id or (target, relation) in seen:
                    continue
                rewritten.append({"id": target, "rel": relation})
                seen.add((target, relation))
            if memory.extra.get("invalidated_by") == old_id:
                memory.extra["invalidated_by"] = new_id
                changed = True
            if not changed:
                continue
            memory.links = rewritten
            write_memory(path, memory)
            update_search_index(store, path, memory)


def _add_install_options(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--python", default=None,
                        help="bridge python that can import mnemosyne (hermes)")
    parser.add_argument("--hermes-home", default=None,
                        help="HERMES_HOME (default: $HERMES_HOME or ~/.hermes)")
    parser.add_argument("--force", action="store_true",
                        help="overwrite an existing plugin dir")
    parser.add_argument("--no-config", action="store_true",
                        help="do not edit the host's config file")
    parser.add_argument("--dry-run", action="store_true",
                        help="preview without writing")


def cmd_install(args: argparse.Namespace) -> int:
    from mnemosyne.integrations._registry import INSTALLERS

    handler = INSTALLERS.get(args.agent)
    if handler is None:
        known = ", ".join(sorted(INSTALLERS))
        print(f"Unknown agent: {args.agent}. Known: {known}", file=sys.stderr)
        return 2
    return handler(args)


def cmd_inject(args: argparse.Namespace) -> int:
    from mnemosyne.events import handle_event

    try:
        raw = sys.stdin.read().strip()
        payload = json.loads(raw) if raw else {}
        if not isinstance(payload, dict):
            raise ValueError("inject payload must be a JSON object")
        result = handle_event(args.event, payload, session=args.session, channel=args.channel)
    except Exception as exc:
        # Even in fail-safe mode, say what broke on stderr: session_end
        # persists memories, and a silently-swallowed failure there looks
        # identical to "nothing worth saving".
        print(f"inject failed: {exc}", file=sys.stderr)
        if args.fail_safe:
            return 0
        return 1
    if args.fmt == "json":
        print(json.dumps(
            {
                "context": result.context,
                "memory_ids": result.memory_ids,
                "approx_tokens": result.approx_tokens,
            },
            ensure_ascii=False,
        ))
    elif result.context:
        print(result.context)
    return 0


def cmd_distill(args: argparse.Namespace) -> int:
    from mnemosyne.distill import distill_text, turns_to_text
    from mnemosyne.transcripts import parse_transcript

    if args.stdin:
        text = sys.stdin.read()
    else:
        text = turns_to_text(parse_transcript(args.transcript, getattr(args, "fmt", "auto")))
    actions = distill_text(text, source=_normalize_source(args.source), commit=args.commit)
    if not actions:
        print("No findings extracted.")
        return 0
    for action in actions:
        marker = action.get("id", "(dry-run)")
        print(f"[{action['verdict']}] {action['type']}: {action['title']} -> {marker}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
