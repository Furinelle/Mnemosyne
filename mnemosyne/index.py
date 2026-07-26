"""Persistent SQLite FTS index for Mnemosyne memories."""

from __future__ import annotations

import sqlite3
import math
import struct
import sys
import time
from contextlib import closing
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

import portalocker

from mnemosyne.embedding.base import call_with_timeout
from mnemosyne.schema import Memory, parse_memory
from mnemosyne.search import memory_search_text, tokenize
from mnemosyne.store import Store, iter_memory_paths, load_config, load_memories
from mnemosyne.tokenizer import script_runs


INDEX_FILENAME = "index.sqlite"
INDEX_VERSION = 3


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
    """Create/migrate the index schema, guarded by a file lock.

    Mnemosyne is multi-agent by design, so two processes can call this for
    the very first time on the same fresh store concurrently. WAL +
    busy_timeout only serialize row-level writes; the schema bootstrap below
    is a check-then-create sequence (see _ensure_fts_table) that is not
    atomic across separate connections without an explicit mutex, which used
    to raise "table memories_fts already exists" under real concurrency.
    """
    store.root.mkdir(parents=True, exist_ok=True)
    lock_path = store.root / ".index.lock"
    with portalocker.Lock(str(lock_path), mode="a", timeout=10.0):
        with closing(_connect(index_path(store))) as connection:
            connection.execute(
                """
                CREATE TABLE IF NOT EXISTS memories_meta (
                    document_id TEXT PRIMARY KEY,
                    scope TEXT NOT NULL,
                    memory_id TEXT NOT NULL,
                    path TEXT NOT NULL,
                    type TEXT NOT NULL,
                    status TEXT NOT NULL DEFAULT 'active',
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
                "status": "TEXT NOT NULL DEFAULT 'active'",
                "embedding": "BLOB",
                "embedding_model": "TEXT NOT NULL DEFAULT ''",
                "embedding_dim": "INTEGER NOT NULL DEFAULT 0",
                "embedding_mtime": "REAL NOT NULL DEFAULT 0",
            }
            for column, definition in migrations.items():
                if column not in columns:
                    connection.execute(f"ALTER TABLE memories_meta ADD COLUMN {column} {definition}")
                    if column == "status":
                        _backfill_status(connection)
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


def _backfill_status(connection: sqlite3.Connection) -> None:
    """Populate lifecycle status when migrating a pre-v3 metadata table."""
    rows = connection.execute("SELECT document_id, path FROM memories_meta").fetchall()
    for document_id, path_text in rows:
        try:
            memory = parse_memory(Path(path_text).read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        connection.execute(
            "UPDATE memories_meta SET status = ? WHERE document_id = ?",
            (memory.status, document_id),
        )


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
        by_path: dict[str, list[tuple[str, float]]] = {}
        for document_id, path_text, mtime in rows:
            by_path.setdefault(path_text, []).append((document_id, mtime))
        seen_paths: set[str] = set()
        seen_document_ids: set[str] = set()
        for path in iter_memory_paths(store, include_archive=include_archive):
            spath = str(path)
            try:
                mtime = path.stat().st_mtime
            except OSError:
                continue
            seen_paths.add(spath)
            existing = by_path.get(spath, [])
            if len(existing) == 1 and abs(existing[0][1] - mtime) <= 1e-6:
                seen_document_ids.add(existing[0][0])
                continue
            try:
                memory = parse_memory(path.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                for document_id, _ in existing:
                    delete_document_index(connection, document_id)
                continue
            if not memory.id:
                for document_id, _ in existing:
                    delete_document_index(connection, document_id)
                continue
            document_id = f"{store.scope}:{memory.id}"
            seen_document_ids.add(document_id)
            for old_document_id, _ in existing:
                if old_document_id != document_id:
                    delete_document_index(connection, old_document_id)
            # Replace the searchable text while preserving embeddings on a
            # same-id metadata row. index_memory performs the metadata UPSERT.
            connection.execute("DELETE FROM memories_fts WHERE document_id = ?", (document_id,))
            index_memory(connection, store, path, memory)
        for spath, indexed_rows in by_path.items():
            if spath not in seen_paths:
                for document_id, _ in indexed_rows:
                    # The same stable memory ID may have moved to a new path
                    # earlier in this pass. Its UPSERT already updated path;
                    # deleting the old path's ID here would delete the new row.
                    if document_id not in seen_document_ids:
                        delete_document_index(connection, document_id)
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
    include_superseded: bool = False,
) -> list[IndexedSearchResult]:
    expression = _query_expression(query)
    # Tokens the FTS expression cannot represent (dense-script runs shorter than
    # a trigram) always get their own LIKE pass, whose hits are merged in rather
    # than used only as an all-or-nothing fallback: in a mixed query like
    # "并发锁 portalocker" the English term matches, and gating the fallback on
    # "no rows at all" silently dropped the Chinese half of the query.
    like_tokens = [token for token in tokenize(query) if _needs_like_fallback(token)]
    if not expression and not like_tokens:
        return []

    results: list[IndexedSearchResult] = []
    for store in stores:
        sync_index(store)
        with closing(_connect(index_path(store))) as connection:
            connection.row_factory = sqlite3.Row
            rows = (
                _search_rows(
                    connection,
                    expression,
                    limit,
                    memory_type,
                    include_archive,
                    include_superseded,
                )
                if expression
                else []
            )
            if like_tokens:
                rows = _merge_rows(
                    rows,
                    _search_rows_like(
                        connection,
                        like_tokens,
                        limit,
                        memory_type,
                        include_archive,
                        include_superseded,
                    ),
                    limit,
                )
        for row in rows:
            path = Path(row["path"])
            try:
                memory = parse_memory(path.read_text(encoding="utf-8"))
            except (OSError, ValueError):
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
            document_id, scope, memory_id, path, type, status, archived, strength, tags, summary, mtime
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(document_id) DO UPDATE SET
            scope=excluded.scope,
            memory_id=excluded.memory_id,
            path=excluded.path,
            type=excluded.type,
            status=excluded.status,
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
            memory.status,
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
    delete_document_index(connection, document_id)


def delete_document_index(connection: sqlite3.Connection, document_id: str) -> None:
    """Delete an index row when only its path-associated document ID is known."""
    connection.execute("DELETE FROM memories_meta WHERE document_id = ?", (document_id,))
    connection.execute("DELETE FROM memories_fts WHERE document_id = ?", (document_id,))


def lookup_indexed_memory(
    stores: Iterable[Store], memory_id: str
) -> tuple[Store, Path, Memory] | None:
    """Resolve a memory id via the persistent index instead of scanning files.

    Link expansion resolves every linked id of every candidate; a linear
    re-read of the whole store per link makes search O(candidates x links x N).
    """
    for store in stores:
        if not index_path(store).exists():
            continue
        try:
            with closing(_connect(index_path(store))) as connection:
                row = connection.execute(
                    "SELECT path FROM memories_meta WHERE memory_id = ? LIMIT 1",
                    (memory_id,),
                ).fetchone()
        except sqlite3.Error:
            continue
        if row is None:
            continue
        path = Path(row[0])
        try:
            return store, path, parse_memory(path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
    return None


def _search_rows(
    connection: sqlite3.Connection,
    expression: str,
    limit: int,
    memory_type: str,
    include_archive: bool,
    include_superseded: bool,
) -> list[sqlite3.Row]:
    filters = ["memories_fts MATCH ?"]
    params: list[object] = [expression]
    if memory_type:
        filters.append("m.type = ?")
        params.append(memory_type)
    if not include_archive:
        filters.append("m.archived = 0")
    if not include_superseded:
        filters.append("m.status != 'superseded'")
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
    include_superseded: bool,
) -> list[sqlite3.Row]:
    short_tokens = [token for token in tokens if _needs_like_fallback(token)]
    if not short_tokens:
        return []
    fields = ("title", "summary", "body", "tags", "type")
    token_filters: list[str] = []
    token_params: list[object] = []
    for token in short_tokens:
        pattern = f"%{_escape_like(token)}%"
        token_filters.append("(" + " OR ".join(f"memories_fts.{field} LIKE ? ESCAPE '\\'" for field in fields) + ")")
        token_params.extend([pattern] * len(fields))
    # Rank by how many query tokens a row matches. Ordering by strength alone
    # let a high-strength row that happens to contain one fragment outrank a row
    # matching the whole query.
    hits = " + ".join(f"(CASE WHEN {condition} THEN 1 ELSE 0 END)" for condition in token_filters)
    params: list[object] = [*token_params]
    params.extend(token_params)
    filters = ["(" + " OR ".join(token_filters) + ")"]
    if memory_type:
        filters.append("m.type = ?")
        params.append(memory_type)
    if not include_archive:
        filters.append("m.archived = 0")
    if not include_superseded:
        filters.append("m.status != 'superseded'")
    params.append(limit)
    where = " AND ".join(filters)
    return list(
        connection.execute(
            f"""
            SELECT
                m.path,
                m.summary,
                m.type,
                ({hits}) + (m.strength / 1000.0) AS score,
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


def _merge_rows(primary: list, secondary: list, limit: int) -> list[dict]:
    """Combine FTS and LIKE hits, summing the score of rows both lanes found."""
    merged: dict[str, dict] = {}
    for row in (*primary, *secondary):
        key = str(row["path"])
        existing = merged.get(key)
        if existing is None:
            merged[key] = {
                "path": row["path"],
                "summary": row["summary"],
                "type": row["type"],
                "score": float(row["score"]),
                "why_matched": row["why_matched"],
            }
            continue
        existing["score"] += float(row["score"])
    ordered = sorted(merged.values(), key=lambda item: item["score"], reverse=True)
    return ordered[:limit]


def _query_expression(query: str) -> str:
    """Build an FTS5 MATCH expression for the trigram-tokenized index.

    The trigram tokenizer only matches substrings of three characters or more,
    so feeding it the bigrams `tokenize` produces makes every dense-script
    (e.g. Chinese) query match nothing at all. Emit sliding trigrams for recall
    plus the whole run as an exact-substring signal — bm25 sums the matched
    terms, so a row containing the full run outscores one matching a fragment.
    Runs shorter than three characters are left to the LIKE lane.
    """
    terms: list[str] = []
    for chunk, dense in script_runs(query):
        if not dense:
            terms.append(chunk)
            continue
        if len(chunk) < 3:
            continue
        terms.extend(chunk[index : index + 3] for index in range(len(chunk) - 2))
        if len(chunk) > 3:
            terms.append(chunk)
    if not terms:
        return ""
    return " OR ".join(f'"{term}"' for term in dict.fromkeys(terms))


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
            except (OSError, ValueError):
                continue
            yield IndexedEmbedding(
                store=store,
                path=path,
                memory=memory,
                vector=decode_embedding(row["embedding"], int(row["embedding_dim"])),
            )


def backfill_embeddings(
    store: Store,
    embedder,
    include_archive: bool = True,
    batch_size: int | None = None,
) -> int:
    """Embed memories whose vector is missing, from another model, or stale.

    Incremental: fresh embeddings (embedding_mtime >= file mtime, same model)
    are skipped. Batching gives every chunk its own timeout, so one slow or
    failing call skips that chunk with a warning instead of silently writing
    zero embeddings for the whole store, which is what a single whole-run
    timeout used to do.
    """
    sync_index(store, include_archive=include_archive)
    if batch_size is None:
        batch_size = int(load_config(store).get("embedding", {}).get("batch_size", 32))
    batch_size = max(1, batch_size)
    filters = ["(embedding IS NULL OR embedding_model != ? OR embedding_mtime < mtime)"]
    params: list[object] = [embedder.model_id]
    if not include_archive:
        filters.append("archived = 0")
    if not index_path(store).exists():
        return 0
    with closing(_connect(index_path(store))) as connection:
        connection.row_factory = sqlite3.Row
        rows = connection.execute(
            f"SELECT memory_id, path FROM memories_meta WHERE {' AND '.join(filters)}",
            params,
        ).fetchall()
    count = 0
    for start in range(0, len(rows), batch_size):
        chunk = rows[start : start + batch_size]
        ids: list[str] = []
        texts: list[str] = []
        for row in chunk:
            try:
                memory = parse_memory(Path(row["path"]).read_text(encoding="utf-8"))
            except (OSError, ValueError):
                continue
            ids.append(row["memory_id"])
            texts.append(memory_search_text(memory))
        if not texts:
            continue
        vectors = call_with_timeout(lambda: embedder.embed(texts), timeout=30.0, fallback=None)
        if vectors is None or len(vectors) != len(ids):
            print(
                f"mnemosyne: embedding batch failed; skipped {len(ids)} memories in {store.scope} store",
                file=sys.stderr,
            )
            continue
        for memory_id, vector in zip(ids, vectors):
            if vector:
                write_embedding(store, memory_id, vector, embedder.model_id)
                count += 1
    return count


_FTS_TABLE_SQL = """
CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    document_id UNINDEXED,
    title,
    summary,
    body,
    tags,
    type,
    tokenize='trigram'
)
"""


def _ensure_fts_table(connection: sqlite3.Connection) -> None:
    row = connection.execute(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memories_fts'"
    ).fetchone()
    if row is not None and "trigram" not in (row[0] or "").lower():
        connection.execute("DROP TABLE IF EXISTS memories_fts")
        row = None
    if row is None:
        # IF NOT EXISTS is the real guard against the concurrent-bootstrap
        # race (see ensure_index's docstring); the SELECT above is only a
        # cheap fast path to skip work when the table is already current.
        connection.execute(_FTS_TABLE_SQL)
        _rebuild_fts_from_meta(connection)


def _rebuild_fts_from_meta(connection: sqlite3.Connection) -> None:
    rows = connection.execute("SELECT document_id, path, tags, summary FROM memories_meta").fetchall()
    for document_id, path_text, tags, summary in rows:
        try:
            memory = parse_memory(Path(path_text).read_text(encoding="utf-8"))
        except (OSError, ValueError):
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
    """True for dense-script tokens too short for the trigram tokenizer to match."""
    if len(token) >= 3:
        return False
    runs = script_runs(token)
    return bool(runs) and runs[0][1]


def _escape_like(token: str) -> str:
    return token.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_")
