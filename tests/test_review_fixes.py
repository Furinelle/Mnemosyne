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


class ConsolidateTests(unittest.TestCase):
    def _seed(self):
        self.assertEqual(0, main(["init"]))
        for importance, content in (("80", "use httpOnly cookie for JWT tokens"),
                                    ("60", "use httpOnly cookie for JWT auth tokens")):
            self.assertEqual(0, main([
                "write", "--type", "pitfall", "--importance", importance, "--force",
                "--allow-duplicate", "--title", "JWT cookie storage",
                "--content", content,
            ]))

    def test_consolidate_dry_run_lists_pairs_without_changes(self) -> None:
        import io
        from contextlib import redirect_stdout

        with isolated_workspace():
            self._seed()
            output = io.StringIO()

            with redirect_stdout(output):
                code = main(["consolidate", "--scope", "project"])

            self.assertEqual(0, code)
            self.assertIn("would merge", output.getvalue())
            self.assertEqual(2, len(load_memories(project_store())))

    def test_consolidate_commit_merges_weaker_into_stronger(self) -> None:
        with isolated_workspace():
            self._seed()

            self.assertEqual(0, main(["consolidate", "--scope", "project", "--commit"]))

            memories = load_memories(project_store())
            self.assertEqual(1, len(memories))
            survivor = memories[0][1]
            self.assertEqual(80, survivor.strength)
            self.assertIn("auth tokens", survivor.body)


class SupersededRetrievalTests(unittest.TestCase):
    def _write_pair(self):
        self.assertEqual(0, main(["init"]))
        self.assertEqual(0, main([
            "write", "--type", "arch_decision", "--importance", "60", "--force",
            "--title", "JWT storage decision", "--content", "use localStorage for JWT",
        ]))
        store = project_store()
        old_id = load_memories(store)[0][1].id
        self.assertEqual(0, main([
            "write", "--type", "arch_decision", "--importance", "60", "--force",
            "--title", "JWT storage decision",
            "--content", "switch to httpOnly cookie for JWT storage",
        ]))
        new_id = next(m.id for _, m in load_memories(store) if m.id != old_id)
        return store, old_id, new_id

    def test_superseded_memory_is_marked_and_filtered(self) -> None:
        from mnemosyne.fusion import search as fusion_search

        with isolated_workspace():
            store, old_id, new_id = self._write_pair()
            memories = {m.id: m for _, m in load_memories(store)}
            self.assertEqual("superseded", memories[old_id].status)
            self.assertEqual(new_id, memories[old_id].extra.get("invalidated_by"))

            default_ids = [r.memory.id for r in fusion_search([store], "JWT storage", limit=5)]
            self.assertNotIn(old_id, default_ids)
            self.assertIn(new_id, default_ids)

            all_ids = [
                r.memory.id
                for r in fusion_search([store], "JWT storage", limit=5, include_superseded=True)
            ]
            self.assertIn(old_id, all_ids)

    def test_cli_search_include_superseded_flag(self) -> None:
        import io
        from contextlib import redirect_stdout

        with isolated_workspace():
            store, old_id, _new_id = self._write_pair()
            output = io.StringIO()
            with redirect_stdout(output):
                main(["search", "JWT storage", "--format", "json", "--include-superseded"])
            self.assertIn(old_id, output.getvalue())


