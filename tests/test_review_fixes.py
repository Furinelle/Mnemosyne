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


class CorruptFileToleranceTests(unittest.TestCase):
    def test_parse_memory_tolerates_non_numeric_counters(self) -> None:
        from mnemosyne.schema import parse_memory

        text = "---\nid: bad-1\ntype: pitfall\nstrength: high\naccess_count: many\n---\n\n## t\n\nbody\n"

        memory = parse_memory(text)

        self.assertEqual(0, memory.strength)
        self.assertEqual(0, memory.access_count)
        self.assertEqual("bad-1", memory.id)

    def test_load_memories_skips_undecodable_file(self) -> None:
        from mnemosyne.schema import Memory
        from mnemosyne.store import ensure_store, working_path, write_memory

        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            good = Memory(id="good-1", type="codebase", strength=70, body="## ok\n\nfine")
            write_memory(working_path(store, good), good)
            (store.working_dir / "junk.md").write_bytes(b"\xff\xfe\x00 not utf8")

            memories = load_memories(store)

            self.assertEqual(["good-1"], [memory.id for _, memory in memories])

    def test_corrupt_memory_paths_reports_bad_files(self) -> None:
        from mnemosyne.store import corrupt_memory_paths, ensure_store

        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            (store.working_dir / "junk.md").write_bytes(b"\xff\xfe\x00 not utf8")
            (store.working_dir / "no_id.md").write_text("---\ntype: pitfall\n---\n\nbody\n", encoding="utf-8")

            bad = corrupt_memory_paths(store)

            self.assertEqual({"junk.md", "no_id.md"}, {path.name for path in bad})


if __name__ == "__main__":
    unittest.main()
