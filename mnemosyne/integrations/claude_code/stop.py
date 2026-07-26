"""Stop hook: dry-run maintain, surface core candidates, optional auto-distill."""

from __future__ import annotations

import json

from mnemosyne.events import handle_event
from mnemosyne.hooks._common import collect_stores, hook_safe, read_event
from mnemosyne.lifecycle import MaintainSummary, maintain_memory
from mnemosyne.store import load_config, load_memories


def main() -> None:
    with hook_safe():
        event = read_event()
        messages: list[str] = []

        # --- core candidate detection (dry-run maintain) ---
        summary = MaintainSummary()
        for store in collect_stores():
            config = load_config(store)
            for path, memory in load_memories(store, include_archive=False):
                result, candidate = maintain_memory(
                    store, path, memory, config["thresholds"], dry_run=True
                )
                if result == "core_candidate" and candidate is not None:
                    summary.core_candidates.append(candidate)
        if summary.core_candidates:
            lines = ["Mnemosyne: core memory promotion candidates detected:"]
            for memory in summary.core_candidates:
                lines.append(f"- {memory.id}: {memory.injection_summary}")
            lines.append("Edit core.md manually to promote them.")
            messages.append("\n".join(lines))

        # --- auto-distill via the session_end event ---
        distilled = _maybe_distill(event)
        if distilled:
            messages.append(distilled)

        if messages:
            print(json.dumps({"systemMessage": "\n\n".join(messages)}, ensure_ascii=False))


def _maybe_distill(event: dict) -> str:
    if event.get("stop_hook_active"):
        return ""  # avoid re-entrancy: we already ran on the previous Stop
    transcript_path = event.get("transcript_path")
    if not transcript_path:
        return ""
    result = handle_event(
        "session_end",
        {
            "transcript": {"path": str(transcript_path), "format": "claude-jsonl"},
            "source": "claude-code",
        },
    )
    return result.context


if __name__ == "__main__":
    main()
