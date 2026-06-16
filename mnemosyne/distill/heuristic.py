"""Deterministic, stdlib-only extraction rules (conservative: high precision)."""

from __future__ import annotations

import re

from mnemosyne.codex import Finding
from mnemosyne.distill import Turn

# Preference: user corrects the agent's default choice.
_PREFERENCE_RE = re.compile(r"(不要用|别用|不用|改用|下次用|下次记得|don't use|use .* instead)", re.IGNORECASE)
# Pitfall: an error signal co-occurs with a resolution signal in one turn.
_ERROR_RE = re.compile(r"(错误|报错|Traceback|Exception|Error|failed|失败)", re.IGNORECASE)
_FIX_RE = re.compile(r"(根因|原因是|修复|改成|改为|fixed|the fix|因为)", re.IGNORECASE)

_PREFERENCE_CONFIDENCE = 0.6
_PITFALL_CONFIDENCE = 0.7


class HeuristicExtractor:
    def __init__(self, confidence_threshold: float = 0.6, max_findings: int = 5) -> None:
        self.confidence_threshold = confidence_threshold
        self.max_findings = max_findings

    def extract(self, turns: list[Turn]) -> list[Finding]:
        findings: list[Finding] = []
        for turn in turns:
            candidate = self._match(turn)
            if candidate is None:
                continue
            finding, confidence = candidate
            if confidence < self.confidence_threshold:
                continue
            findings.append(finding)
            if len(findings) >= self.max_findings:
                break
        return findings

    def _match(self, turn: Turn) -> tuple[Finding, float] | None:
        text = turn.text.strip()
        if not text:
            return None
        if turn.role == "user" and _PREFERENCE_RE.search(text):
            return (
                Finding(
                    type="preference",
                    importance=65,
                    title=text[:60],
                    tags=["auto", "preference"],
                    content=text,
                ),
                _PREFERENCE_CONFIDENCE,
            )
        if turn.role == "assistant" and _ERROR_RE.search(text) and _FIX_RE.search(text):
            return (
                Finding(
                    type="pitfall",
                    importance=75,
                    title=text[:60],
                    tags=["auto", "pitfall"],
                    content=text[:1000],
                ),
                _PITFALL_CONFIDENCE,
            )
        return None
