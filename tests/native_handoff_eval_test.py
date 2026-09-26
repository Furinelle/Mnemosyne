"""Fail-closed checks for the NX04 simulated contract scorer."""

import tempfile
import os
from pathlib import Path
import sys
import time
import unittest

from native_handoff_eval import invoke, score, summarize, validate_receipt


class ContractTests(unittest.TestCase):
    def setUp(self):
        self.receipt = {"memory_ref": {"store_id": "s", "memory_id": "m"},
                        "revision": 1, "checkpoint_id": "c", "observed_failure": 1}
        self.observed = {"memories": [{"memory_ref": self.receipt["memory_ref"],
                        "revision": {"semantic_rev": 1},
                        "memory": {"body": "Quartz-J9 uses SQLite on port 7429."},
                        "provenance": {"source_events": [{"origin": "native-cli-fixture"}]}}],
                         "checkpoints": [{"checkpoint": {"id": "c", "data": {
                             "unresolved": ["synthetic test failed"],
                             "next_action": "Fix the failing synthetic test"}},
                             "test_results": [{"status": "reported_fail"}]}],
                         "answer": {"next_action": "Fix the failing synthetic test"},
                         "context_bytes": 100}

    def test_exit_zero_without_native_save_is_failure(self):
        with tempfile.TemporaryDirectory() as temp:
            receipt = invoke([sys.executable, "-c", "print('{}')"], Path(temp), {})
        self.assertEqual(receipt.strip(), "{}")
        with self.assertRaisesRegex(RuntimeError, "producer_saved_nothing"):
            validate_receipt("failed_test", {})

    def test_valid_json_wrong_next_action_is_quality_failure(self):
        self.observed["answer"]["next_action"] = "Declare done"
        row = score("failed_test", self.receipt, self.observed, "controller-secret")
        self.assertEqual((row["protocol_status"], row["quality_status"]),
                         ("valid", "failed"))

    def test_controller_canary_leak_invalidates_protocol(self):
        self.observed["memories"][0]["memory"]["body"] += " controller-secret"
        row = score("failed_test", self.receipt, self.observed, "controller-secret")
        self.assertEqual(row["protocol_status"], "canary_leak")

    def test_malformed_native_view_invalidates_protocol(self):
        self.observed["memories"][0]["memory"] = None
        row = score("failed_test", self.receipt, self.observed, "controller-secret")
        self.assertEqual(row["protocol_status"], "bad_schema")

    def test_timeout_and_missing_row_cannot_pass(self):
        with tempfile.TemporaryDirectory() as temp:
            nested = ("import os, pathlib, time; "
                      "pathlib.Path('child.pid').write_text(str(os.getpid())); time.sleep(30)")
            child = (f"import sys; sys.path.insert(0, {str(Path(__file__).resolve().parent)!r}); "
                     "import native_handoff_eval as h; from pathlib import Path; "
                     "h.WORKER_PROCESS = True; "
                     f"h.invoke([sys.executable, '-c', {nested!r}], Path.cwd(), {{}})")
            with self.assertRaisesRegex(RuntimeError, "timeout"):
                invoke([sys.executable, "-c", child], Path(temp), {}, timeout=0.2)
            pid = int((Path(temp) / "child.pid").read_text())
            for _ in range(20):
                try:
                    os.kill(pid, 0)
                    stat = Path(f"/proc/{pid}/stat")
                    if stat.exists() and stat.read_text().split(") ", 1)[1].startswith("Z "):
                        break  # Dead Linux orphan awaiting init reaping.
                except ProcessLookupError:
                    break
                time.sleep(0.01)
            else:
                self.fail("timed-out worker left a descendant alive")
        rows = [{"direction": "cli_to_mcp", "case": "failed_test",
                 "invocation_status": "completed", "protocol_status": "valid",
                 "quality_status": "passed"}]
        self.assertEqual(summarize(rows), {"execution_complete": False,
                                           "acceptance_passed": False})
        rows *= 8
        self.assertEqual(summarize(rows), {"execution_complete": False,
                                           "acceptance_passed": False})

    def test_output_is_bounded(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaisesRegex(RuntimeError, "output_limit"):
                invoke([sys.executable, "-c", "print('x' * 70000)"], Path(temp), {})


if __name__ == "__main__":
    unittest.main()
