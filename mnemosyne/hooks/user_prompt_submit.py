"""UserPromptSubmit hook: inject top-3 relevant memories based on prompt keywords."""

from __future__ import annotations

import json
import sys

from mnemosyne.hooks._common import (
    collect_stores,
    extract_keywords,
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
        prompt = (event.get('prompt') or '').strip()
        if len(prompt) < 10:
            return
        keywords = extract_keywords(prompt, limit=8)
        if not keywords:
            return
        session_id = str(event.get('session_id') or '')
        already = load_injected_ids(session_id)
        results = run_search(' '.join(keywords), limit=3, update_access=True)
        fresh = [item for item in results if item['id'] not in already]
        if not fresh:
            return
        stores = collect_stores()
        config = load_config(stores[-1] if stores else None)
        max_tokens = int(config.get('injection', {}).get('max_tokens', 2000))
        context = format_for_injection(fresh, max_tokens=max_tokens)
        if not context:
            return
        record_injected_ids(session_id, [item['id'] for item in fresh])
        output = {
            'hookSpecificOutput': {
                'hookEventName': 'UserPromptSubmit',
                'additionalContext': context,
            }
        }
        print(json.dumps(output, ensure_ascii=False))


if __name__ == '__main__':
    main()
