"""UserPromptSubmit hook: inject top-3 relevant memories based on prompt keywords."""

from __future__ import annotations

import json
import sys

from mnemosyne.hooks._common import (
    extract_keywords,
    format_for_injection,
    hook_safe,
    read_event,
    run_search,
)


def main() -> None:
    with hook_safe():
        event = read_event()
        prompt = (event.get('prompt') or '').strip()
        if len(prompt) < 10:
            return
        keywords = extract_keywords(prompt, limit=8)
        if not keywords:
            return
        results = run_search(' '.join(keywords), limit=3, update_access=True)
        if not results:
            return
        context = format_for_injection(results)
        if not context:
            return
        output = {
            'hookSpecificOutput': {
                'hookEventName': 'UserPromptSubmit',
                'additionalContext': context,
            }
        }
        print(json.dumps(output, ensure_ascii=False))


if __name__ == '__main__':
    main()
