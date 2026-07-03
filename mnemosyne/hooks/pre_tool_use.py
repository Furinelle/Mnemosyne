"""PreToolUse hook for Edit|Write: inject memories matching the target file basename."""

from __future__ import annotations

import json
import sys
from pathlib import Path

from mnemosyne.hooks._common import (
    collect_stores,
    format_for_injection,
    hook_safe,
    load_injected_ids,
    read_event,
    record_injected_ids,
    run_search,
)
from mnemosyne.store import load_config


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
        session_id = str(event.get('session_id') or '')
        already = load_injected_ids(session_id)
        results = [
            item for item in run_search(basename, limit=2, update_access=False)
            if item['id'] not in already
        ]
        if not results:
            return
        stores = collect_stores()
        config = load_config(stores[-1] if stores else None)
        max_tokens = int(config.get('injection', {}).get('max_tokens', 2000))
        context = f'## Memories relevant to {basename}\n\n' + format_for_injection(results, max_tokens=max_tokens)
        record_injected_ids(session_id, [item['id'] for item in results])
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
