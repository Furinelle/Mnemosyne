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
    api.write_entry(type="pitfall", importance=80, content="portalocker deadlock on windows", title="portalocker deadlock")
    first = handle_event("turn_start", {"prompt": "portalocker deadlock question"}, session="s2")
    second = handle_event("turn_start", {"prompt": "portalocker deadlock question"}, session="s2")
    assert first.memory_ids and not second.memory_ids


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
