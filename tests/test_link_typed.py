from __future__ import annotations

import io
import unittest
from contextlib import redirect_stderr, redirect_stdout

from mnemosyne.cli import main
from mnemosyne.schema import Memory
from mnemosyne.store import ensure_store, load_memories, project_store, working_path, write_memory
from tests.helpers import isolated_workspace


def _memory(memory_id: str) -> Memory:
    return Memory(id=memory_id, type="codebase", strength=70, body=f"## {memory_id}")


class TypedLinkTests(unittest.TestCase):
    def test_asymmetric_relation_writes_reverse_relation(self) -> None:
        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            for memory in (_memory("new"), _memory("old")):
                write_memory(working_path(store, memory), memory)

            self.assertEqual(0, main(["link", "new", "old", "--rel", "supersedes"]))

            memories = {memory.id: memory for _, memory in load_memories(store)}
            self.assertEqual([{"id": "old", "rel": "supersedes"}], memories["new"].links)
            self.assertEqual([{"id": "new", "rel": "superseded_by"}], memories["old"].links)

    def test_symmetric_relation_writes_same_relation(self) -> None:
        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            for memory in (_memory("a"), _memory("b")):
                write_memory(working_path(store, memory), memory)

            self.assertEqual(0, main(["link", "a", "b", "--rel", "contradicts"]))

            memories = {memory.id: memory for _, memory in load_memories(store)}
            self.assertEqual("contradicts", memories["a"].links[0]["rel"])
            self.assertEqual("contradicts", memories["b"].links[0]["rel"])

    def test_custom_relation_requires_explicit_flag(self) -> None:
        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            for memory in (_memory("a"), _memory("b")):
                write_memory(working_path(store, memory), memory)
            stderr = io.StringIO()

            with redirect_stderr(stderr):
                code = main(["link", "a", "b", "--rel", "custom"])

            self.assertEqual(2, code)
            self.assertIn("--allow-custom", stderr.getvalue())
            self.assertEqual(0, main(["link", "a", "b", "--rel", "custom", "--allow-custom"]))

    def test_graph_cli_outputs_mermaid(self) -> None:
        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            for memory in (_memory("a"), _memory("b")):
                write_memory(working_path(store, memory), memory)
            self.assertEqual(0, main(["link", "a", "b"]))
            output = io.StringIO()

            with redirect_stdout(output):
                code = main(["graph", "a", "--format", "mermaid"])

            self.assertEqual(0, code)
            self.assertIn("graph LR", output.getvalue())
            self.assertIn("related", output.getvalue())


if __name__ == "__main__":
    unittest.main()
