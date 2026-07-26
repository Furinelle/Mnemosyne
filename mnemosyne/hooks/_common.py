"""Claude Code hook protocol helpers + backward-compat re-exports.

The generic logic (keyword extraction, search, injection formatting, session
dedup state) moved to mnemosyne.injection and mnemosyne.session_state; only
the Claude Code stdin/stdout protocol bits live here.
"""

from __future__ import annotations

import json
import sys
import traceback
from contextlib import contextmanager

from mnemosyne.injection import (  # noqa: F401  (re-exported for backward compat)
    STOPWORDS,
    _approx_tokens,
    collect_stores,
    extract_keywords,
    format_for_injection,
    run_search,
)
from mnemosyne.session_state import (  # noqa: F401  (re-exported for backward compat)
    SESSION_STATE_FILENAME,
    SESSION_STATE_TTL_HOURS,
    _session_state_path,
    load_injected_ids,
    record_injected_ids,
)


@contextmanager
def hook_safe():
    """Ensure any hook script exits 0 even on exceptions, to never block Claude."""
    try:
        yield
    except Exception:
        traceback.print_exc(file=sys.stderr)
        sys.exit(0)


def read_event() -> dict:
    """Read a JSON event from stdin, tolerating a leading UTF-8 BOM.

    PowerShell 7+ prepends a BOM when piping strings to subprocess stdin,
    which json.load otherwise rejects. Returns {} when stdin is empty.
    """
    raw = sys.stdin.read()
    if not raw:
        return {}
    if raw.startswith('﻿'):
        raw = raw[1:]
    return json.loads(raw)
