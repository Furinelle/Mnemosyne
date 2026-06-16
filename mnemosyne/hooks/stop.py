"""Stop hook: dry-run maintain, surface core candidates, optional auto-distill."""

from __future__ import annotations

import json

from mnemosyne.hooks._common import collect_stores, hook_safe, read_event
from mnemosyne.lifecycle import MaintainSummary, maintain_memory
from mnemosyne.store import find_project_store, load_config, load_memories


def main() -> None:
    with hook_safe():
        event = read_event()
        messages: list[str] = []

        # --- existing behaviour: core candidate detection (dry-run maintain) ---
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

        # --- new behaviour: auto-distill from the transcript ---
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
    project = find_project_store()
    config = load_config(project)
    if not config.get("distill", {}).get("enabled"):
        return ""
    from mnemosyne.distill import distill_text, parse_claude_transcript, turns_to_text

    turns = parse_claude_transcript(transcript_path)
    if not turns:
        return ""
    actions = distill_text(turns_to_text(turns), source="claude-code", commit=True)
    saved = [a for a in actions if a.get("id")]
    if not saved:
        return ""
    lines = ["Mnemosyne: auto-saved memories from this session:"]
    for action in saved:
        lines.append(f"- [{action['verdict']}] {action['type']}: {action['title']} ({action['id']})")
    return "\n".join(lines)


if __name__ == "__main__":
    main()
