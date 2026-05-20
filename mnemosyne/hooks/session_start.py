"""SessionStart hook: inject core memory at session start."""

from __future__ import annotations

import json
import sys

from mnemosyne.hooks._common import collect_stores, hook_safe
from mnemosyne.store import read_core


def main() -> None:
    with hook_safe():
        event = json.load(sys.stdin)
        source = event.get('source', 'startup')
        if source in ('resume', 'compact'):
            return
        parts: list[str] = []
        for store in collect_stores():
            content = read_core(store).strip()
            if not content:
                continue
            label = 'Global Core' if store.scope == 'global' else 'Project Core'
            parts.append(f'### {label}\n{content}')
        if not parts:
            return
        output = {
            'hookSpecificOutput': {
                'hookEventName': 'SessionStart',
                'additionalContext': '## Mnemosyne Memory\n\n' + '\n\n'.join(parts),
            }
        }
        print(json.dumps(output, ensure_ascii=False))


if __name__ == '__main__':
    main()
