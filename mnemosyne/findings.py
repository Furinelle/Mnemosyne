"""Agent-neutral finding model shared by handoff, distill, and write paths."""

from __future__ import annotations

from dataclasses import dataclass

FALLBACK_TYPES = ('arch_decision', 'pitfall', 'codebase', 'preference', 'handoff', 'session_summary')


@dataclass
class Finding:
    type: str
    importance: int
    title: str
    tags: list[str]
    content: str
    evidence: str = ""


def allowed_types(config: dict | None = None) -> tuple[str, ...]:
    """Memory types accepted on ingest/distill paths.

    Derived from the store config's memory.types so user-defined types work
    on every write path, not only manual `write`; the hardcoded tuple is only
    a fallback for configless contexts.
    """
    if config:
        types = config.get("memory", {}).get("types") or []
        if types:
            return tuple(str(t) for t in types)
    return FALLBACK_TYPES
