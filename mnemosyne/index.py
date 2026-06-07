"""Persistent SQLite FTS index for Mnemosyne memories."""

from __future__ import annotations

import sqlite3
import math
import struct
import time
from contextlib import closing
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

from mnemosyne.embedding.base import call_with_timeout
from mnemosyne.schema import Memory, parse_memory
from mnemosyne.search import memory_search_text, tokenize
from mnemosyne.store import Store, iter_memory_paths, load_config, load_memories


INDEX_FILENAME = "index.sqlite"
INDEX_VERSION = 2


def _connect(path: Path) -> sqlite3.Connection:
    """Open the index with concurrency-safe pragmas.

    Mnemosyne is multi-agent by design: the markdown store is guarded by
    portalocker, so the SQLite index needs matching protection. WAL lets
    readers and one writer coexist; busy_timeout makes concurrent writers
    wait briefly instead of immediately raising 'database is locked'.
    """
    connection = sqlite3.connect(str(path), timeout=5.0)
    connection.execute("PRAGMA busy_timeout=5000")
    if str(connection.execute("PRAGMA journal_mode").fetchone()[0]).lower() != "wal":
        deadline = time.monotonic() + 5.0
        while True:
            try:
                connection.execute("PRAGMA journal_mode=WAL")
                break
            except sqlite3.OperationalError as exc:
                if "locked" not in str(exc).lower() or time.monotonic() >= deadline:
                    connection.close()
                    raise
                time.sleep(0.05)
    return connection


@dataclass
class IndexedSearchResult:
    store: Store
    path: Path
    memory: Memory
    score: float
    why_matched: str


