from __future__ import annotations

import unittest
from contextlib import closing
from datetime import date, timedelta
from unittest.mock import patch

from mnemosyne.fusion import search
from mnemosyne.index import _connect, ensure_index, index_path, iter_embeddings, reindex_store, search_index, write_embedding
from mnemosyne.schema import Memory
from mnemosyne.store import ensure_store, load_config, project_store, working_path, write_memory
from tests.helpers import isolated_workspace
from tests.test_fusion import HashEmbedder


class ExpiryTests(unittest.TestCase):
    def test_expired_keyword_hits_do_not_exhaust_pool_before_maintenance(self) -> None:
        for indexed in (False, True):
            for query in ("needle", "认证"):
                with self.subTest(indexed=indexed, query=query), isolated_workspace():
                    store = project_store()
                    ensure_store(store)
                    expired = Memory(
                        id="expired", type="codebase", strength=90,
                        expires="2020-01-01", body=" ".join([query] * 8),
                    )
                    active = Memory(id="active", type="codebase", strength=20, body=query)
                    for memory in (expired, active):
                        write_memory(working_path(store, memory), memory)
                    config = load_config(store)
                    config["search"]["index_enabled"] = indexed
                    config["fusion"]["bm25_pool_size"] = 1
                    config["fusion"]["link_expansion"] = False

                    if indexed:
                        self.assertEqual(["active"], [r.memory.id for r in search_index([store], query, limit=1)])
                    self.assertEqual(["active"], [r.memory.id for r in search([store], query, limit=1, config=config)])
                    history = search([store], query, include_archive=True, config=config)
                    self.assertEqual({"active", "expired"}, {r.memory.id for r in history})
                    self.assertTrue(working_path(store, expired).exists())

    def test_expiry_today_and_free_text_remain_searchable(self) -> None:
        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            for memory_id, expiry in (("today", date.today().isoformat()), ("note", "认证方案重构时失效")):
                memory = Memory(id=memory_id, type="codebase", strength=70, expires=expiry, body="needle")
                write_memory(working_path(store, memory), memory)

            self.assertEqual({"today", "note"}, {r.memory.id for r in search_index([store], "needle")})
            with patch("mnemosyne.schema.date") as clock:
                clock.today.return_value = date.today() + timedelta(days=1)
                self.assertEqual(["note"], [r.memory.id for r in search_index([store], "needle")])

    def test_v3_index_migration_preserves_memory_and_embedding(self) -> None:
        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            memory = Memory(id="old", type="codebase", strength=70, expires="2020-01-01", body="needle")
            memory_path = working_path(store, memory)
            write_memory(memory_path, memory)
            original = memory_path.read_bytes()
            with closing(_connect(index_path(store))) as connection:
                connection.execute("""
                    CREATE TABLE memories_meta (
                        document_id TEXT PRIMARY KEY, scope TEXT NOT NULL, memory_id TEXT NOT NULL,
                        path TEXT NOT NULL, type TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'active',
                        archived INTEGER NOT NULL, strength INTEGER NOT NULL, tags TEXT NOT NULL,
                        summary TEXT NOT NULL, mtime REAL NOT NULL DEFAULT 0, embedding BLOB,
                        embedding_model TEXT NOT NULL DEFAULT '', embedding_dim INTEGER NOT NULL DEFAULT 0,
                        embedding_mtime REAL NOT NULL DEFAULT 0
                    )
                """)
                connection.execute("""
                    INSERT INTO memories_meta (document_id, scope, memory_id, path, type, archived,
                                               strength, tags, summary, mtime, embedding, embedding_model,
                                               embedding_dim, embedding_mtime)
                    VALUES ('project:old', 'project', 'old', ?, 'codebase', 0, 70, '', 'needle', ?, ?,
                            'hash-v1', 2, ?)
                """, (str(memory_path), memory_path.stat().st_mtime, b"\x00\x3c\x00\x00", memory_path.stat().st_mtime))
                connection.execute("PRAGMA user_version=3")
                connection.commit()

            ensure_index(store)

            self.assertEqual([], search_index([store], "needle"))
            self.assertEqual(["old"], [r.memory.id for r in search_index([store], "needle", include_archive=True)])
            embeddings = list(iter_embeddings([store], "hash-v1", 2, include_archive=True))
            self.assertEqual([[1.0, 0.0]], [item.vector for item in embeddings])
            self.assertEqual(original, memory_path.read_bytes())

    def test_expired_vector_hits_do_not_exhaust_pool(self) -> None:
        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            for memory_id, expiry in (("expired", "2020-01-01"), ("active", "")):
                memory = Memory(id=memory_id, type="codebase", strength=70, expires=expiry, body="unrelated")
                write_memory(working_path(store, memory), memory)
            reindex_store(store)
            write_embedding(store, "expired", [1.0, 0.0], "hash-v1")
            write_embedding(store, "active", [0.9, 0.1], "hash-v1")
            config = load_config(store)
            config["fusion"]["vec_pool_size"] = 1
            config["fusion"]["link_expansion"] = False

            results = search([store], "alpha", limit=1, config=config, embedder=HashEmbedder())

            self.assertEqual(["active"], [r.memory.id for r in results])
            history = search([store], "alpha", include_archive=True, config=config, embedder=HashEmbedder())
            self.assertEqual({"active", "expired"}, {r.memory.id for r in history})

    def test_expired_link_target_cannot_expand_further(self) -> None:
        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            source = Memory(id="source", type="codebase", strength=70, body="needle",
                            links=[{"id": "expired", "rel": "related"}])
            expired = Memory(id="expired", type="codebase", strength=70, body="unrelated",
                             expires="2020-01-01", links=[{"id": "tail", "rel": "related"}])
            tail = Memory(id="tail", type="codebase", strength=70, body="other")
            for memory in (source, expired, tail):
                write_memory(working_path(store, memory), memory)
            config = load_config(store)
            config["fusion"]["link_expansion_max_hops"] = 2

            self.assertEqual(["source"], [r.memory.id for r in search([store], "needle", config=config)])
            history = search([store], "needle", include_archive=True, config=config)
            self.assertEqual({"source", "expired", "tail"}, {r.memory.id for r in history})


if __name__ == "__main__":
    unittest.main()
