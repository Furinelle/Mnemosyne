"""Extraction core: turn a conversation into Findings, dedup, and persist."""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

from mnemosyne.codex import Finding
from mnemosyne.hooks._common import run_search
from mnemosyne.search import tokenize


@dataclass(frozen=True)
class Turn:
    role: str
    text: str


def _block_text(content: object) -> str:
    if isinstance(content, str):
        return content.strip()
    if isinstance(content, list):
        parts: list[str] = []
        for block in content:
            if isinstance(block, dict) and block.get("type") == "text":
                parts.append(str(block.get("text", "")))
        return "\n".join(p for p in parts if p).strip()
    return ""


def parse_claude_transcript(path: Path) -> list[Turn]:
    """Parse a Claude Code JSONL transcript into user/assistant text turns."""
    turns: list[Turn] = []
    try:
        raw = Path(path).read_text(encoding="utf-8")
    except OSError:
        return turns
    for line in raw.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        message = record.get("message")
        if not isinstance(message, dict):
            continue
        role = message.get("role")
        if role not in ("user", "assistant"):
            continue
        text = _block_text(message.get("content"))
        if text:
            turns.append(Turn(role=role, text=text))
    return turns


def turns_to_text(turns: list[Turn]) -> str:
    return "\n\n".join(f"[{turn.role}] {turn.text}" for turn in turns)


def jaccard(a: list[str], b: list[str]) -> float:
    set_a, set_b = set(a), set(b)
    if not set_a or not set_b:
        return 0.0
    union = set_a | set_b
    return len(set_a & set_b) / len(union)


def classify_against_store(
    finding: Finding,
    *,
    dedup_threshold: float = 0.85,
    subject_threshold: float = 0.5,
) -> tuple[str, str | None]:
    """Return ('new'|'duplicate'|'supersede', target_memory_id_or_None).

    duplicate  -> content nearly identical to an existing memory; skip writing.
    supersede  -> same subject, materially different content; write new + link.
    """
    text = f"{finding.title} {finding.content}".strip()
    results = run_search(text, limit=3, update_access=False)
    if not results:
        return ("new", None)
    top = results[0]
    summary_tokens = tokenize(top.get("summary", ""))
    content_sim = jaccard(tokenize(text), summary_tokens)
    subject_sim = jaccard(tokenize(finding.title), summary_tokens)
    if content_sim >= dedup_threshold:
        return ("duplicate", top["id"])
    if subject_sim >= subject_threshold:
        return ("supersede", top["id"])
    return ("new", None)
