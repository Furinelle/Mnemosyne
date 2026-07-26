"""Per-session injection dedup state, keyed by a caller-supplied session id.

Any adapter may pass its host's session identifier (or any stable string) to
avoid re-injecting the same memories every turn. Without a session id the
dedup degrades gracefully: nothing is recorded and every call injects fresh.
"""

from __future__ import annotations

import json
import os
from datetime import datetime, timedelta
from pathlib import Path

from mnemosyne.store import find_project_store, global_store

SESSION_STATE_FILENAME = '.session_injected.json'
SESSION_STATE_TTL_HOURS = 48


def _session_state_path() -> Path:
    project = find_project_store()
    root = project.root if project is not None else global_store().root
    return root / SESSION_STATE_FILENAME


def load_injected_ids(session_id: str) -> set[str]:
    if not session_id:
        return set()
    try:
        data = json.loads(_session_state_path().read_text(encoding='utf-8'))
    except (OSError, ValueError):
        return set()
    entry = data.get('sessions', {}).get(session_id, {})
    return set(entry.get('ids', []))


def record_injected_ids(session_id: str, memory_ids: list[str]) -> None:
    """Remember which memories this session has already seen.

    Injection hooks fire on every prompt and edit; without this, the same
    memory is re-injected each turn and quietly eats the context budget.
    Sessions older than the TTL are pruned so the state file stays small.
    """
    if not session_id or not memory_ids:
        return
    path = _session_state_path()
    try:
        data = json.loads(path.read_text(encoding='utf-8'))
    except (OSError, ValueError):
        data = {}
    if not isinstance(data, dict):
        data = {}
    sessions = data.setdefault('sessions', {})
    now = datetime.now()
    cutoff = now - timedelta(hours=SESSION_STATE_TTL_HOURS)
    for key in list(sessions):
        try:
            stamp = datetime.fromisoformat(str(sessions[key].get('ts', '')))
        except (AttributeError, ValueError):
            stamp = None
        if stamp is None or stamp < cutoff:
            del sessions[key]
    entry = sessions.setdefault(session_id, {'ts': now.isoformat(), 'ids': []})
    entry['ts'] = now.isoformat()
    entry['ids'] = sorted(set(entry['ids']) | set(memory_ids))
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix('.tmp')
        tmp.write_text(json.dumps(data, ensure_ascii=False), encoding='utf-8')
        os.replace(tmp, path)
    except OSError:
        pass
