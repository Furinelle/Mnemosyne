"""Persistent SQLite FTS index for Mnemosyne memories."""

from __future__ import annotations

import sqlite3
from contextlib import closing
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

from mnemosyne.schema import Memory, parse_memory
from mnemosyne.search import memory_search_text, tokenize
from mnemosyne.store import Store, iter_memory_paths, load_config, load_memories


INDEX_FILENAME = "index.sqlite"


def _connect(path: Path) -> sqlite3.Connection:
    """Open the index with concurrency-safe pragmas.

    Mnemosyne is multi-agent by design: the markdown store is guarded by
    portalocker, so the SQLite index needs matching protection. WAL lets
    readers and one writer coexist; busy_timeout makes concurrent writers
    wait briefly instead of immediately raising 'database is locked'.
    """
    connection = sqlite3.connect(str(path), timeout=5.0)
    connection.execute("PRAGMA busy_timeout=5000")
    connection.execute("PRAGMA journal_mode=WAL")
    return connection


@dataclass
class IndexedSearchResult:
    store: Store
    path: Path
    memory: Memory
    score: float
    why_matched: str


def index_path(store: Store) -> Path:
    return store.root / INDEX_FILENAME


def fts_available() -> bool:
    try:
        with closing(sqlite3.connect(":memory:")) as connection:
            connection.execute("CREATE VIRTUAL TABLE t USING fts5(content)")
        return True
    except sqlite3.Error:
        return False


def index_enabled(store: Store) -> bool:
    """Whether the persistent index should be maintained for this store.

    Returns False when SQLite lacks FTS5 (so write paths never try to build
    an index the runtime cannot query — otherwise ``mnemosyne write`` would
    crash on minimal Python builds) or when the user set
    ``search.index_enabled = false`` in config.toml.
    """
    if not fts_available():
        return False
    return bool(load_config(store).get("search", {}).get("index_enabled", True))


def ensure_index(store: Store) -> None:
    store.root.mkdir(parents=True, exist_ok=True)
    with closing(_connect(index_path(store))) as connection:
        connection.execute(
            """
            CREATE TABLE IF NOT EXISTS memories_meta (
                document_id TEXT PRIMARY KEY,
                scope TEXT NOT NULL,
                memory_id TEXT NOT NULL,
                path TEXT NOT NULL,
                type TEXT NOT NULL,
                archived INTEGER NOT NULL,
                strength INTEGER NOT NULL,
                tags TEXT NOT NULL,
                summary TEXT NOT NULL,
                mtime REAL NOT NULL DEFAULT 0
            )
            """
        )
        columns = {row[1] for row in connection.execute("PRAGMA table_info(memories_meta)")}
        if "mtime" not in columns:
            connection.execute("ALTER TABLE memories_meta ADD COLUMN mtime REAL NOT NULL DEFAULT 0")
        connection.execute(
            """
            CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                document_id UNINDEXED,
                title,
                summary,
                body,
                tags,
                type
            )
            """
        )
        connection.commit()


def reindex_store(store: Store, include_archive: bool = True) -> int:
    ensure_index(store)
    count = 0
    with closing(_connect(index_path(store))) as connection:
        connection.execute("DELETE FROM memories_meta")
        connection.execute("DELETE FROM memories_fts")
        for path, memory in load_memories(store, include_archive=include_archive):
            index_memory(connection, store, path, memory)
            count += 1
        connection.commit()
    return count


def sync_index(store: Store, include_archive: bool = True) -> None:
    """Reconcile the index with on-disk files using cheap mtime checks.

    Memories are plain markdown a user (or another tool) may edit directly,
    so the index can drift. This stats every file (cheap) and only re-reads
    and re-indexes the ones whose mtime changed, plus drops rows whose file
    has disappeared. Keeps the index trustworthy without a full rebuild.
    """
    if not index_enabled(store):
        return
    if not index_path(store).exists():
        reindex_store(store, include_archive=include_archive)
        return
    ensure_index(store)
    with closing(_connect(index_path(store))) as connection:
        rows = connection.execute(
            "SELECT document_id, path, mtime FROM memories_meta"
        ).fetchall()
        by_path = {row[1]: (row[0], row[2]) for row in rows}
        seen_paths: set[str] = set()
        for path in iter_memory_paths(store, include_archive=include_archive):
            spath = str(path)
            try:
                mtime = path.stat().st_mtime
            except OSError:
                continue
            seen_paths.add(spath)
            existing = by_path.get(spath)
            if existing is not None and abs(existing[1] - mtime) <= 1e-6:
                continue
            try:
                memory = parse_memory(path.read_text(encoding="utf-8"))
            except OSError:
                continue
            delete_memory_index(connection, store.scope, memory.id)
            index_memory(connection, store, path, memory)
        for spath, (document_id, _) in by_path.items():
            if spath not in seen_paths:
                connection.execute("DELETE FROM memories_meta WHERE document_id = ?", (document_id,))
                connection.execute("DELETE FROM memories_fts WHERE document_id = ?", (document_id,))
        connection.commit()


