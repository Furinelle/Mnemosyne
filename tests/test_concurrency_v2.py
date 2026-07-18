from __future__ import annotations

import multiprocessing
import os
import time
import unittest
from argparse import Namespace
from types import SimpleNamespace
from pathlib import Path

from mnemosyne.index import iter_embeddings
from mnemosyne.store import project_store
from tests.helpers import isolated_workspace


def _write_vector(root: str, home: str, memory_id: str) -> None:
    os.chdir(root)
    os.environ["MNEMOSYNE_HOME"] = home
    from mnemosyne.index import update_memory_index, write_embedding
    from mnemosyne.schema import Memory
    from mnemosyne.store import ensure_store, project_store, working_path, write_memory

    store = project_store()
    ensure_store(store)
    memory = Memory(
        id=memory_id,
        type="codebase",
        strength=70,
        canonical_summary=f"concurrent {memory_id}",
        injection_summary=f"concurrent {memory_id}",
        body=f"concurrent {memory_id}",
    )
    path = working_path(store, memory)
    write_memory(path, memory)
    update_memory_index(store, path, memory)
    write_embedding(store, memory.id, [1.0, 0.0], "concurrent-hash")


def _race_maintenance(root: str, home: str, start: multiprocessing.synchronize.Event) -> None:
    os.chdir(root)
    os.environ["MNEMOSYNE_HOME"] = home

    import portalocker

    from mnemosyne.hooks import session_start

    calls_path = Path(home) / "maintain-calls"

    def record_spawn(scope: str) -> bool:
        lock_path = calls_path.with_suffix(".lock")
        with portalocker.Lock(str(lock_path), mode="a", timeout=5):
            with calls_path.open("a", encoding="utf-8") as handle:
                handle.write(scope + "\n")
        # Keep the pre-fix check/spawn/mark window open long enough for every
        # process to observe the missing marker.
        time.sleep(0.1)
        return True

    session_start._spawn_maintain = record_spawn
    start.wait(5)
    session_start.maybe_run_maintain()


def _write_concurrent_supersede(
    root: str,
    home: str,
    index: int,
    start: multiprocessing.synchronize.Event,
) -> None:
    os.chdir(root)
    os.environ["MNEMOSYNE_HOME"] = home

    from mnemosyne.codex import Finding
    from mnemosyne.distill import process_finding
    from mnemosyne.store import project_store

    start.wait(5)
    process_finding(
        Finding(
            type="arch_decision",
            importance=70,
            title="Concurrent decision",
            tags=["race"],
            content=f"choice{index}",
        ),
        source="test",
        commit=True,
        store=project_store(),
        subject_threshold=0.5,
    )


class ConcurrencyV2Tests(unittest.TestCase):
    def test_two_processes_can_write_embedding_rows(self) -> None:
        with isolated_workspace() as (project, home):
            context = multiprocessing.get_context("fork")
            workers = [
                context.Process(target=_write_vector, args=(str(project), str(home), f"memory-{index}"))
                for index in range(2)
            ]
            for worker in workers:
                worker.start()
            for worker in workers:
                worker.join(10)

            self.assertEqual([0, 0], [worker.exitcode for worker in workers])
            rows = list(iter_embeddings([project_store()], "concurrent-hash", 2))
            self.assertEqual(["memory-0", "memory-1"], sorted(row.memory.id for row in rows))

    def test_access_bump_does_not_overwrite_a_newer_supersedes_update(self) -> None:
        with isolated_workspace() as (_project, _home):
            from mnemosyne.cli import cmd_link, print_search_results
            from mnemosyne.schema import Memory, parse_memory
            from mnemosyne.store import ensure_store, load_config, working_path, write_memory

            store = project_store()
            ensure_store(store)
            newer = Memory(
                id="newer", type="pitfall", strength=90,
                canonical_summary="new repair", injection_summary="new repair", body="new repair",
            )
            older = Memory(
                id="older", type="pitfall", strength=70,
                canonical_summary="old repair", injection_summary="old repair", body="old repair",
            )
            newer_path = working_path(store, newer)
            older_path = working_path(store, older)
            write_memory(newer_path, newer)
            write_memory(older_path, older)

            # This is the object a search loaded before the relationship write.
            stale_result = SimpleNamespace(
                store=store,
                path=older_path,
                memory=older,
                score=1.0,
                why_matched="old",
                score_breakdown={},
            )
            self.assertEqual(
                0,
                cmd_link(Namespace(id1="newer", id2="older", rel="supersedes", allow_custom=False)),
            )

            print_search_results([stale_result], "json", load_config(store))

            current = parse_memory(older_path.read_text(encoding="utf-8"))
            self.assertEqual("superseded", current.status)
            self.assertEqual("newer", current.extra.get("invalidated_by"))
            self.assertIn({"id": "newer", "rel": "superseded_by"}, current.links)
            self.assertEqual(1, current.access_count)

    def test_concurrent_session_starts_schedule_maintenance_once(self) -> None:
        with isolated_workspace() as (project, home):
            context = multiprocessing.get_context("fork")
            start = context.Event()
            workers = [
                context.Process(
                    target=_race_maintenance,
                    args=(str(project), str(home), start),
                )
                for _ in range(6)
            ]
            for worker in workers:
                worker.start()
            start.set()
            for worker in workers:
                worker.join(10)

            self.assertEqual([0] * len(workers), [worker.exitcode for worker in workers])
            calls = (home / "maintain-calls").read_text(encoding="utf-8").splitlines()
            self.assertEqual(["global"], calls)

    def test_concurrent_same_subject_updates_leave_one_active_head(self) -> None:
        with isolated_workspace() as (project, home):
            from mnemosyne.codex import Finding, write_finding
            from mnemosyne.store import ensure_store, load_memories

            store = project_store()
            ensure_store(store)
            write_finding(
                Finding(
                    type="arch_decision",
                    importance=70,
                    title="Concurrent decision",
                    tags=["race"],
                    content="initial",
                ),
                "test",
                store=store,
            )
            context = multiprocessing.get_context("fork")
            start = context.Event()
            workers = [
                context.Process(
                    target=_write_concurrent_supersede,
                    args=(str(project), str(home), index, start),
                )
                for index in range(6)
            ]
            for worker in workers:
                worker.start()
            start.set()
            for worker in workers:
                worker.join(10)

            self.assertEqual([0] * len(workers), [worker.exitcode for worker in workers])
            active = [memory for _, memory in load_memories(store) if memory.status == "active"]
            self.assertEqual(1, len(active))


if __name__ == "__main__":
    unittest.main()
