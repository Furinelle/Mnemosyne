"""Two-way handoff channel for non-Claude agents."""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass
from datetime import date

from mnemosyne.cli import make_memory_id, summarize
from mnemosyne.hooks._common import (
    collect_stores,
    extract_keywords,
    format_for_injection,
    run_search,
)
from mnemosyne.schema import Memory
from mnemosyne.store import project_store, read_core, working_path, write_memory

FINDINGS_HEADER_RE = re.compile(r'^\s*\*\*(?:新发现|Findings)[:：]\*\*\s*$', re.MULTILINE)
FIELD_RE = re.compile(r'^\s*-\s*(\w+)\s*:\s*(.*)$')
CONTENT_OPEN_RE = re.compile(r'^\s*-\s*content\s*:\s*\|\s*$')
ALLOWED_TYPES = ('arch_decision', 'pitfall', 'codebase', 'preference', 'handoff')


@dataclass
class Finding:
    type: str
    importance: int
    title: str
    tags: list[str]
    content: str


def prep(task: str, max_memories: int = 5) -> str:
    parts: list[str] = ['## Project memory (via Mnemosyne)', '']
    for store in collect_stores():
        core = read_core(store).strip()
        if not core:
            continue
        label = 'Global Core' if store.scope == 'global' else 'Project Core'
        parts.append(f'### {label}')
        parts.append(core)
        parts.append('')
    keywords = extract_keywords(task, limit=8)
    if keywords:
        results = run_search(' '.join(keywords), limit=max_memories, update_access=False)
        if results:
            parts.append('### Relevant prior memories')
            parts.append(format_for_injection(results))
            parts.append('')
    parts.append('### Mnemosyne CLI available')
    parts.append('If you need more context mid-task:')
    parts.append('    python -m mnemosyne search "<keywords>" --format json --limit 3')
    parts.append('')
    parts.append('### Reporting new findings')
    parts.append('When you finish, if you discovered something worth persisting,')
    parts.append('append a block in this exact format at the END of your reply:')
    parts.append('')
    parts.append('**新发现:**')
    parts.append('- type: pitfall|arch_decision|codebase|handoff')
    parts.append('- importance: 50-90')
    parts.append('- title: <=80 chars')
    parts.append('- tags: tag1, tag2')
    parts.append('- content: |')
    parts.append('    <multiline content here, 4-space indent>')
    parts.append('')
    parts.append('Multiple findings: repeat the block. Skip if there is nothing to record.')
    return '\n'.join(parts)


def parse_findings(text: str) -> list[Finding]:
    text = text.lstrip('﻿')
    header_match = FINDINGS_HEADER_RE.search(text)
    if not header_match:
        return []
    body = text[header_match.end():]
    lines = body.splitlines()
    findings: list[Finding] = []
    current: dict | None = None
    content_lines: list[str] | None = None
    content_indent: int | None = None

    def flush() -> None:
        if current is None:
            return
        type_value = current.get('type', '').strip()
        if type_value not in ALLOWED_TYPES:
            print(f'mnemosyne: dropping finding, unknown type {type_value!r}', file=sys.stderr)
            return
        try:
            importance = int(str(current.get('importance', '50')).strip())
        except ValueError:
            print(f'mnemosyne: dropping finding, bad importance {current.get("importance")!r}', file=sys.stderr)
            return
        importance = max(0, min(100, importance))
        title = current.get('title', '').strip().strip('"').strip("'")
        if not title:
            print('mnemosyne: dropping finding, empty title', file=sys.stderr)
            return
        tags_raw = current.get('tags', '').strip()
        tags = [t.strip() for t in tags_raw.split(',') if t.strip()] if tags_raw else []
        content = (current.get('content') or '').strip()
        if not content:
            print(f'mnemosyne: dropping finding {title!r}, empty content', file=sys.stderr)
            return
        findings.append(Finding(type_value, importance, title[:80], tags, content))

    index = 0
    while index < len(lines):
        line = lines[index]
        if content_lines is not None:
            stripped = line.strip()
            if not stripped:
                content_lines.append('')
                index += 1
                continue
            if content_indent is None:
                leading = len(line) - len(line.lstrip(' '))
                if leading == 0:
                    if current is not None:
                        current['content'] = '\n'.join(content_lines).rstrip()
                    content_lines = None
                    content_indent = None
                    continue
                content_indent = leading
            indent_now = len(line) - len(line.lstrip(' '))
            if indent_now < content_indent:
                if current is not None:
                    current['content'] = '\n'.join(content_lines).rstrip()
                content_lines = None
                content_indent = None
                continue
            content_lines.append(line[content_indent:])
            index += 1
            continue
        if CONTENT_OPEN_RE.match(line):
            if current is None:
                index += 1
                continue
            content_lines = []
            content_indent = None
            index += 1
            continue
        match = FIELD_RE.match(line)
        if match:
            key = match.group(1).lower()
            value = match.group(2)
            if key == 'type':
                if current is not None:
                    flush()
                current = {'type': value}
            elif current is not None and key in ('importance', 'title', 'tags'):
                current[key] = value
            index += 1
            continue
        if line.strip().startswith('**'):
            if current is not None:
                if content_lines is not None:
                    current['content'] = '\n'.join(content_lines).rstrip()
                    content_lines = None
                    content_indent = None
                flush()
                current = None
            break
        index += 1

    if current is not None:
        if content_lines is not None:
            current['content'] = '\n'.join(content_lines).rstrip()
        flush()

    return findings


def write_finding(finding: Finding, source: str) -> str:
    store = project_store()
    today = date.today().isoformat()
    memory_id = make_memory_id(finding.type, today)
    summary = summarize(finding.title, finding.content)
    memory = Memory(
        id=memory_id,
        type=finding.type,
        source=source,
        strength=min(100, finding.importance),
        created=today,
        last_accessed=today,
        access_count=0,
        tags=finding.tags,
        links=[],
        canonical_summary=summary,
        injection_summary=summary,
        status='active',
        body=f'## {finding.title}\n\n{finding.content}',
        expires='',
    )
    path = working_path(store, memory)
    write_memory(path, memory)
    return memory_id


def ingest(text: str, source: str = 'codex', commit: bool = False) -> list[dict]:
    findings = parse_findings(text)
    actions: list[dict] = []
    for finding in findings:
        record = {
            'type': finding.type,
            'importance': finding.importance,
            'title': finding.title,
            'tags': finding.tags,
            'content_preview': finding.content[:120] + ('...' if len(finding.content) > 120 else ''),
        }
        if commit:
            record['id'] = write_finding(finding, source)
        actions.append(record)
    return actions
