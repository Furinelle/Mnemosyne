from __future__ import annotations

import unittest
from pathlib import Path

from mnemosyne.fusion import FusionSearchResult, expand_links, rrf, search
from mnemosyne.index import write_embedding
from mnemosyne.schema import Memory
from mnemosyne.store import ensure_store, load_config, project_store, working_path, write_memory
from tests.helpers import isolated_workspace


def _memory(memory_id: str, body: str, links: list[dict[str, str]] | None = None) -> Memory:
    return Memory(
        id=memory_id,
        type="codebase",
        strength=70,
        links=links or [],
        canonical_summary=body,
        injection_summary=body,
        body=body,
    )


class HashEmbedder:
    dimensions = 2
    model_id = "hash-v1"

    def embed_one(self, text: str) -> list[float]:
        return [1.0, 0.0] if "alpha" in text else [0.0, 1.0]


class PreferSecondReranker:
    model_id = "prefer-second"

    def rerank(self, query: str, docs: list[str]) -> list[float]:
        return [0.9 if "second" in doc else 0.1 for doc in docs]


class FusionTests(unittest.TestCase):
    def test_rerank_keeps_unreranked_tail_behind_reranked_head(self) -> None:
        from mnemosyne.fusion import _rerank
        from mnemosyne.store import Store

        store = Store("project", Path("."))
        results = [
            FusionSearchResult(store=store, path=Path("a.md"), memory=_memory("first", "first doc"), score=6.0),
            FusionSearchResult(store=store, path=Path("b.md"), memory=_memory("second", "second doc"), score=5.0),
            FusionSearchResult(store=store, path=Path("c.md"), memory=_memory("third", "third doc"), score=4.0),
        ]

        reranked = _rerank("query", results, PreferSecondReranker(), top_n=1)

        self.assertEqual(["second", "first", "third"], [item.memory.id for item in reranked])

    def test_rrf_matches_reference_formula(self) -> None:
        scores = rrf([["a", "b"], ["b", "c"]], k=60)

        self.assertAlmostEqual(1 / 61, scores["a"])
        self.assertAlmostEqual(1 / 62 + 1 / 61, scores["b"])
        self.assertAlmostEqual(1 / 62, scores["c"])

    def test_rrf_handles_empty_lanes(self) -> None:
        self.assertEqual({}, rrf([]))
        self.assertEqual({}, rrf([[]]))

    def test_link_expansion_uses_typed_relation_weight(self) -> None:
        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            target = _memory("target", "target body")
            source = _memory(
                "source",
                "source body",
                links=[
                    {"id": "target", "rel": "refines"},
                    {"id": "related-target", "rel": "related"},
                ],
            )
            related = _memory("related-target", "related body")
            for memory in (source, target, related):
                path = working_path(store, memory)
                write_memory(path, memory)
            candidates = {
                "project:source": FusionSearchResult(
                    store=store,
                    path=working_path(store, source),
                    memory=source,
                    score=1.0,
                )
            }

            expanded = expand_links(candidates, [store], {"link_expansion": True})

            self.assertAlmostEqual(0.7, expanded["project:target"].score)
            self.assertAlmostEqual(0.5, expanded["project:related-target"].score)

    def test_link_expansion_marks_contradictions(self) -> None:
        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            target = _memory("target", "target body")
            source = _memory("source", "source body", links=[{"id": "target", "rel": "contradicts"}])
            for memory in (source, target):
                write_memory(working_path(store, memory), memory)
            candidates = {
                "project:source": FusionSearchResult(
                    store=store,
                    path=working_path(store, source),
                    memory=source,
                    score=1.0,
                )
            }

            expanded = expand_links(candidates, [store], {"link_expansion": True})

            self.assertEqual(["source"], expanded["project:target"].score_breakdown["contradicts_with"])

    def test_vector_lane_can_recall_document_without_bm25_match(self) -> None:
        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            first = _memory("first", "unrelated text")
            second = _memory("second", "other words")
            for memory in (first, second):
                path = working_path(store, memory)
                write_memory(path, memory)
            from mnemosyne.index import reindex_store

            reindex_store(store)
            write_embedding(store, first.id, [1.0, 0.0], "hash-v1")
            write_embedding(store, second.id, [0.0, 1.0], "hash-v1")
            config = load_config(store)
            config["embedding"]["enabled"] = True
            config["fusion"]["link_expansion"] = False

            results = search([store], "alpha", limit=1, config=config, embedder=HashEmbedder())

            self.assertEqual(["first"], [result.memory.id for result in results])
            self.assertIn("vec", results[0].score_breakdown)

    def test_type_filter_applies_to_vector_only_candidates(self) -> None:
        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            wrong_type = _memory("vector-wrong-type", "unrelated vector document")
            wanted_type = Memory(
                id="wanted-type",
                type="pitfall",
                strength=70,
                canonical_summary="unrelated pitfall",
                injection_summary="unrelated pitfall",
                body="unrelated pitfall",
            )
            for memory in (wrong_type, wanted_type):
                write_memory(working_path(store, memory), memory)
            from mnemosyne.index import reindex_store

            reindex_store(store)
            write_embedding(store, wrong_type.id, [1.0, 0.0], "hash-v1")
            write_embedding(store, wanted_type.id, [0.0, 1.0], "hash-v1")
            config = load_config(store)
            config["embedding"]["enabled"] = True
            config["fusion"]["link_expansion"] = False

            results = search(
                [store],
                "alpha",
                limit=5,
                type_filter="pitfall",
                config=config,
                embedder=HashEmbedder(),
            )

            self.assertEqual([], results)

    def test_default_search_removes_archive_targets_added_by_links(self) -> None:
        with isolated_workspace():
            from mnemosyne.store import move_to_archive

            store = project_store()
            ensure_store(store)
            archived = _memory("archived-target", "archived target details")
            source = _memory(
                "working-source",
                "active source needle",
                links=[{"id": archived.id, "rel": "related"}],
            )
            archived_path = working_path(store, archived)
            write_memory(archived_path, archived)
            move_to_archive(store, archived_path, archived, "2026-07")
            write_memory(working_path(store, source), source)
            config = load_config(store)
            config["search"]["index_enabled"] = False

            results = search(
                [store], "active source needle", limit=5, include_archive=False, config=config
            )

            self.assertEqual(["working-source"], [result.memory.id for result in results])

    def test_bm25_only_returns_empty_for_empty_query(self) -> None:
        with isolated_workspace():
            config = load_config(project_store())
            config["search"]["index_enabled"] = False

            self.assertEqual([], search([project_store()], "", config=config))

    def test_reranker_can_reorder_top_candidates(self) -> None:
        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            first = _memory("first", "shared token first")
            second = _memory("second", "shared token second")
            for memory in (first, second):
                write_memory(working_path(store, memory), memory)
            config = load_config(store)
            config["search"]["index_enabled"] = False
            config["fusion"]["link_expansion"] = False
            config["rerank"]["enabled"] = True

            results = search([store], "shared token", limit=2, config=config, reranker=PreferSecondReranker())

            self.assertEqual(["second", "first"], [result.memory.id for result in results])
            self.assertEqual(0.9, results[0].score_breakdown["rerank"])


if __name__ == "__main__":
    unittest.main()
