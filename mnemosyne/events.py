"""Agent-neutral injection events.

Four lifecycle moments cover what memory injection needs from any host:

- session_start: assemble core memory for the opening context
- turn_start:    inject memories relevant to the user's prompt
- file_touch:    inject memories relevant to files about to be modified
- session_end:   distill the finished conversation into new memories

Adapters translate their host's native hook protocol into these events
(see integrations/); `mnemosyne inject` exposes them to any host with a
shell. Host-specific policy (auto-init, maintenance scheduling, re-entrancy
guards) stays in the adapters.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path

from mnemosyne.injection import (
    _approx_tokens,
    collect_stores,
    extract_keywords,
    format_for_injection,
    resolve_show_hint,
    run_search,
)
from mnemosyne.session_state import load_injected_ids, record_injected_ids
from mnemosyne.store import find_project_store, load_config, read_core

EVENTS = ("session_start", "turn_start", "file_touch", "session_end")

MIN_PROMPT_CHARS = 10


@dataclass
class InjectionResult:
    context: str                       # ready-to-inject text; "" means nothing to add
    memory_ids: list[str] = field(default_factory=list)
    approx_tokens: int = 0


def handle_event(
    event: str,
    payload: dict,
    *,
    session: str = "",
    channel: str = "cli",
    update_access: bool | None = None,
) -> InjectionResult:
    if event == "session_start":
        return _session_start()
    if event == "turn_start":
        return _turn_start(payload, session, channel, update_access)
    if event == "file_touch":
        return _file_touch(payload, session, channel, update_access)
    if event == "session_end":
        return _session_end(payload)
    raise ValueError(f"Unknown event: {event!r}. Expected one of: {', '.join(EVENTS)}")


def _finish(context: str, memory_ids: list[str]) -> InjectionResult:
    if not context:
        return InjectionResult(context="", memory_ids=[])
    return InjectionResult(context=context, memory_ids=memory_ids, approx_tokens=_approx_tokens(context))


def _session_start() -> InjectionResult:
    parts: list[str] = []
    for store in collect_stores():
        content = read_core(store).strip()
        if not content:
            continue
        label = "Global Core" if store.scope == "global" else "Project Core"
        parts.append(f"### {label}")
        parts.append(content)
    if not parts:
        return InjectionResult(context="")
    return _finish("## Mnemosyne Memory\n\n" + "\n\n".join(parts), [])


def _injection_params(channel: str) -> tuple[int, int, str | None]:
    stores = collect_stores()
    config = load_config(stores[-1] if stores else None)
    max_tokens = int(config.get("injection", {}).get("max_tokens", 2000))
    summary_chars = int(config.get("injection", {}).get("summary_chars", 120))
    return max_tokens, summary_chars, resolve_show_hint(config, channel)


def _turn_start(payload: dict, session: str, channel: str, update_access: bool | None) -> InjectionResult:
    prompt = str(payload.get("prompt") or "").strip()
    if len(prompt) < MIN_PROMPT_CHARS:
        return InjectionResult(context="")
    keywords = extract_keywords(prompt, limit=8)
    if not keywords:
        return InjectionResult(context="")
    already = load_injected_ids(session)
    results = run_search(
        " ".join(keywords),
        limit=3,
        update_access=True if update_access is None else update_access,
    )
    fresh = [item for item in results if item["id"] not in already]
    if not fresh:
        return InjectionResult(context="")
    max_tokens, summary_chars, show_hint = _injection_params(channel)
    context = format_for_injection(fresh, max_tokens=max_tokens, summary_chars=summary_chars, show_hint=show_hint)
    if not context:
        return InjectionResult(context="")
    ids = [item["id"] for item in fresh]
    record_injected_ids(session, ids)
    return _finish(context, ids)


def _file_touch(payload: dict, session: str, channel: str, update_access: bool | None) -> InjectionResult:
    files = [str(item) for item in (payload.get("files") or []) if str(item).strip()]
    basenames: list[str] = []
    for file_path in files:
        name = Path(file_path).name
        if name and name not in basenames:
            basenames.append(name)
    if not basenames:
        return InjectionResult(context="")
    already = load_injected_ids(session)
    results: list[dict] = []
    seen_ids: set[str] = set()
    for basename in basenames:
        for item in run_search(
            basename,
            limit=2,
            update_access=False if update_access is None else update_access,
        ):
            if item["id"] in already or item["id"] in seen_ids:
                continue
            seen_ids.add(item["id"])
            results.append(item)
    if not results:
        return InjectionResult(context="")
    max_tokens, summary_chars, show_hint = _injection_params(channel)
    body = format_for_injection(results, max_tokens=max_tokens, summary_chars=summary_chars, show_hint=show_hint)
    if not body:
        return InjectionResult(context="")
    ids = [item["id"] for item in results]
    record_injected_ids(session, ids)
    return _finish(f"## Memories relevant to {', '.join(basenames)}\n\n" + body, ids)


def _session_end(payload: dict) -> InjectionResult:
    project = find_project_store()
    config = load_config(project)
    if not config.get("distill", {}).get("enabled"):
        return InjectionResult(context="")
    from mnemosyne.distill import (
        distill_text,
        load_processed_turns,
        record_processed_turns,
        turns_to_text,
    )
    from mnemosyne.transcripts import parse_transcript

    source = str(payload.get("source") or "agent")
    transcript = payload.get("transcript")
    if isinstance(transcript, dict) and transcript.get("path"):
        path = str(transcript["path"])
        turns = parse_transcript(Path(path), str(transcript.get("format") or "auto"))
        if not turns:
            return InjectionResult(context="")
        done = load_processed_turns(path)
        if done > len(turns):
            done = 0  # transcript rotated/replaced; start over
        new_turns = turns[done:]
        if not new_turns:
            return InjectionResult(context="")
        actions = distill_text(turns_to_text(new_turns), source=source, commit=True)
        record_processed_turns(path, len(turns))
    else:
        text = str(payload.get("text") or "")
        if not text.strip():
            return InjectionResult(context="")
        actions = distill_text(text, source=source, commit=True)

    saved = [action for action in actions if action.get("id")]
    if not saved:
        return InjectionResult(context="")
    lines = ["Mnemosyne: auto-saved memories from this session:"]
    for action in saved:
        lines.append(f"- [{action['verdict']}] {action['type']}: {action['title']} ({action['id']})")
    return _finish("\n".join(lines), [action["id"] for action in saved])
