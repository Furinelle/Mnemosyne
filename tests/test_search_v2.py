from __future__ import annotations

import io
import json
import unittest
from contextlib import redirect_stderr, redirect_stdout

from mnemosyne.cli import main
from mnemosyne.hooks._common import run_search
from mnemosyne.schema import Memory
from mnemosyne.store import ensure_store, project_store, working_path, write_memory
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


class SearchV2Tests(unittest.TestCase):
    def test_cli_search_expands_linked_memory(self) -> None:
        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            source = _memory("source", "unique-source-token", [{"id": "target", "rel": "refines"}])
            target = _memory("target", "target-only")
            for memory in (source, target):
                write_memory(working_path(store, memory), memory)
            output = io.StringIO()

            with redirect_stdout(output):
                code = main(["search", "unique-source-token", "--scope", "project", "--format", "json"])

            self.assertEqual(0, code)
            data = json.loads(output.getvalue())
            self.assertEqual(["source", "target"], [item["id"] for item in data])
            self.assertIn("link_boost", data[1]["score_breakdown"])

    def test_embed_backfill_reports_disabled_without_traceback(self) -> None:
        with isolated_workspace():
            self.assertEqual(0, main(["init"]))
            stderr = io.StringIO()
            stdout = io.StringIO()

            with redirect_stderr(stderr), redirect_stdout(stdout):
                code = main(["embed-backfill", "--scope", "project"])

            self.assertEqual(1, code)
            self.assertIn("embedding is disabled", stderr.getvalue())
            self.assertIn("total: 0", stdout.getvalue())

    def test_hook_search_keeps_default_bm25_behavior(self) -> None:
        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            memory = _memory("hook-target", "hook-search-needle")
            write_memory(working_path(store, memory), memory)

            results = run_search("hook-search-needle", limit=2)

            self.assertEqual(["hook-target"], [item["id"] for item in results])


if __name__ == "__main__":
    unittest.main()
