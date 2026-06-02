from __future__ import annotations

import json
import unittest

from mnemosyne.graph import build_graph, render_ascii, render_graph, render_mermaid
from mnemosyne.schema import Memory
from mnemosyne.store import ensure_store, project_store, working_path, write_memory
from tests.helpers import isolated_workspace


def _memory(memory_id: str, links: list[dict[str, str]] | None = None) -> Memory:
    return Memory(
        id=memory_id,
        type="codebase",
        strength=70,
        links=links or [],
        canonical_summary=f"summary {memory_id}",
        injection_summary=f"summary {memory_id}",
        body=f"## Title {memory_id}",
    )


class GraphTests(unittest.TestCase):
    def test_bfs_honors_depth_and_breaks_cycles(self) -> None:
        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            memories = [
                _memory("a", [{"id": "b", "rel": "refines"}]),
                _memory("b", [{"id": "c", "rel": "related"}]),
                _memory("c", [{"id": "a", "rel": "related"}]),
            ]
            for memory in memories:
                write_memory(working_path(store, memory), memory)

            graph = build_graph("a", [store], depth=2)

            self.assertEqual(["a", "b", "c"], [node["id"] for node in graph["nodes"]])
            self.assertEqual(2, len(graph["edges"]))

    def test_mermaid_ascii_and_json_renderers(self) -> None:
        graph = {
            "root": "a",
            "nodes": [
                {"id": "a", "title": "Title a", "type": "codebase", "scope": "project"},
                {"id": "b", "title": "Title b", "type": "pitfall", "scope": "project"},
            ],
            "edges": [{"source": "a", "target": "b", "rel": "refines"}],
        }

        mermaid = render_mermaid(graph)
        ascii_graph = render_ascii(graph)
        encoded = render_graph(graph, "json")

        self.assertIn("graph LR", mermaid)
        self.assertIn('a["Title a"] -- refines --> b["Title b"]', mermaid)
        self.assertIn("a", ascii_graph)
        self.assertIn("  -> [refines] b", ascii_graph)
        self.assertEqual(graph, json.loads(encoded))


if __name__ == "__main__":
    unittest.main()
