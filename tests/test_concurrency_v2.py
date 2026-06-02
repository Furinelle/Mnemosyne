from __future__ import annotations

import multiprocessing
import os
import unittest
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


if __name__ == "__main__":
    unittest.main()
