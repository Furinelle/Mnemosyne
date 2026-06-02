from __future__ import annotations

import sqlite3
import unittest
from contextlib import closing

from mnemosyne.cli import main
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


if __name__ == "__main__":
    unittest.main()
