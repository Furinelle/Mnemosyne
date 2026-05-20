"""SessionStart hook: inject core memory at session start."""

from __future__ import annotations

import json
import subprocess
import sys
from datetime import datetime, timedelta

from mnemosyne.hooks._common import collect_stores, hook_safe, read_event
from mnemosyne.store import find_project_store, read_core


MAINTAIN_INTERVAL = timedelta(hours=24)


def maybe_run_maintain() -> None:
    project = find_project_store()
    if project is None:
        return
    marker = project.root / '.last_maintain'
    if marker.exists():
        try:
            last = datetime.fromisoformat(marker.read_text(encoding='utf-8').strip())
        except (ValueError, OSError):
            last = None
        if last is not None:
            now = datetime.now(tz=last.tzinfo) if last.tzinfo else datetime.now()
            if now - last < MAINTAIN_INTERVAL:
                return
    kwargs: dict = {'stdout': subprocess.DEVNULL, 'stderr': subprocess.DEVNULL}
    if sys.platform == 'win32':
        kwargs['creationflags'] = subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_PROCESS_GROUP
    else:
        kwargs['start_new_session'] = True
    try:
        subprocess.Popen(
            [sys.executable, '-m', 'mnemosyne', 'maintain', '--scope', 'all'],
            **kwargs,
        )
    except OSError:
        return
    try:
        marker.write_text(datetime.now().isoformat(), encoding='utf-8')
    except OSError:
        pass


def main() -> None:
    with hook_safe():
        event = read_event()
        source = event.get('source', 'startup')
        if source in ('resume', 'compact'):
            return
        parts: list[str] = []
        for store in collect_stores():
            content = read_core(store).strip()
            if not content:
                continue
            label = 'Global Core' if store.scope == 'global' else 'Project Core'
            parts.append(f'### {label}')
            parts.append(content)
        if not parts:
            return
        maybe_run_maintain()
        output = {
            'hookSpecificOutput': {
                'hookEventName': 'SessionStart',
                'additionalContext': '## Mnemosyne Memory\n\n' + '\n\n'.join(parts),
            }
        }
        print(json.dumps(output, ensure_ascii=False))


if __name__ == '__main__':
    main()
