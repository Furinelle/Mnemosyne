"""SessionStart hook: inject core memory at session start."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from datetime import datetime, timedelta
from pathlib import Path

from mnemosyne.hooks._common import collect_stores, hook_safe, read_event
from mnemosyne.store import (
    Store,
    ensure_store,
    find_project_store,
    global_store,
    read_core,
    template_text,
)


MAINTAIN_INTERVAL = timedelta(hours=24)


def maybe_auto_init_project() -> None:
    """Auto-create .mnemosyne/ in the enclosing git project if missing.

    Skipped when MNEMOSYNE_AUTO_INIT is '0'/'false'/'no', when no .git
    ancestor is found, when a .mnemosyne-disable marker exists in the
    project root, or when .mnemosyne/ already exists.
    """
    if os.environ.get('MNEMOSYNE_AUTO_INIT', '1').lower() in ('0', 'false', 'no'):
        return
    current = Path.cwd().resolve()
    project_root: Path | None = None
    while True:
        if (current / '.git').exists():
            project_root = current
            break
        parent = current.parent
        if parent == current:
            break
        current = parent
    if project_root is None:
        return
    if (project_root / '.mnemosyne-disable').exists():
        return
    target = project_root / '.mnemosyne'
    if target.exists():
        return
    try:
        if target.resolve() == global_store().root.resolve():
            return
    except OSError:
        return
    try:
        ensure_store(Store('project', target), template_text('core_project.md'))
    except OSError:
        pass


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
        maybe_auto_init_project()
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
