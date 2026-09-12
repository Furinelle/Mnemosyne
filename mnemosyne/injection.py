"""Host-agnostic context injection: keyword extraction, search, formatting.

This module has no knowledge of any agent's hook protocol; adapters map their
host events onto these helpers (usually via mnemosyne.events.handle_event).
"""

from __future__ import annotations

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

from mnemosyne.search import BM25, SearchDocument, memory_search_text
from mnemosyne.schema import is_expired
from mnemosyne.tokenizer import script_runs
from mnemosyne.store import (
    Store,
    bump_memory_access,
    find_project_store,
    global_store,
    load_config,
    load_memories,
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

DEFAULT_SHOW_HINT = 'Run `python3 -m mnemosyne show <id>` for full detail.'
MCP_SHOW_HINT = 'Call the mnemosyne_show tool with the id for full detail.'


def resolve_show_hint(config: dict, channel: str) -> str | None:
    """Pick the retrieval hint appended to injected memory lists.

    A pure-MCP client cannot run shell commands, so the hint must match the
    channel the agent actually has; "none" suppresses the footer entirely.
    """
    template = str(config.get('injection', {}).get('show_command_template', '') or '')
    if template:
        return template
    if channel == 'mcp':
        return MCP_SHOW_HINT
    if channel == 'none':
        return None
    return DEFAULT_SHOW_HINT


def extract_keywords(text: str, limit: int = 8) -> list[str]:
    # Let the search layer segment dense-script phrases. Counting their
    # bigrams here used up all slots before later project names were reached.
    if limit <= 0:
        return []
    seen: set[str] = set()
    result: list[str] = []
    for token, _dense in script_runs(text):
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


def run_search(
    query: str,
    limit: int = 5,
    update_access: bool = False,
    stores: list[Store] | None = None,
) -> list[dict]:
    if not query.strip():
        return []
    selected_stores = collect_stores() if stores is None else list(stores)
    config = load_config(selected_stores[-1] if selected_stores else None)
    bonus = int(config['thresholds'].get('bonus_access', 5))

    if bool(config.get('search', {}).get('index_enabled', True)):
        indexed = _run_search_indexed(selected_stores, query, limit, update_access, bonus)
        if indexed is not None:
            return indexed

    documents: list[SearchDocument] = []
    path_lookup: dict[str, tuple[Store, Path, object]] = {}
    for store in selected_stores:
        for path, memory in load_memories(store, include_archive=False):
            if memory.status == 'superseded' or is_expired(memory.expires):
                continue
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
                memory = bump_memory_access(
                    store, path, bonus, today=date.today().isoformat(), lock_timeout=0
                )
            except portalocker.exceptions.LockException:
                pass
        output.append({
            'id': memory.id,
            'path': str(path),
            'scope': store.scope,
            'type': memory.type,
            'tags': list(memory.tags),
            'summary': memory.injection_summary,
            'source': memory.source,
            'created': memory.created,
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

    Injection fires on every prompt and edit, so this is the hot path. The
    index stats files cheaply instead of re-reading every memory from disk.
    Any failure (no FTS5 build, locked db, import error) returns None so the
    caller transparently degrades to the BM25 scan.
    """
    try:
        from mnemosyne.fusion import search as fusion_search
        from mnemosyne.index import fts_available
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
                memory = bump_memory_access(
                    result.store,
                    result.path,
                    bonus,
                    today=date.today().isoformat(),
                    lock_timeout=0,
                    sync_index=True,
                )
            except portalocker.exceptions.LockException:
                pass
            except Exception:
                pass
        output.append({
            'id': memory.id,
            'path': str(result.path),
            'scope': result.store.scope,
            'type': memory.type,
            'tags': list(memory.tags),
            'summary': memory.injection_summary,
            'source': memory.source,
            'created': memory.created,
            'strength': int(memory.strength),
            'score': round(result.score, 4),
        })
    return output


def record_injected_access(results: list[dict], memory_ids: list[str]) -> None:
    """Count only rendered memories, using search-resolved paths without a rescan."""
    emitted = set(memory_ids)
    selected = [item for item in results if item['id'] in emitted and item.get('path')]
    if not selected:
        return
    stores = collect_stores()
    config = load_config(stores[-1] if stores else None)
    bonus = int(config['thresholds'].get('bonus_access', 5))
    by_scope = {store.scope: store for store in stores}
    for item in selected:
        store = by_scope.get(item['scope'])
        if store is None:
            continue
        try:
            bump_memory_access(store, Path(item['path']), bonus, lock_timeout=0, sync_index=True)
        except Exception:
            # Access accounting must not prevent injection if another agent
            # holds the lock, archives the memory, or the cache is unavailable.
            pass


def format_for_injection(
    results: list[dict],
    max_tokens: int | None = None,
    summary_chars: int = 120,
    show_hint: str | None = DEFAULT_SHOW_HINT,
    *,
    prefix: str = '',
    emitted_ids: list[str] | None = None,
) -> str:
    """One line per memory plus a retrieval hint: the injection is a table of
    contents, not the content. Agents pull full bodies on demand, which keeps
    the per-prompt context cost near-constant as the store grows."""
    if not results:
        return ''
    lines: list[str] = ([prefix.rstrip(), ''] if prefix else []) + ['## Relevant memories from Mnemosyne', '']
    footer = show_hint or ''

    def render(entries: list[str]) -> str:
        return '\n'.join([*lines, *entries, *(['', footer] if footer else [])])

    # Relevance first: a strong-but-irrelevant memory must not displace a
    # weak-but-relevant one. Strength only breaks ties between equal scores.
    sorted_results = sorted(
        results,
        key=lambda item: (float(item.get('score', 0) or 0), int(item.get('strength', 0) or 0)),
        reverse=True,
    )
    entries: list[str] = []
    for item in sorted_results:
        tag_part = f" [{', '.join(item['tags'])}]" if item['tags'] else ''
        summary = ' '.join(str(item['summary']).split())
        if len(summary) > summary_chars:
            summary = summary[: max(1, summary_chars - 3)].rstrip() + '...'
        source = ' '.join(str(item.get('source') or '').split())
        created = ' '.join(str(item.get('created') or '').split())
        provenance = [source] if source else []
        if created:
            provenance.append(f"recorded {created}")
        source_part = f" [{'; '.join(provenance)}]" if provenance else ''
        identity = f"- ({item['scope']}/{item['type']}) {item['id']}{source_part}"
        detail = f"{tag_part}: {summary}"
        line = identity + detail
        if max_tokens is not None and _approx_tokens(render([*entries, line])) > max_tokens:
            if entries:
                continue
            # Keep the complete id so every emitted item can actually be
            # retrieved. Measure CJK and all headings/footer while shrinking.
            while detail and _approx_tokens(render([identity + detail.rstrip() + '...'])) > max_tokens:
                detail = detail[:-1]
            line = identity + detail.rstrip() + '...'
            if _approx_tokens(render([line])) > max_tokens:
                continue
        entries.append(line)
        if emitted_ids is not None:
            emitted_ids.append(item['id'])
    if not entries:
        return ''
    return render(entries)


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
