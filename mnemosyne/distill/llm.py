"""Optional LLM-backed extractor. Requires an API key. The network call is
isolated so the heuristic path never imports anything heavy.
"""

from __future__ import annotations

import json
import os

from mnemosyne.findings import FALLBACK_TYPES as ALLOWED_TYPES, Finding

_BASE_PROMPT = (
    "You extract durable memories from a developer/agent conversation. "
    "Return ONLY a JSON array. Each element: "
    '{"type": one of '
    + "|".join(ALLOWED_TYPES)
    + ', "importance": 50-90, "title": <=80 chars, "tags": [..], "content": "...",'
    ' "evidence": short verbatim quote from the conversation supporting this memory}. '
    "Only include genuinely reusable facts (pitfalls, decisions, preferences, codebase, handoff). "
    "Empty array if nothing is worth saving."
)

_SUMMARY_INSTRUCTION = (
    ' Additionally, include exactly one element with type "session_summary": '
    "2-4 sentences on what was worked on and the outcome, importance 55."
)


def _build_prompt(conversation: str, include_session_summary: bool) -> str:
    prompt = _BASE_PROMPT
    if include_session_summary:
        prompt += _SUMMARY_INSTRUCTION
    return prompt + "\n\nCONVERSATION:\n" + conversation


def _parse_llm_json(payload: str) -> list[Finding]:
    payload = payload.strip()
    start, end = payload.find("["), payload.rfind("]")
    if start == -1 or end == -1:
        return []
    try:
        items = json.loads(payload[start : end + 1])
    except json.JSONDecodeError:
        return []
    findings: list[Finding] = []
    for item in items:
        if not isinstance(item, dict):
            continue
        type_value = str(item.get("type", "")).strip()
        if type_value not in ALLOWED_TYPES:
            continue
        title = str(item.get("title", "")).strip()[:80]
        content = str(item.get("content", "")).strip()
        if not title or not content:
            continue
        try:
            importance = max(0, min(100, int(item.get("importance", 60))))
        except (TypeError, ValueError):
            importance = 60
        tags = [str(t).strip() for t in item.get("tags", []) if str(t).strip()]
        evidence = str(item.get("evidence", "")).strip()[:200]
        findings.append(Finding(type_value, importance, title, tags, content, evidence))
    return findings


class LLMExtractor:
    def __init__(self, config: dict, max_findings: int = 5) -> None:
        distill_cfg = config.get("distill", {})
        self.llm_cfg = distill_cfg.get("llm", {})
        self.include_session_summary = bool(distill_cfg.get("session_summary", False))
        self.max_findings = max_findings

    def extract(self, turns) -> list[Finding]:
        from mnemosyne.distill import turns_to_text

        api_key = os.environ.get(self.llm_cfg.get("api_key_env", "OPENAI_API_KEY"), "")
        if not api_key:
            import sys

            print("mnemosyne: distill.llm enabled but API key missing; skipping", file=sys.stderr)
            return []
        payload = self._call_api(
            _build_prompt(turns_to_text(turns), self.include_session_summary), api_key
        )
        return _parse_llm_json(payload)[: self.max_findings]

    def _call_api(self, prompt: str, api_key: str) -> str:
        import urllib.request

        base = self.llm_cfg.get("api_base", "https://api.openai.com/v1").rstrip("/")
        body = json.dumps(
            {
                "model": self.llm_cfg.get("model") or "gpt-4o-mini",
                "messages": [{"role": "user", "content": prompt}],
                "temperature": 0,
            }
        ).encode("utf-8")
        request = urllib.request.Request(
            f"{base}/chat/completions",
            data=body,
            headers={"Authorization": f"Bearer {api_key}", "Content-Type": "application/json"},
        )
        with urllib.request.urlopen(request, timeout=30) as response:
            data = json.loads(response.read().decode("utf-8"))
        return data["choices"][0]["message"]["content"]
