from __future__ import annotations

import io
import unittest
from contextlib import redirect_stdout

from mnemosyne.cli import main
from tests.helpers import isolated_workspace


class DoctorCommandTests(unittest.TestCase):
    def test_doctor_reports_core_checks(self) -> None:
        with isolated_workspace():
            self.assertEqual(0, main(["init"]))
            output = io.StringIO()

            with redirect_stdout(output):
                code = main(["doctor", "--scope", "project"])

            self.assertEqual(0, code)
            text = output.getvalue()
            self.assertIn("portalocker", text)
            self.assertIn("templates", text)
            self.assertIn("index", text)
            self.assertIn("embedder", text)
            self.assertIn("reranker", text)
            self.assertIn("mcp", text)


    def test_doctor_reports_corrupt_files(self) -> None:
        from mnemosyne.store import ensure_store, project_store

        with isolated_workspace():
            self.assertEqual(0, main(["init"]))
            store = project_store()
            ensure_store(store)
            (store.working_dir / "junk.md").write_bytes(b"\xff\xfe not utf8")
            output = io.StringIO()

            with redirect_stdout(output):
                main(["doctor", "--scope", "project"])

            text = output.getvalue()
            self.assertIn("corrupt files", text)
            self.assertIn("junk.md", text)


if __name__ == "__main__":
    unittest.main()
