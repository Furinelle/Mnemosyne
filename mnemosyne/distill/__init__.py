"""Extraction core: turn a conversation into Findings, dedup, and persist."""

from __future__ import annotations

import json
import os
from dataclasses import dataclass
from datetime import datetime, timedelta
from pathlib import Path

from mnemosyne.codex import Finding
from mnemosyne.search import tokenize
from mnemosyne.store import (
    Store,
    find_memory,
    find_project_store,
    global_store,
    load_config,
    load_memories,
    lock_store,
    lock_stores,
    stores_for_scope,
)


@dataclass(frozen=True)
class Turn:
    role: str
    text: str


DISTILL_STATE_FILENAME = ".distill_state.json"
DISTILL_STATE_TTL_HOURS = 168  # transcripts can span days; prune after a week


def _distill_state_path() -> Path:
    from mnemosyne.store import find_project_store

    project = find_project_store()
    root = project.root if project is not None else global_store().root
    return root / DISTILL_STATE_FILENAME


def load_processed_turns(transcript_key: str) -> int:
    if not transcript_key:
        return 0
    try:
        data = json.loads(_distill_state_path().read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return 0
    entry = data.get("transcripts", {}).get(transcript_key, {})
    try:
        return max(0, int(entry.get("turns", 0)))
    except (TypeError, ValueError):
        return 0


def record_processed_turns(transcript_key: str, count: int) -> None:
    """Remember how many turns of a transcript have been distilled.

    The Stop hook fires after every assistant reply; without this the whole
    transcript is re-parsed and re-extracted each turn. Failures degrade
    silently to full re-processing (dedup still guards correctness).
    """
    if not transcript_key:
        return
    path = _distill_state_path()
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        data = {}
    if not isinstance(data, dict):
        data = {}
    transcripts = data.setdefault("transcripts", {})
    now = datetime.now()
    cutoff = now - timedelta(hours=DISTILL_STATE_TTL_HOURS)
    for key in list(transcripts):
        try:
            stamp = datetime.fromisoformat(str(transcripts[key].get("ts", "")))
        except (AttributeError, ValueError):
            stamp = None
        if stamp is None or stamp < cutoff:
            del transcripts[key]
    transcripts[transcript_key] = {"ts": now.isoformat(), "turns": max(0, int(count))}
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix(".tmp")
        tmp.write_text(json.dumps(data, ensure_ascii=False), encoding="utf-8")
        os.replace(tmp, path)
    except OSError:
        pass


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
    store: Store | None = None,
    dedup_threshold: float = 0.85,
    subject_threshold: float = 0.5,
) -> tuple[str, str | None]:
    """Return ('new'|'duplicate'|'supersede', target_memory_id_or_None).

    duplicate  -> content nearly identical to an existing memory; skip writing.
    supersede  -> same subject, materially different content; write new + link.
    """
    destination = store or find_project_store() or global_store()
    text = f"{finding.title} {finding.content}".strip()
    text_tokens = tokenize(text)
    title_tokens = tokenize(finding.title)
    supersede_target: str | None = None
    supersede_score = 0.0
    for _path, memory in load_memories(destination, include_archive=False):
        if memory.type != finding.type or memory.status == "superseded":
            continue
        # Compare against the full stored body, not the truncated summary.
        # summarize() caps at ~220 chars, so long findings lost their tail
        # tokens and never reached dedup_threshold -> the same memory was
        # rewritten on every session (observed: 9 identical copies).
        body_tokens = tokenize(memory.body or "")
        if jaccard(text_tokens, body_tokens) >= dedup_threshold:
            return ("duplicate", memory.id)
        subject_score = jaccard(title_tokens, body_tokens)
        if subject_score >= subject_threshold and subject_score > supersede_score:
            supersede_target = memory.id
            supersede_score = subject_score
    if supersede_target is not None:
        return ("supersede", supersede_target)
    return ("new", None)


def process_finding(
    finding: Finding,
    *,
    source: str,
    commit: bool,
    store: Store | None = None,
    dedup_threshold: float = 0.85,
    subject_threshold: float = 0.5,
) -> dict:
    """Classify and optionally persist a finding in one destination store."""
    destination = store or find_project_store() or global_store()
    if not commit:
        verdict, target = classify_against_store(
            finding,
            store=destination,
            dedup_threshold=dedup_threshold,
            subject_threshold=subject_threshold,
        )
        return {"verdict": verdict, "target": target}

    with lock_store(destination):
        verdict, target = classify_against_store(
            finding,
            store=destination,
            dedup_threshold=dedup_threshold,
            subject_threshold=subject_threshold,
        )
        record: dict = {"verdict": verdict, "target": target}
        if verdict == "duplicate":
            if commit and target:
                record["id"] = target
            return record
        from mnemosyne.codex import write_finding

        record["id"] = write_finding(finding, source, store=destination, _locked=True)
        if verdict == "supersede" and target:
            _apply_supersedes(
                record["id"], target, stores=[destination], _locked=True
            )
    return record


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
    store = find_project_store() or global_store()
    actions: list[dict] = []
    for finding in findings:
        outcome = process_finding(
            finding,
            source=source,
            commit=commit,
            store=store,
            dedup_threshold=dedup,
            subject_threshold=subject,
        )
        record = {
            "verdict": outcome["verdict"],
            "type": finding.type,
            "title": finding.title,
            "target": outcome.get("target"),
        }
        if "id" in outcome:
            record["id"] = outcome["id"]
        actions.append(record)
    return actions


def _apply_supersedes(
    new_id: str,
    old_id: str,
    *,
    stores: list[Store] | None = None,
    _locked: bool = False,
) -> None:
    from mnemosyne.cli import DEMOTE_ON_SUPERSEDE, add_link, update_search_index
    from mnemosyne.relations import reverse
    from mnemosyne.store import write_memory

    selected = stores_for_scope("all") if stores is None else list(stores)
    selected = [store for store in selected if store.root.exists()]

    def apply_locked() -> None:
        new = find_memory(new_id, selected, include_archive=False)
        old = find_memory(old_id, selected, include_archive=True)
        if new is None or old is None:
            return
        new_store, new_path, new_memory = new
        old_store, old_path, old_memory = old
        add_link(new_memory, old_memory.id, "supersedes")
        add_link(old_memory, new_memory.id, reverse("supersedes") or "superseded_by")
        old_memory.strength = max(0, old_memory.strength - DEMOTE_ON_SUPERSEDE)
        old_memory.status = "superseded"
        old_memory.extra["invalidated_by"] = new_id
        write_memory(new_path, new_memory)
        write_memory(old_path, old_memory)
        update_search_index(new_store, new_path, new_memory)
        update_search_index(old_store, old_path, old_memory)

    if _locked:
        apply_locked()
    else:
        with lock_stores(selected):
            apply_locked()
