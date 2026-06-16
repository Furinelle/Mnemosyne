"""Extraction core: turn a conversation into Findings, dedup, and persist."""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

from mnemosyne.codex import Finding
from mnemosyne.hooks._common import run_search
from mnemosyne.search import tokenize
from mnemosyne.store import find_memory, load_config, stores_for_scope


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


def _make_extractor(config: dict):
    distill_cfg = config.get("distill", {})
    engine = distill_cfg.get("engine", "heuristic")
    threshold = float(distill_cfg.get("confidence_threshold", 0.6))
    max_findings = int(distill_cfg.get("max_findings_per_session", 5))
    if engine == "llm":
        from mnemosyne.distill.llm import LLMExtractor

        return LLMExtractor(config, max_findings=max_findings)
    from mnemosyne.distill.heuristic import HeuristicExtractor

    return HeuristicExtractor(confidence_threshold=threshold, max_findings=max_findings)


def _parse_role_lines(text: str) -> list[Turn]:
    """Parse '[role] text' lines (as produced by `turns_to_text`) back into turns.

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


def _findings_from_text(text: str, config: dict) -> list[Finding]:
    distill_cfg = config.get("distill", {})
    engine = distill_cfg.get("engine", "heuristic")
    if engine == "host":
        from mnemosyne.codex import parse_findings

        return parse_findings(text)
    turns = _parse_role_lines(text)
    if not turns:
        turns = [Turn(role="assistant", text=text)]
    return _make_extractor(config).extract(turns)


def distill_text(text: str, *, source: str = "claude-code", commit: bool = False) -> list[dict]:
    """Extract findings from conversation text, dedup, and (optionally) persist."""
    config = load_config()
    findings = _findings_from_text(text, config)
    distill_cfg = config.get("distill", {})
    dedup = float(distill_cfg.get("dedup_threshold", 0.85))
    subject = float(distill_cfg.get("subject_threshold", 0.5))
    actions: list[dict] = []
    for finding in findings:
        verdict, target = classify_against_store(
            finding, dedup_threshold=dedup, subject_threshold=subject
        )
        record = {
            "verdict": verdict,
            "type": finding.type,
            "title": finding.title,
            "target": target,
        }
        if commit and verdict != "duplicate":
            from mnemosyne.codex import write_finding

            new_id = write_finding(finding, source)
            record["id"] = new_id
            if verdict == "supersede" and target:
                _apply_supersedes(new_id, target)
        actions.append(record)
    return actions


def _apply_supersedes(new_id: str, old_id: str) -> None:
    from mnemosyne.cli import DEMOTE_ON_SUPERSEDE, add_link, update_search_index
    from mnemosyne.relations import reverse
    from mnemosyne.store import write_memory

    stores = stores_for_scope("all")
    new = find_memory(new_id, stores, include_archive=False)
    old = find_memory(old_id, stores, include_archive=True)
    if new is None or old is None:
        return
    new_store, new_path, new_memory = new
    old_store, old_path, old_memory = old
    add_link(new_memory, old_memory.id, "supersedes")
    add_link(old_memory, new_memory.id, reverse("supersedes") or "superseded_by")
    old_memory.strength = max(0, old_memory.strength - DEMOTE_ON_SUPERSEDE)
    write_memory(new_path, new_memory)
    write_memory(old_path, old_memory)
    update_search_index(new_store, new_path, new_memory)
    update_search_index(old_store, old_path, old_memory)
