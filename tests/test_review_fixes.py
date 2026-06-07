from __future__ import annotations

import unittest

from mnemosyne.cli import main
from mnemosyne.hooks._common import _approx_tokens
from mnemosyne.lifecycle import maintain_memory
from mnemosyne.schema import _format_scalar, parse_value
from mnemosyne.store import load_config, load_memories, project_store
from tests.helpers import isolated_workspace


class ReviewFixTests(unittest.TestCase):
    def test_supersede_demotes_target(self) -> None:
        with isolated_workspace():
            self.assertEqual(0, main(["init"]))
            self.assertEqual(
                0,
                main(["write", "--type", "codebase", "--importance", "60",
                      "--force", "--content", "old approach alpha"]),
            )
            store = project_store()
            old_id = load_memories(store)[0][1].id
            self.assertEqual(
                0,
                main(["write", "--type", "codebase", "--importance", "60",
                      "--force", "--content", "new approach beta"]),
            )
            new_id = next(m.id for _, m in load_memories(store) if m.id != old_id)

            # "new supersedes old" -> old (the target) should be demoted.
            self.assertEqual(0, main(["link", new_id, old_id, "--rel", "supersedes"]))

            reloaded = {m.id: m for _, m in load_memories(store)}
            self.assertLess(reloaded[old_id].strength, 60)
            self.assertEqual(60, reloaded[new_id].strength)

    def test_expired_memory_is_archived(self) -> None:
        with isolated_workspace():
            self.assertEqual(0, main(["init"]))
            self.assertEqual(
                0,
                main(["write", "--type", "preference", "--importance", "90",
                      "--force", "--expires", "2020-01-01", "--content", "stale pref"]),
            )
            store = project_store()
            path, memory = load_memories(store)[0]
            thresholds = load_config(store).get("thresholds", {})

            status, _ = maintain_memory(store, path, memory, thresholds, dry_run=True)
            self.assertEqual("archived", status)

    def test_approx_tokens_counts_cjk_heavier(self) -> None:
        # 20 CJK chars should estimate well above the naive len//4 == 5.
        self.assertGreaterEqual(_approx_tokens("中" * 20), 10)
        # ASCII stays at the ~4-chars-per-token estimate.
        self.assertEqual(10, _approx_tokens("a" * 40))

    def test_quoted_scalar_roundtrips_backslash_and_quote(self) -> None:
        original = r'path C:\dir, say "hi"'
        self.assertEqual(original, parse_value(_format_scalar(original)))


if __name__ == "__main__":
    unittest.main()