def update_memory_index(store: Store, path: Path, memory: Memory) -> None:
    if not index_enabled(store):
        return
    ensure_index(store)
    with closing(_connect(index_path(store))) as connection:
        delete_memory_index(connection, store.scope, memory.id)
        index_memory(connection, store, path, memory)
        connection.commit()


def remove_memory_index(store: Store, memory_id: str) -> None:
    if not index_path(store).exists():
        return
    with closing(_connect(index_path(store))) as connection:
        delete_memory_index(connection, store.scope, memory_id)
        connection.commit()


def search_index(
    stores: Iterable[Store],
    query: str,
    limit: int = 5,
    memory_type: str = "",
    include_archive: bool = False,
) -> list[IndexedSearchResult]:
    expression = _query_expression(query)
    if not expression:
        return []

    results: list[IndexedSearchResult] = []
    for store in stores:
        sync_index(store)
        with closing(_connect(index_path(store))) as connection:
            connection.row_factory = sqlite3.Row
            rows = _search_rows(connection, expression, limit, memory_type, include_archive)
        for row in rows:
            path = Path(row["path"])
            try:
                memory = parse_memory(path.read_text(encoding="utf-8"))
            except OSError:
                continue
            results.append(
                IndexedSearchResult(
                    store=store,
                    path=path,
                    memory=memory,
                    score=float(row["score"]),
                    why_matched=row["why_matched"],
                )
            )

    results.sort(key=lambda item: item.score, reverse=True)
    return results[:limit]


def index_memory(connection: sqlite3.Connection, store: Store, path: Path, memory: Memory) -> None:
    document_id = f"{store.scope}:{memory.id}"
    archived = 1 if "archive" in path.parts else 0
    tags = ", ".join(memory.tags)
    summary = memory.injection_summary or memory.canonical_summary
    try:
        mtime = path.stat().st_mtime
    except OSError:
        mtime = 0.0
    connection.execute(
        """
        INSERT OR REPLACE INTO memories_meta (
            document_id, scope, memory_id, path, type, archived, strength, tags, summary, mtime
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            document_id,
            store.scope,
            memory.id,
            str(path),
            memory.type,
            archived,
            int(memory.strength),
            tags,
            summary,
            mtime,
        ),
    )
    connection.execute(
        """
        INSERT INTO memories_fts (document_id, title, summary, body, tags, type)
        VALUES (?, ?, ?, ?, ?, ?)
        """,
        (document_id, memory.title, summary, memory_search_text(memory), tags, memory.type),
    )


def delete_memory_index(connection: sqlite3.Connection, scope: str, memory_id: str) -> None:
    document_id = f"{scope}:{memory_id}"
    connection.execute("DELETE FROM memories_meta WHERE document_id = ?", (document_id,))
    connection.execute("DELETE FROM memories_fts WHERE document_id = ?", (document_id,))


def _search_rows(
    connection: sqlite3.Connection,
    expression: str,
    limit: int,
    memory_type: str,
    include_archive: bool,
) -> list[sqlite3.Row]:
    filters = ["memories_fts MATCH ?"]
    params: list[object] = [expression]
    if memory_type:
        filters.append("m.type = ?")
        params.append(memory_type)
    if not include_archive:
        filters.append("m.archived = 0")
    params.append(limit)
    where = " AND ".join(filters)
    return list(
        connection.execute(
            f"""
            SELECT
                m.path,
                m.summary,
                m.type,
                -bm25(memories_fts) + (m.strength / 1000.0) AS score,
                snippet(memories_fts, 3, '[', ']', '...', 16) AS why_matched
            FROM memories_fts
            JOIN memories_meta m ON m.document_id = memories_fts.document_id
            WHERE {where}
            ORDER BY score DESC
            LIMIT ?
            """,
            params,
        )
    )


def _query_expression(query: str) -> str:
    tokens = tokenize(query)
    if not tokens:
        return ""
    return " OR ".join(f'"{token}"' for token in tokens)
