"""UserPromptSubmit hook: inject relevant memories via the turn_start event."""

from __future__ import annotations

import json

from mnemosyne.events import handle_event
from mnemosyne.hooks._common import hook_safe, read_event


def main() -> None:
    with hook_safe():
        event = read_event()
        result = handle_event(
            'turn_start',
            {'prompt': event.get('prompt') or ''},
            session=str(event.get('session_id') or ''),
        )
        if not result.context:
            return
        output = {
            'hookSpecificOutput': {
                'hookEventName': 'UserPromptSubmit',
                'additionalContext': result.context,
            }
        }
        print(json.dumps(output, ensure_ascii=False))


if __name__ == '__main__':
    main()