@dataclass
class IndexedEmbedding:
    store: Store
    path: Path
    memory: Memory
    vector: list[float]


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
                mtime REAL NOT NULL DEFAULT 0,
                embedding BLOB,
                embedding_model TEXT NOT NULL DEFAULT '',
                embedding_dim INTEGER NOT NULL DEFAULT 0,
                embedding_mtime REAL NOT NULL DEFAULT 0
            )
            """
        )
        columns = {row[1] for row in connection.execute("PRAGMA table_info(memories_meta)")}
        migrations = {
            "mtime": "REAL NOT NULL DEFAULT 0",
            "embedding": "BLOB",
            "embedding_model": "TEXT NOT NULL DEFAULT ''",
            "embedding_dim": "INTEGER NOT NULL DEFAULT 0",
            "embedding_mtime": "REAL NOT NULL DEFAULT 0",
        }
        for column, definition in migrations.items():
            if column not in columns:
                connection.execute(f"ALTER TABLE memories_meta ADD COLUMN {column} {definition}")
        _ensure_fts_table(connection)
        connection.execute(f"PRAGMA user_version={INDEX_VERSION}")
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
    document_id = f"{store.scope}:{memory.id}"
    with closing(_connect(index_path(store))) as connection:
        # Refresh only the FTS row; index_memory upserts the metadata row and
        # preserves its embedding columns. Deleting the meta row here would drop
        # the stored embedding on every refresh.
        connection.execute("DELETE FROM memories_fts WHERE document_id = ?", (document_id,))
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
            if not rows and any(_needs_like_fallback(token) for token in tokenize(query)):
                rows = _search_rows_like(connection, tokenize(query), limit, memory_type, include_archive)
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
    # UPSERT (not INSERT OR REPLACE) so the embedding columns are preserved on
    # update. INSERT OR REPLACE deletes the row and re-inserts it, which would
    # reset embedding/embedding_model/embedding_dim/embedding_mtime to their
    # schema defaults on every metadata refresh (e.g. a search access bump).
    connection.execute(
        """
        INSERT INTO memories_meta (
            document_id, scope, memory_id, path, type, archived, strength, tags, summary, mtime
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(document_id) DO UPDATE SET
            scope=excluded.scope,
            memory_id=excluded.memory_id,
            path=excluded.path,
            type=excluded.type,
            archived=excluded.archived,
            strength=excluded.strength,
            tags=excluded.tags,
            summary=excluded.summary,
            mtime=excluded.mtime
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
    _index_fts(connection, document_id, memory, tags, summary)


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


def _search_rows_like(
    connection: sqlite3.Connection,
    tokens: list[str],
    limit: int,
    memory_type: str,
    include_archive: bool,
) -> list[sqlite3.Row]:
    short_tokens = [token for token in tokens if _needs_like_fallback(token)]
    if not short_tokens:
        return []
    fields = ("title", "summary", "body", "tags", "type")
    token_filters: list[str] = []
    params: list[object] = []
    for token in short_tokens:
        pattern = f"%{_escape_like(token)}%"
        token_filters.append("(" + " OR ".join(f"memories_fts.{field} LIKE ? ESCAPE '\\'" for field in fields) + ")")
        params.extend([pattern] * len(fields))
    filters = ["(" + " OR ".join(token_filters) + ")"]
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
                (m.strength / 1000.0) AS score,
                m.summary AS why_matched
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


def encode_embedding(vector: list[float]) -> bytes:
    if not vector:
        return b""
    return struct.pack(f"<{len(vector)}e", *vector)


def decode_embedding(blob: bytes | None, dimensions: int) -> list[float]:
    if not blob or dimensions <= 0:
        return []
    # Half-float ('e') is 2 bytes; a mismatch means the stored dim and blob have
    # diverged (corruption, partial write, model change). Skip rather than raise
    # struct.error, which would otherwise crash the whole vector lane / reindex.
    if len(blob) != dimensions * 2:
        return []
    return list(struct.unpack(f"<{dimensions}e", blob))


def cosine_similarity(first: list[float], second: list[float]) -> float:
    if not first or len(first) != len(second):
        return 0.0
    dot = sum(left * right for left, right in zip(first, second))
    first_norm = math.sqrt(sum(value * value for value in first))
    second_norm = math.sqrt(sum(value * value for value in second))
    if not first_norm or not second_norm:
        return 0.0
    return dot / (first_norm * second_norm)


def write_embedding(store: Store, memory_id: str, vector: list[float], model_id: str) -> None:
    ensure_index(store)
    with closing(_connect(index_path(store))) as connection:
        connection.execute(
            """
            UPDATE memories_meta
            SET embedding = ?, embedding_model = ?, embedding_dim = ?, embedding_mtime = mtime
            WHERE scope = ? AND memory_id = ?
            """,
            (encode_embedding(vector), model_id, len(vector), store.scope, memory_id),
        )
        connection.commit()


def iter_embeddings(
    stores: Iterable[Store],
    model_id: str,
    dimensions: int,
    include_archive: bool = False,
) -> Iterable[IndexedEmbedding]:
    for store in stores:
        sync_index(store, include_archive=include_archive)
        with closing(_connect(index_path(store))) as connection:
            connection.row_factory = sqlite3.Row
            filters = ["embedding IS NOT NULL", "embedding_model = ?", "embedding_dim = ?"]
            params: list[object] = [model_id, dimensions]
            if not include_archive:
                filters.append("archived = 0")
            rows = connection.execute(
                f"SELECT path, embedding, embedding_dim FROM memories_meta WHERE {' AND '.join(filters)}",
                params,
            ).fetchall()
        for row in rows:
            path = Path(row["path"])
            try:
                memory = parse_memory(path.read_text(encoding="utf-8"))
            except OSError:
                continue
            yield IndexedEmbedding(
                store=store,
                path=path,
                memory=memory,
                vector=decode_embedding(row["embedding"], int(row["embedding_dim"])),
            )


def backfill_embeddings(store: Store, embedder, include_archive: bool = True) -> int:
    sync_index(store, include_archive=include_archive)
    memories = load_memories(store, include_archive=include_archive)
    vectors = call_with_timeout(
        lambda: embedder.embed([memory_search_text(memory) for _, memory in memories]),
        timeout=30.0,
        fallback=[],
    )
    count = 0
    for (_path, memory), vector in zip(memories, vectors):
        if vector is None:
            continue
        write_embedding(store, memory.id, vector, embedder.model_id)
        count += 1
    return count


def _ensure_fts_table(connection: sqlite3.Connection) -> None:
    row = connection.execute(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memories_fts'"
    ).fetchone()
    if row is not None and "trigram" not in (row[0] or "").lower():
        connection.execute("DROP TABLE memories_fts")
        row = None
    if row is None:
        connection.execute(
            """
            CREATE VIRTUAL TABLE memories_fts USING fts5(
                document_id UNINDEXED,
                title,
                summary,
                body,
                tags,
                type,
                tokenize='trigram'
            )
            """
        )
        _rebuild_fts_from_meta(connection)


def _rebuild_fts_from_meta(connection: sqlite3.Connection) -> None:
    rows = connection.execute("SELECT document_id, path, tags, summary FROM memories_meta").fetchall()
    for document_id, path_text, tags, summary in rows:
        try:
            memory = parse_memory(Path(path_text).read_text(encoding="utf-8"))
        except OSError:
            continue
        _index_fts(connection, document_id, memory, tags, summary)


def _index_fts(connection: sqlite3.Connection, document_id: str, memory: Memory, tags: str, summary: str) -> None:
    connection.execute(
        """
        INSERT INTO memories_fts (document_id, title, summary, body, tags, type)
        VALUES (?, ?, ?, ?, ?, ?)
        """,
        (document_id, memory.title, summary, memory_search_text(memory), tags, memory.type),
    )


def _needs_like_fallback(token: str) -> bool:
    return len(token) < 3 and any("\u4e00" <= character <= "\u9fff" for character in token)


def _escape_like(token: str) -> str:
    return token.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_")
