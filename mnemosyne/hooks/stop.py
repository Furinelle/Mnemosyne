"""Stop hook: run a dry-run maintain pass and surface core memory promotion candidates."""

from __future__ import annotations

import json
import sys

from mnemosyne.hooks._common import collect_stores, hook_safe
from mnemosyne.lifecycle import MaintainSummary, maintain_memory
from mnemosyne.store import load_config, load_memories


def main() -> None:
    with hook_safe():
        try:
            json.load(sys.stdin)
        except Exception:
            pass
        summary = MaintainSummary()
        for store in collect_stores():
            config = load_config(store)
            for path, memory in load_memories(store, include_archive=False):
                result, candidate = maintain_memory(
                    store,
                    path,
                    memory,
                    config['thresholds'],
                    dry_run=True,
                )
                if result == 'core_candidate' and candidate is not None:
                    summary.core_candidates.append(candidate)
        if not summary.core_candidates:
            return
        lines = ['Mnemosyne: core memory promotion candidates detected:']
        for memory in summary.core_candidates:
            lines.append(f'- {memory.id}: {memory.injection_summary}')
        lines.append('Edit core.md manually to promote them.')
        output = {
            'systemMessage': '\n'.join(lines),
        }
        print(json.dumps(output, ensure_ascii=False))


if __name__ == '__main__':
    main()
