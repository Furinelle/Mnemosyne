"""PreToolUse hook: inject memories relevant to the file a tool is about to touch."""

from __future__ import annotations

import json

from mnemosyne.events import handle_event
from mnemosyne.hooks._common import hook_safe, read_event
from mnemosyne.store import load_config


def main() -> None:
    with hook_safe():
        event = read_event()
        tool_input = event.get('tool_input') or {}
        file_path = tool_input.get('file_path') or ''
        if not file_path:
            return
        tool_name = str(event.get('tool_name', ''))
        write_tools = [
            str(name)
            for name in load_config().get('hooks', {}).get('write_tools', ['Edit', 'Write'])
        ]
        if tool_name not in write_tools:
            return
        result = handle_event(
            'file_touch',
            {'files': [file_path]},
            session=str(event.get('session_id') or ''),
        )
        if not result.context:
            return
        # No permissionDecision: this hook only injects context. Emitting
        # 'allow' here would bypass the user's tool-approval prompt for every
        # matched tool call whose target happens to match a memory.
        output = {
            'hookSpecificOutput': {
                'hookEventName': 'PreToolUse',
                'additionalContext': result.context,
            }
        }
        print(json.dumps(output, ensure_ascii=False))


if __name__ == '__main__':
    main()
