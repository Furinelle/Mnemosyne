"""PreToolUse hook for Edit|Write: inject memories matching the target file basename."""

from __future__ import annotations

import json
import sys
from pathlib import Path

from mnemosyne.hooks._common import format_for_injection, hook_safe, read_event, run_search


def main() -> None:
    with hook_safe():
        event = read_event()
        tool_name = event.get('tool_name', '')
        if tool_name not in ('Edit', 'Write'):
            return
        tool_input = event.get('tool_input') or {}
        file_path = tool_input.get('file_path') or ''
        if not file_path:
            return
        basename = Path(file_path).name
        if not basename:
            return
        results = run_search(basename, limit=2, update_access=False)
        if not results:
            return
        context = f'## Memories relevant to {basename}\n\n' + format_for_injection(results)
        output = {
            'hookSpecificOutput': {
                'hookEventName': 'PreToolUse',
                'permissionDecision': 'allow',
                'additionalContext': context,
            }
        }
        print(json.dumps(output, ensure_ascii=False))


if __name__ == '__main__':
    main()
