from __future__ import annotations

import sqlite3
import unittest
from contextlib import closing

from mnemosyne.cli import main, make_memory_id
from mnemosyne.index import (
    INDEX_VERSION,
    _connect,
    cosine_similarity,
    decode_embedding,
    encode_embedding,
    ensure_index,
    index_path,
    iter_embeddings,
    search_index,
    update_memory_index,
    write_embedding,
)
from mnemosyne.store import load_memories, project_store
from tests.helpers import isolated_workspace


class IndexV2Tests(unittest.TestCase):
    def test_ensure_index_migrates_v1_metadata_columns(self) -> None:
        with isolated_workspace():
            store = project_store()
            store.root.mkdir()
            with closing(_connect(index_path(store))) as connection:
                connection.execute(
                    """
                    CREATE TABLE memories_meta (
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
                connection.commit()

            ensure_index(store)

            with closing(_connect(index_path(store))) as connection:
                columns = {row[1] for row in connection.execute("PRAGMA table_info(memories_meta)")}
                version = connection.execute("PRAGMA user_version").fetchone()[0]
            self.assertEqual(INDEX_VERSION, version)
            self.assertTrue({"embedding", "embedding_model", "embedding_dim", "embedding_mtime"} <= columns)

    def test_float16_blob_round_trip_and_cosine(self) -> None:
        vector = [1.0, -0.5, 0.25]

        restored = decode_embedding(encode_embedding(vector), len(vector))

        self.assertAlmostEqual(1.0, restored[0], places=3)
        self.assertAlmostEqual(-0.5, restored[1], places=3)
        self.assertAlmostEqual(1.0, cosine_similarity([1.0, 0.0], [2.0, 0.0]), places=6)
        self.assertEqual(0.0, cosine_similarity([], []))

    def test_written_embedding_is_returned_for_matching_model(self) -> None:
        with isolated_workspace():
            self.assertEqual(0, main(["init"]))
            self.assertEqual(
                0,
                main(
                    [
                        "write",
                        "--type",
                        "codebase",
                        "--importance",
                        "70",
                        "--force",
                        "--content",
                        "vector storage smoke",
                    ]
                ),
            )
            store = project_store()
            memory = load_memories(store)[0][1]

            write_embedding(store, memory.id, [1.0, 0.0], "hash-v1")

            rows = list(iter_embeddings([store], "hash-v1", 2))
            self.assertEqual(1, len(rows))
            self.assertEqual(memory.id, rows[0].memory.id)
            self.assertEqual([], list(iter_embeddings([store], "other-model", 2)))

    def test_trigram_index_falls_back_for_two_character_cjk_query(self) -> None:
        with isolated_workspace():
            self.assertEqual(0, main(["init"]))
            self.assertEqual(
                0,
                main(
                    [
                        "write",
                        "--type",
                        "pitfall",
                        "--importance",
                        "80",
                        "--force",
                        "--content",
                        "调试认证失败需要检查令牌",
                    ]
                ),
            )

            results = search_index([project_store()], "认证", limit=3)

            self.assertEqual(1, len(results))
            self.assertIn("认证", results[0].memory.body)

    def test_embedding_survives_metadata_reindex(self) -> None:
        with isolated_workspace():
            self.assertEqual(0, main(["init"]))
            self.assertEqual(
                0,
                main(
                    [
                        "write",
                        "--type",
                        "codebase",
                        "--importance",
                        "70",
                        "--force",
                        "--content",
                        "vector survives reindex",
                    ]
                ),
            )
            store = project_store()
            path, memory = load_memories(store)[0]
            write_embedding(store, memory.id, [1.0, 0.0], "hash-v1")

            # A search bumps strength/access and re-indexes the metadata row.
            memory.strength = min(100, memory.strength + 5)
            update_memory_index(store, path, memory)

            rows = list(iter_embeddings([store], "hash-v1", 2))
            self.assertEqual(1, len(rows))
            self.assertEqual(memory.id, rows[0].memory.id)

    def test_decode_embedding_rejects_length_mismatch(self) -> None:
        blob = encode_embedding([1.0, 0.0, 0.5])  # 3 dims, 6 bytes
        self.assertEqual([], decode_embedding(blob, 4))  # claims 4 dims
        self.assertEqual(3, len(decode_embedding(blob, 3)))

    def test_negative_importance_clamped_to_zero(self) -> None:
        with isolated_workspace():
            self.assertEqual(0, main(["init"]))
            self.assertEqual(
                0,
                main(
                    [
                        "write",
                        "--type",
                        "pitfall",
                        "--importance=-50",
                        "--force",
                        "--content",
                        "clamp me",
                    ]
                ),
            )
            memory = load_memories(project_store())[0][1]
            self.assertEqual(0, memory.strength)

    def test_make_memory_id_uses_long_unique_suffix(self) -> None:
        sample = make_memory_id("pitfall", "2026-06-07")
        self.assertTrue(sample.startswith("pitfall-2026-06-07-"))
        self.assertEqual(8, len(sample.rsplit("-", 1)[1]))
        ids = {make_memory_id("pitfall", "2026-06-07") for _ in range(2000)}
        self.assertEqual(2000, len(ids))


class FtsBootstrapRaceTests(unittest.TestCase):
    def test_concurrent_fts_table_creation_does_not_raise(self) -> None:
        """Reproduces the CI failure: two forked processes both check
        sqlite_master, both see no memories_fts table, and both proceed to
        create it (mnemosyne/index.py:_ensure_fts_table's SELECT-then-CREATE
        is not atomic across connections). The check step is timing-dependent
        (30/30 local runs passed while the same race failed on GitHub's Linux
        runners), but the CREATE statement itself -- the actual production SQL,
        via _FTS_TABLE_SQL -- is what must tolerate being run twice. Executing
        it on two connections deterministically exercises that without relying
        on OS scheduling luck."""
        from mnemosyne.index import _FTS_TABLE_SQL

        with isolated_workspace():
            store = project_store()
            store.root.mkdir(parents=True, exist_ok=True)
            path = index_path(store)
            connection_a = _connect(path)
            connection_b = _connect(path)
            try:
                # Both connections observe "no table yet" -- the real race window.
                def sees_no_table(connection: sqlite3.Connection) -> bool:
                    return connection.execute(
                        "SELECT name FROM sqlite_master WHERE type='table' AND name='memories_fts'"
                    ).fetchone() is None

                self.assertTrue(sees_no_table(connection_a))
                self.assertTrue(sees_no_table(connection_b))

                # Connection A wins the race and commits the table first.
                connection_a.execute(_FTS_TABLE_SQL)
                connection_a.commit()

                # Connection B already decided (above) that the table was
                # missing and proceeds to create it too -- must not raise
                # "table memories_fts already exists".
                connection_b.execute(_FTS_TABLE_SQL)
                connection_b.commit()
            finally:
                connection_a.close()
                connection_b.close()


class LookupIndexedMemoryTests(unittest.TestCase):
    def test_lookup_via_index_and_missing_id(self) -> None:
        from mnemosyne.index import lookup_indexed_memory, reindex_store
        from mnemosyne.schema import Memory
        from mnemosyne.store import ensure_store, working_path, write_memory

        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            memory = Memory(id="target-1", type="codebase", strength=70, body="## t\n\nbody")
            write_memory(working_path(store, memory), memory)
            reindex_store(store)

            found = lookup_indexed_memory([store], "target-1")

            self.assertIsNotNone(found)
            self.assertEqual("target-1", found[2].id)
            self.assertIsNone(lookup_indexed_memory([store], "missing-1"))

    def test_lookup_without_index_returns_none(self) -> None:
        from mnemosyne.index import lookup_indexed_memory
        from mnemosyne.store import ensure_store

        with isolated_workspace():
            store = project_store()
            ensure_store(store)

            self.assertIsNone(lookup_indexed_memory([store], "anything"))


if __name__ == "__main__":
    unittest.main()
