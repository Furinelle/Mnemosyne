import json

import pytest

from mnemosyne import api
from mnemosyne.events import handle_event


def test_session_start_includes_core(tmp_store):
    tmp_store.core_path.write_text("# Project Core Memory\n\n- remember the basics\n", encoding="utf-8")
    result = handle_event("session_start", {})
    assert "Project Core" in result.context
    assert result.approx_tokens > 0


def test_turn_start_injects_relevant_memory(tmp_store):
    api.write_entry(type="pitfall", importance=80, content="portalocker deadlock on windows", title="portalocker deadlock")
    result = handle_event("turn_start", {"prompt": "why does portalocker deadlock happen"}, session="s1")
    assert result.memory_ids and "portalocker" in result.context


def test_turn_start_dedups_by_session(tmp_store):
    from mnemosyne.store import load_memories

    api.write_entry(type="pitfall", importance=80, content="portalocker deadlock on windows", title="portalocker deadlock")
    first = handle_event("turn_start", {"prompt": "portalocker deadlock question"}, session="s2")
    after_first = load_memories(tmp_store)[0][1]
    second = handle_event("turn_start", {"prompt": "portalocker deadlock question"}, session="s2")
    after_second = load_memories(tmp_store)[0][1]
    assert first.memory_ids and not second.memory_ids
    assert after_first.access_count == after_second.access_count == 1
    assert after_first.strength == after_second.strength


@pytest.mark.parametrize("indexed", [True, False])
@pytest.mark.parametrize("event,update_access,expected_access", [
    ("turn_start", None, 1),
    ("turn_start", False, 0),
    ("file_touch", None, 0),
    ("file_touch", True, 1),
])
def test_events_count_access_only_after_rendering(tmp_store, monkeypatch, indexed, event, update_access, expected_access):
    from mnemosyne.schema import Memory
    from mnemosyne.store import load_memories, working_path, write_memory

    tmp_store.config_path.write_text(f"[search]\nindex_enabled = {str(indexed).lower()}\n")
    for i in range(3):
        memory = Memory(id=f"memory-{i}", type="codebase", strength=60,
                        injection_summary="budgetneedle " + "并发检索" * 50, body="budgetneedle")
        write_memory(working_path(tmp_store, memory), memory)
    monkeypatch.setattr("mnemosyne.events._injection_params", lambda channel: (80, 1000, None))
    payload = {"prompt": "budgetneedle retrieval memories", "files": ["/tmp/budgetneedle.py"]}

    result = handle_event(event, payload, session="access", update_access=update_access)
    assert len(result.memory_ids) == 1
    for _, memory in load_memories(tmp_store):
        assert memory.access_count == (expected_access if memory.id in result.memory_ids else 0)

    # A budget too small to render anything must not count even a fresh hit.
    monkeypatch.setattr("mnemosyne.events._injection_params", lambda channel: (1, 1000, None))
    empty = handle_event(event, payload, session="access", update_access=update_access)
    assert not empty.memory_ids
    assert sum(memory.access_count for _, memory in load_memories(tmp_store)) == expected_access


@pytest.mark.parametrize("event,payload", [
    ("turn_start", {"prompt": "find the relevant memory notes"}),
    ("file_touch", {"files": ["/tmp/服务器配置.py"]}),
])
def test_budgeted_events_only_dedup_emitted_memories(tmp_path, monkeypatch, event, payload):
    from mnemosyne.session_state import load_injected_ids

    items = [{"id": f"memory-{i}", "scope": "project", "type": "pitfall", "tags": [],
              "summary": "并发检索" * 40, "strength": 50, "score": 4 - i} for i in (1, 2, 3)]
    monkeypatch.setattr("mnemosyne.events.run_search", lambda *args, **kwargs: items)
    monkeypatch.setattr("mnemosyne.events._injection_params", lambda channel: (80, 1000, None))
    monkeypatch.setattr("mnemosyne.session_state._session_state_path", lambda: tmp_path / "state.json")

    first = handle_event(event, payload, session="budget")
    assert first.memory_ids == ["memory-1"]
    assert first.approx_tokens <= 80
    assert load_injected_ids("budget") == {"memory-1"}
    second = handle_event(event, payload, session="budget")
    assert second.memory_ids == ["memory-2"]
    assert second.approx_tokens <= 80


def test_turn_start_short_prompt_empty(tmp_store):
    result = handle_event("turn_start", {"prompt": "hi"})
    assert result.context == "" and not result.memory_ids


def test_file_touch_matches_basename(tmp_store):
    api.write_entry(type="codebase", importance=60, content="store.py handles locking", title="store.py notes", tags=["store.py"])
    result = handle_event("file_touch", {"files": ["/x/y/store.py"]}, session="s3")
    assert result.memory_ids
    assert "store.py" in result.context


def test_session_end_distill_disabled(tmp_store):
    result = handle_event("session_end", {"text": "[user] hi\n[assistant] done"})
    assert result.context == "" and not result.memory_ids


def test_unknown_event_raises(tmp_store):
    with pytest.raises(ValueError):
        handle_event("bogus", {})


def test_inject_cli_json(tmp_store, capsys, monkeypatch):
    import io
    import sys

    from mnemosyne.cli import main

    monkeypatch.setattr(sys, "stdin", io.StringIO(json.dumps({"prompt": "anything at all here"})))
    assert main(["inject", "--event", "turn_start", "--format", "json"]) == 0
    payload = json.loads(capsys.readouterr().out)
    assert set(payload) == {"context", "memory_ids", "approx_tokens"}


def test_inject_cli_fail_safe(tmp_store, capsys, monkeypatch):
    import io
    import sys

    from mnemosyne.cli import main

    monkeypatch.setattr(sys, "stdin", io.StringIO("this is not json"))
    assert main(["inject", "--event", "turn_start", "--fail-safe"]) == 0
    assert capsys.readouterr().out == ""
