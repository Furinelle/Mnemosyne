"""Extraction core: turn a conversation into Findings, dedup, and persist."""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path


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