class WriteDedupTests(unittest.TestCase):
    def test_classification_is_isolated_to_destination_scope(self) -> None:
        from mnemosyne.codex import Finding
        from mnemosyne.distill import classify_against_store
        from mnemosyne.schema import Memory
        from mnemosyne.store import ensure_store, global_store, working_path, write_memory

        with isolated_workspace():
            project = project_store()
            ensure_store(project)
            global_memory = Memory(
                id="global-copy",
                type="pitfall",
                strength=70,
                body="## JWT note\n\nuse httpOnly cookie for JWT",
            )
            global_scope = global_store()
            ensure_store(global_scope)
            write_memory(working_path(global_scope, global_memory), global_memory)
            finding = Finding(
                "pitfall", 70, "JWT note", ["jwt"], "use httpOnly cookie for JWT"
            )

            verdict, target = classify_against_store(finding, store=project)

            self.assertEqual(("new", None), (verdict, target))

    def test_classification_only_compares_same_memory_type(self) -> None:
        from mnemosyne.codex import Finding
        from mnemosyne.distill import classify_against_store
        from mnemosyne.schema import Memory
        from mnemosyne.store import ensure_store, working_path, write_memory

        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            existing = Memory(
                id="same-text-different-type",
                type="codebase",
                strength=70,
                body="## JWT note\n\nuse httpOnly cookie for JWT",
            )
            write_memory(working_path(store, existing), existing)
            finding = Finding(
                "pitfall", 70, "JWT note", ["jwt"], "use httpOnly cookie for JWT"
            )

            verdict, target = classify_against_store(finding, store=store)

            self.assertEqual(("new", None), (verdict, target))

    def test_force_write_skips_exact_duplicate(self) -> None:
        import io
        from contextlib import redirect_stdout

        with isolated_workspace():
            self.assertEqual(0, main(["init"]))
            args = ["write", "--type", "pitfall", "--importance", "70", "--force",
                    "--title", "JWT note", "--content", "use httpOnly cookie for JWT"]
            self.assertEqual(0, main(args))
            output = io.StringIO()

            with redirect_stdout(output):
                code = main(args)

            self.assertEqual(0, code)
            self.assertIn("Duplicate of", output.getvalue())
            store = project_store()
            self.assertEqual(1, len(load_memories(store)))

    def test_allow_duplicate_writes_anyway(self) -> None:
        with isolated_workspace():
            self.assertEqual(0, main(["init"]))
            args = ["write", "--type", "pitfall", "--importance", "70", "--force",
                    "--title", "JWT note", "--content", "use httpOnly cookie for JWT"]
            self.assertEqual(0, main(args))
            self.assertEqual(0, main(args + ["--allow-duplicate"]))

            self.assertEqual(2, len(load_memories(project_store())))

    def test_same_subject_write_supersedes(self) -> None:
        with isolated_workspace():
            self.assertEqual(0, main(["init"]))
            self.assertEqual(0, main([
                "write", "--type", "arch_decision", "--importance", "60", "--force",
                "--title", "JWT storage decision",
                "--content", "use localStorage for JWT",
            ]))
            store = project_store()
            old_id = load_memories(store)[0][1].id

            self.assertEqual(0, main([
                "write", "--type", "arch_decision", "--importance", "60", "--force",
                "--title", "JWT storage decision",
                "--content", "switch to httpOnly cookie for JWT storage",
            ]))

            memories = {m.id: m for _, m in load_memories(store)}
            self.assertEqual(2, len(memories))
            old = memories[old_id]
            self.assertLess(old.strength, 60, "superseded memory must be demoted")
            self.assertTrue(
                any(link.get("rel") == "superseded_by" for link in old.links),
                "old memory must carry the superseded_by backlink",
            )


class MemoryIndexRewriteTests(unittest.TestCase):
    def test_maintain_rewrites_memory_index(self) -> None:
        from mnemosyne.cli import update_memory_index_file
        from mnemosyne.schema import Memory
        from mnemosyne.store import ensure_store, working_path, write_memory

        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            keep = Memory(id="keep-1", type="codebase", strength=90,
                          injection_summary="stays", body="## keep\n\nstays")
            drop = Memory(id="drop-1", type="pitfall", strength=25,
                          injection_summary="archived away", body="## drop\n\ngoes")
            for memory in (keep, drop):
                write_memory(working_path(store, memory), memory)
                update_memory_index_file(store, memory)

            main(["maintain", "--scope", "project"])

            content = (store.root / "MEMORY.md").read_text(encoding="utf-8")
            self.assertIn("keep-1", content)
            self.assertNotIn("drop-1", content, "archived memories must leave MEMORY.md")


class ExpiresSemanticsTests(unittest.TestCase):
    _THRESHOLDS = {
        "decay_per_run": 1,
        "archive_strength": 30,
        "deprecated_strength": 5,
        "core_strength": 200,
        "core_access_count": 99,
    }

    def _memory(self, expires: str):
        from mnemosyne.schema import Memory

        return Memory(id="m-1", type="pitfall", strength=80, expires=expires, body="## t\n\nbody")

    def test_free_text_expires_never_archives(self) -> None:
        from pathlib import Path

        from mnemosyne.store import Store

        result, _ = maintain_memory(
            Store("project", Path(".")), Path("m-1.md"), self._memory("认证方案重构时失效"),
            self._THRESHOLDS, dry_run=True,
        )

        self.assertEqual("decayed", result)

    def test_iso_expires_archives_past_date(self) -> None:
        from pathlib import Path

        from mnemosyne.store import Store

        result, _ = maintain_memory(
            Store("project", Path(".")), Path("m-1.md"), self._memory("2020-01-01"),
            self._THRESHOLDS, dry_run=True,
        )

        self.assertEqual("archived", result)

    def test_is_date_expiry(self) -> None:
        from mnemosyne.lifecycle import is_date_expiry

        self.assertTrue(is_date_expiry("2026-12-31"))
        self.assertFalse(is_date_expiry("认证方案重构时失效"))
        self.assertFalse(is_date_expiry("2026-1-1"))


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
