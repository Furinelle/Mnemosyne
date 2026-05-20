"""Common helpers for Claude Code hook scripts."""

from __future__ import annotations

import sys
import traceback
from contextlib import contextmanager
from datetime import date
from pathlib import Path

try:
    import portalocker
except ModuleNotFoundError:
    class _LockException(Exception):
        pass

    class _PortalockerExceptions:
        LockException = _LockException

    class _Portalocker:
        exceptions = _PortalockerExceptions()

    portalocker = _Portalocker()

from mnemosyne.search import BM25, SearchDocument, memory_search_text, tokenize
from mnemosyne.store import (
    Store,
    find_project_store,
    global_store,
    load_config,
    load_memories,
    write_memory,
)


STOPWORDS: frozenset[str] = frozenset({
    'the', 'a', 'an', 'is', 'are', 'was', 'were', 'be', 'been', 'being',
    'do', 'does', 'did', 'have', 'has', 'had', 'will', 'would', 'shall', 'should',
    'can', 'could', 'may', 'might', 'must', 'this', 'that', 'these', 'those',
    'i', 'you', 'he', 'she', 'it', 'we', 'they', 'me', 'him', 'her', 'us', 'them',
    'my', 'your', 'his', 'its', 'our', 'their', 'and', 'or', 'but', 'if', 'then',
    'of', 'to', 'in', 'on', 'at', 'by', 'for', 'with', 'from', 'as', 'so',
    '的', '是', '了', '我', '你', '他', '她', '它', '我们', '你们', '他们',
    '这', '那', '这个', '那个', '这些', '那些', '和', '与', '或', '但', '如果',
    '一', '一个', '一些', '什么', '怎么', '为什么', '哪里', '哪个',
})


@contextmanager
def hook_safe():
    """Ensure any hook script exits 0 even on exceptions, to never block Claude."""
    try:
        yield
    except Exception:
        traceback.print_exc(file=sys.stderr)
        sys.exit(0)


def read_event() -> dict:
    """Read a JSON event from stdin, tolerating a leading UTF-8 BOM.

    PowerShell 7+ prepends a BOM when piping strings to subprocess stdin,
    which json.load otherwise rejects. Returns {} when stdin is empty.
    """
    import json

    raw = sys.stdin.read()
    if not raw:
        return {}
    if raw.startswith('﻿'):
        raw = raw[1:]
    return json.loads(raw)


def extract_keywords(text: str, limit: int = 8) -> list[str]:
    tokens = tokenize(text)
    seen: set[str] = set()
    result: list[str] = []
    for token in tokens:
        if token in STOPWORDS or len(token) < 2:
            continue
        if token in seen:
            continue
        seen.add(token)
        result.append(token)
        if len(result) >= limit:
            break
    return result


def collect_stores() -> list[Store]:
    stores: list[Store] = [global_store()]
    project = find_project_store()
    if project is not None:
        stores.append(project)
    return stores


def run_search(query: str, limit: int = 5, update_access: bool = False) -> list[dict]:
    if not query.strip():
        return []
    stores = collect_stores()
    documents: list[SearchDocument] = []
    path_lookup: dict[str, tuple[Store, Path, object]] = {}
    for store in stores:
        for path, memory in load_memories(store, include_archive=False):
            document_id = f'{store.scope}:{memory.id}'
            documents.append(SearchDocument(document_id, memory_search_text(memory), memory))
            path_lookup[document_id] = (store, path, memory)
    if not documents:
        return []
    results = BM25(documents).search(query, limit)
    if not results:
        return []
    config = load_config(stores[-1])
    bonus = int(config['thresholds'].get('bonus_access', 5))
    output: list[dict] = []
    for result in results:
        store, path, memory = path_lookup[result.document.id]
        if update_access:
            try:
                memory.access_count += 1
                memory.last_accessed = date.today().isoformat()
                memory.strength = min(100, memory.strength + bonus)
                write_memory(path, memory, lock_timeout=0)
            except portalocker.exceptions.LockException:
                pass
        output.append({
            'id': memory.id,
            'scope': store.scope,
            'type': memory.type,
            'tags': list(memory.tags),
            'summary': memory.injection_summary,
            'score': round(result.score, 4),
        })
    return output


def format_for_injection(results: list[dict]) -> str:
    if not results:
        return ''
    lines: list[str] = ['## Relevant memories from Mnemosyne', '']
    for item in results:
        tag_part = f" [{', '.join(item['tags'])}]" if item['tags'] else ''
        lines.append(f"- ({item['scope']}/{item['type']}) {item['id']}{tag_part}")
        summary = item['summary']
        if len(summary) > 220:
            summary = summary[:217].rstrip() + '...'
        lines.append(f'  {summary}')
    return '\n'.join(lines)
