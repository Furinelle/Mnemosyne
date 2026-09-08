"""Transcript parser registry: turn any agent's session log into Turns.

Four first-class formats:

- ``claude-jsonl``: Claude Code session JSONL ({"message": {"role", "content"}})
- ``codex-jsonl``: Codex rollout JSONL (``response_item`` message records)
- ``grok-jsonl``: Grok Build chat history JSONL (top-level ``type`` + ``content``)
- ``role-jsonl``:   neutral one-object-per-line format ({"role", "text"}) —
                    preprocess any agent's transcript into this to get the
                    full distill pipeline
- ``text``:         plain text, optionally with ``[user] ...`` / ``[assistant] ...``
                    role markers

``fmt="auto"`` detects the format from the first non-empty line.
"""

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
            if isinstance(block, dict) and block.get("type") in {"text", "input_text", "output_text"}:
                parts.append(str(block.get("text", "")))
        return "\n".join(p for p in parts if p).strip()
    return ""


def parse_claude_jsonl(raw: str) -> list[Turn]:
    turns: list[Turn] = []
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


def parse_codex_jsonl(raw: str) -> list[Turn]:
    """Parse Codex rollout JSONL message records into turns."""
    turns: list[Turn] = []
    for line in raw.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(record, dict) or record.get("type") != "response_item":
            continue
        payload = record.get("payload")
        if not isinstance(payload, dict) or payload.get("type") != "message":
            continue
        role = payload.get("role")
        if role not in ("user", "assistant"):
            continue
        text = _block_text(payload.get("content"))
        if text:
            turns.append(Turn(role=role, text=text))
    return turns


def parse_grok_jsonl(raw: str) -> list[Turn]:
    """Parse Grok Build chat_history.jsonl user and assistant records."""
    turns: list[Turn] = []
    for line in raw.splitlines():
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(record, dict) or record.get("type") not in ("user", "assistant"):
            continue
        text = _block_text(record.get("content"))
        if text:
            turns.append(Turn(role=str(record["type"]), text=text))
    return turns


def parse_role_jsonl(raw: str) -> list[Turn]:
    turns: list[Turn] = []
    for line in raw.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(record, dict):
            continue
        role = record.get("role")
        if role not in ("user", "assistant"):
            continue
        text = str(record.get("text", "")).strip()
        if text:
            turns.append(Turn(role=role, text=text))
    return turns


def parse_role_lines(text: str) -> list[Turn]:
    """Parse '[role] text' lines (as produced by ``turns_to_text``) into turns.

    Continuation lines of a multi-line turn are accumulated into the current
    turn rather than each physical line becoming its own turn. Without this, a
    multi-line user turn loses its `[user]` marker on every line after the
    first, so those lines get misclassified as assistant text and role-gated
    rules (e.g. the preference heuristic) silently drop them.
    """
    turns: list[Turn] = []
    role: str | None = None
    buffer: list[str] = []

    def flush() -> None:
        if role is not None:
            body = "\n".join(buffer).strip()
            if body:
                turns.append(Turn(role=role, text=body))

    for line in text.splitlines():
        if line.startswith("[user] ") or line.startswith("[assistant] "):
            flush()
            role = "user" if line.startswith("[user] ") else "assistant"
            buffer = [line.split("] ", 1)[1]]
        elif role is not None:
            buffer.append(line)
    flush()
    return turns


def parse_text(raw: str) -> list[Turn]:
    turns = parse_role_lines(raw)
    if turns:
        return turns
    stripped = raw.strip()
    if not stripped:
        return []
    return [Turn(role="assistant", text=stripped)]


PARSERS = {
    "claude-jsonl": parse_claude_jsonl,
    "codex-jsonl": parse_codex_jsonl,
    "grok-jsonl": parse_grok_jsonl,
    "role-jsonl": parse_role_jsonl,
    "text": parse_text,
}


def detect_format(sample: str) -> str:
    for line in sample.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            return "text"
        if not isinstance(record, dict):
            return "text"
        if isinstance(record.get("message"), dict):
            return "claude-jsonl"
        payload = record.get("payload")
        if (
            record.get("type") == "response_item"
            and isinstance(payload, dict)
            and payload.get("type") == "message"
        ):
            return "codex-jsonl"
        if record.get("type") in ("user", "assistant") and "content" in record:
            return "grok-jsonl"
        if "role" in record and "text" in record:
            return "role-jsonl"
        # JSON housekeeping record (summary / queue-operation / snapshot …):
        # real Claude Code transcripts rarely START with a message line, so
        # keep scanning instead of concluding "text" from the first line.
        continue
    return "text"


def parse_transcript(path_or_text: Path | str, fmt: str = "auto") -> list[Turn]:
    """Parse a transcript file (Path) or raw string into Turns."""
    if isinstance(path_or_text, Path):
        try:
            raw = path_or_text.read_text(encoding="utf-8")
        except OSError:
            return []
    else:
        raw = path_or_text
    if fmt == "auto":
        fmt = detect_format(raw)
    parser = PARSERS.get(fmt)
    if parser is None:
        raise ValueError(f"Unknown transcript format: {fmt!r}. Expected one of: {', '.join(PARSERS)} or auto")
    return parser(raw)
