"""Common helpers for Claude Code hook scripts."""

from __future__ import annotations

import json
import os
import sys
import traceback
from contextlib import contextmanager
from datetime import date, datetime, timedelta
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


SESSION_STATE_FILENAME = '.session_injected.json'
SESSION_STATE_TTL_HOURS = 48


def _session_state_path() -> Path:
    project = find_project_store()
    root = project.root if project is not None else global_store().root
    return root / SESSION_STATE_FILENAME


def load_injected_ids(session_id: str) -> set[str]:
    if not session_id:
        return set()
    try:
        data = json.loads(_session_state_path().read_text(encoding='utf-8'))
    except (OSError, ValueError):
        return set()
    entry = data.get('sessions', {}).get(session_id, {})
    return set(entry.get('ids', []))


def record_injected_ids(session_id: str, memory_ids: list[str]) -> None:
    """Remember which memories this session has already seen.

    Injection hooks fire on every prompt and edit; without this, the same
    memory is re-injected each turn and quietly eats the context budget.
    Sessions older than the TTL are pruned so the state file stays small.
    """
    if not session_id or not memory_ids:
        return
    path = _session_state_path()
    try:
        data = json.loads(path.read_text(encoding='utf-8'))
    except (OSError, ValueError):
        data = {}
    if not isinstance(data, dict):
        data = {}
    sessions = data.setdefault('sessions', {})
    now = datetime.now()
    cutoff = now - timedelta(hours=SESSION_STATE_TTL_HOURS)
    for key in list(sessions):
        try:
            stamp = datetime.fromisoformat(str(sessions[key].get('ts', '')))
        except (AttributeError, ValueError):
            stamp = None
        if stamp is None or stamp < cutoff:
            del sessions[key]
    entry = sessions.setdefault(session_id, {'ts': now.isoformat(), 'ids': []})
    entry['ts'] = now.isoformat()
    entry['ids'] = sorted(set(entry['ids']) | set(memory_ids))
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix('.tmp')
        tmp.write_text(json.dumps(data, ensure_ascii=False), encoding='utf-8')
        os.replace(tmp, path)
    except OSError:
        pass


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
    config = load_config(stores[-1] if stores else None)
    bonus = int(config['thresholds'].get('bonus_access', 5))

    if bool(config.get('search', {}).get('index_enabled', True)):
        indexed = _run_search_indexed(stores, query, limit, update_access, bonus)
        if indexed is not None:
            return indexed

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
            'strength': int(memory.strength),
            'score': round(result.score, 4),
        })
    return output


def _run_search_indexed(
    stores: list[Store],
    query: str,
    limit: int,
    update_access: bool,
    bonus: int,
) -> list[dict] | None:
    """Try the persistent FTS index; return None to fall back to in-memory BM25.

    Hooks fire on every prompt and edit, so this is the hot path. The index
    stats files cheaply instead of re-reading every memory from disk. Any
    failure (no FTS5 build, locked db, import error) returns None so the
    caller transparently degrades to the BM25 scan.
    """
    try:
        from mnemosyne.fusion import search as fusion_search
        from mnemosyne.index import fts_available, update_memory_index
    except ImportError:
        return None
    if not fts_available():
        return None
    try:
        config = load_config(stores[-1] if stores else None)
        indexed = fusion_search(stores, query, limit=limit, include_archive=False, config=config)
    except Exception:
        return None
    output: list[dict] = []
    for result in indexed:
        memory = result.memory
        if update_access:
            try:
                memory.access_count += 1
                memory.last_accessed = date.today().isoformat()
                memory.strength = min(100, memory.strength + bonus)
                write_memory(result.path, memory, lock_timeout=0)
                update_memory_index(result.store, result.path, memory)
            except portalocker.exceptions.LockException:
                pass
            except Exception:
                pass
        output.append({
            'id': memory.id,
            'scope': result.store.scope,
            'type': memory.type,
            'tags': list(memory.tags),
            'summary': memory.injection_summary,
            'strength': int(memory.strength),
            'score': round(result.score, 4),
        })
    return output


def format_for_injection(
    results: list[dict],
    max_tokens: int | None = None,
    summary_chars: int = 120,
) -> str:
    """One line per memory plus a `show` hint: the injection is a table of
    contents, not the content. Agents pull full bodies on demand, which keeps
    the per-prompt context cost near-constant as the store grows."""
    if not results:
        return ''
    lines: list[str] = ['## Relevant memories from Mnemosyne', '']
    footer = 'Run `python3 -m mnemosyne show <id>` for full detail.'
    used_tokens = _approx_tokens('\n'.join(lines)) + _approx_tokens(footer)
    # Relevance first: a strong-but-irrelevant memory must not displace a
    # weak-but-relevant one. Strength only breaks ties between equal scores.
    sorted_results = sorted(
        results,
        key=lambda item: (float(item.get('score', 0) or 0), int(item.get('strength', 0) or 0)),
        reverse=True,
    )
    emitted = 0
    for item in sorted_results:
        tag_part = f" [{', '.join(item['tags'])}]" if item['tags'] else ''
        summary = ' '.join(str(item['summary']).split())
        if len(summary) > summary_chars:
            summary = summary[: max(1, summary_chars - 3)].rstrip() + '...'
        line = f"- ({item['scope']}/{item['type']}) {item['id']}{tag_part}: {summary}"
        line_tokens = _approx_tokens(line)
        if max_tokens is not None and used_tokens + line_tokens > max_tokens:
            if emitted == 0:
                remaining = max(0, (max_tokens - used_tokens - 8) * 4)
                if remaining > 0:
                    lines.append(line[:remaining].rstrip() + '...')
                    emitted += 1
            break
        lines.append(line)
        used_tokens += line_tokens
        emitted += 1
    if emitted == 0:
        return ''
    lines.extend(['', footer])
    return '\n'.join(lines)


def _approx_tokens(text: str) -> int:
    # CJK text packs far more tokens per character than Latin text (~1 token per
    # 1.5 chars vs ~1 per 4). A flat len//4 underestimates CJK budgets by ~2.5x,
    # which silently overflows the injection token cap for Chinese/Japanese/Korean.
    cjk = sum(
        1
        for ch in text
        if "一" <= ch <= "鿿"  # CJK unified ideographs
        or "぀" <= ch <= "ヿ"  # Hiragana + Katakana
        or "가" <= ch <= "힣"  # Hangul syllables
    )
    other = len(text) - cjk
    return max(1, round(cjk * 0.6 + other / 4))
