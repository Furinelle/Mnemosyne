"""UserPromptSubmit hook: inject top-3 relevant memories based on prompt keywords."""

from __future__ import annotations

import json
import sys

from mnemosyne.hooks._common import (
    collect_stores,
    extract_keywords,
    format_for_injection,
    hook_safe,
    read_event,
    run_search,
)
from mnemosyne.store import load_config


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
        stores = collect_stores()
        config = load_config(stores[-1] if stores else None)
        max_tokens = int(config.get('injection', {}).get('max_tokens', 2000))
        context = format_for_injection(results, max_tokens=max_tokens)
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
