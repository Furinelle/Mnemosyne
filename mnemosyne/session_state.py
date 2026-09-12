"""Per-session injection dedup state, keyed by a caller-supplied session id.

Any adapter may pass its host's session identifier (or any stable string) to
avoid re-injecting the same memories every turn. Without a session id the
dedup degrades gracefully: nothing is recorded and every call injects fresh.
"""

from __future__ import annotations

import json
import os
from datetime import datetime, timezone
from pathlib import Path

import portalocker

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
    entry = _load_sessions(_session_state_path()).get(session_id, {})
    return set(entry.get('ids', []))


def _load_sessions(path: Path) -> dict:
    try:
        data = json.loads(path.read_text(encoding='utf-8'))
    except (OSError, ValueError):
        return {}
    sessions = data.get('sessions', {}) if isinstance(data, dict) else {}
    if not isinstance(sessions, dict):
        return {}
    cutoff = datetime.now(timezone.utc).timestamp() - SESSION_STATE_TTL_HOURS * 3600
    valid = {}
    for key, entry in sessions.items():
        if not isinstance(entry, dict) or not isinstance(entry.get('ids'), list):
            continue
        try:
            stamp = datetime.fromisoformat(str(entry.get('ts', ''))).timestamp()
        except (ValueError, OverflowError, OSError):
            continue
        if stamp >= cutoff:
            valid[key] = {'ts': entry['ts'], 'ids': [item for item in entry['ids'] if isinstance(item, str)]}
    return valid


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
        path.parent.mkdir(parents=True, exist_ok=True)
        # Lock the entire read/merge/replace: atomic replace alone loses
        # another agent's updates when both read the old state concurrently.
        with portalocker.Lock(str(path.with_suffix('.lock')), mode='a', timeout=2):
            sessions = _load_sessions(path)
            entry = sessions.setdefault(session_id, {'ids': []})
            entry['ts'] = datetime.now(timezone.utc).isoformat()
            entry['ids'] = sorted(set(entry['ids']) | set(memory_ids))
            tmp = path.with_suffix('.tmp')
            tmp.write_text(json.dumps({'sessions': sessions}, ensure_ascii=False), encoding='utf-8')
            os.replace(tmp, path)
    except (OSError, portalocker.exceptions.LockException):
        pass
