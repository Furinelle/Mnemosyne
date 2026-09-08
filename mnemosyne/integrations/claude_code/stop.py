"""Stop hook: optional auto-distill."""

from __future__ import annotations

import json
from itertools import islice
from pathlib import Path

from mnemosyne.events import handle_event
from mnemosyne.hooks._common import hook_safe, read_event
from mnemosyne.transcripts import detect_format


def main() -> None:
    with hook_safe():
        event = read_event()
        distilled = _maybe_distill(event)
        if distilled:
            print(json.dumps({"systemMessage": distilled}, ensure_ascii=False))


def _maybe_distill(event: dict) -> str:
    if event.get("stop_hook_active"):
        return ""  # avoid re-entrancy: we already ran on the previous Stop
    transcript_path = event.get("transcript_path")
    if not transcript_path:
        return ""
    try:
        # ponytail: known hosts identify themselves near the start; stream-detect if preambles grow.
        with Path(transcript_path).open(encoding="utf-8") as transcript:
            transcript_format = detect_format("".join(islice(transcript, 100)))
    except OSError:
        transcript_format = "auto"
    source = {
        "codex-jsonl": "codex",
        "grok-jsonl": "grok-build",
    }.get(transcript_format, "claude-code")
    result = handle_event(
        "session_end",
        {
            "transcript": {"path": str(transcript_path), "format": transcript_format},
            "source": source,
        },
    )
    return result.context


if __name__ == "__main__":
    main()
