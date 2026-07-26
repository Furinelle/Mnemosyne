from __future__ import annotations

import unittest

from mnemosyne.cli import main
from tests.helpers import isolated_workspace


class IndexCommandTests(unittest.TestCase):
    def test_reindex_creates_fts_index_and_search_uses_it(self) -> None:
        with isolated_workspace() as (project, _home):
            self.assertEqual(0, main(["init"]))
            self.assertEqual(
                0,
                main(
                    [
                        "write",
                        "--type",
                        "codebase",
                        "--importance",
                        "72",
                        "--source",
                        "test",
                        "--force",
                        "--title",
                        "FTS search smoke",
                        "--tags",
                        "fts,smoke",
                        "--content",
                        "persistent sqlite fts index should find this unique needle",
                    ]
                ),
            )

            index_path = project / ".mnemosyne" / "index.sqlite"
            if index_path.exists():
                index_path.unlink()

            self.assertEqual(0, main(["reindex", "--scope", "project"]))
            self.assertTrue(index_path.exists())

            from mnemosyne.index import search_index
            from mnemosyne.store import project_store

            results = search_index([project_store()], "unique needle", limit=3)
            self.assertEqual(1, len(results))
            self.assertEqual("codebase", results[0].memory.type)


if __name__ == "__main__":
    unittest.main()

