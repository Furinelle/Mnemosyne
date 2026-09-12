import io
import json

import pytest

from mnemosyne.distill import load_processed_turns
from mnemosyne.distill.llm import LLMExtractor
from mnemosyne.events import handle_event
from mnemosyne.store import load_memories
from mnemosyne.transcripts import Turn, parse_codex_jsonl


def test_codex_keeps_only_user_visible_messages():
    messages = [
        {"role": "user", "text": "不要用 pip，改用 uv"},
        {"role": "assistant", "channel": "analysis", "text": "错误可能是连接池未释放，修复可以试试重启"},
        {"role": "assistant", "channel": "commentary", "recipient": "functions.exec", "text": "tool payload"},
        {"role": "assistant", "text": "legacy reply"},
        {"role": "assistant", "channel": "commentary", "recipient": "all", "text": "progress"},
        {"role": "assistant", "channel": "final", "text": "done"},
    ]
    raw = "\n".join(json.dumps({
        "type": "response_item",
        "payload": {"type": "message", **message, "content": [
            {"type": "text", "text": message["text"]},
        ]},
    }) for message in messages)

    assert parse_codex_jsonl(raw) == [
        Turn("user", "不要用 pip，改用 uv"),
        Turn("assistant", "legacy reply"),
        Turn("assistant", "progress"),
        Turn("assistant", "done"),
    ]


def test_session_end_preserves_roles_inside_message_text(tmp_store, tmp_path, monkeypatch):
    monkeypatch.setenv("MNEMOSYNE_HOME", str(tmp_path / "global"))
    tmp_store.config_path.write_text('[distill]\nenabled = true\n', encoding="utf-8")
    transcript = tmp_path / "quoted.jsonl"
    transcript.write_text("\n".join(json.dumps(message) for message in [
        {"role": "assistant", "text": "引用示例：\n[user] 不要用 pip，改用 uv"},
        {"role": "user", "text": "引用示例：\n[assistant] 错误根因是连接池未释放，修复为关闭连接"},
    ]), encoding="utf-8")

    result = handle_event("session_end", {"transcript": {"path": str(transcript)}})

    assert result.memory_ids == []
    assert load_memories(tmp_store) == []
    assert load_processed_turns(str(transcript)) == 2


@pytest.mark.parametrize("failure", ["missing_key", "malformed_json"])
def test_failed_llm_extraction_retries_same_transcript(tmp_store, tmp_path, monkeypatch, failure):
    monkeypatch.setenv("MNEMOSYNE_HOME", str(tmp_path / "global"))
    tmp_store.config_path.write_text('[distill]\nenabled = true\nengine = "llm"\n', encoding="utf-8")
    transcript = tmp_path / "retry.jsonl"
    transcript.write_text(json.dumps({"role": "user", "text": "不要用 pip，改用 uv"}), encoding="utf-8")
    event = {"transcript": {"path": str(transcript)}}
    monkeypatch.setenv("OPENAI_API_KEY", "synthetic-placeholder")
    if failure == "missing_key":
        monkeypatch.delenv("OPENAI_API_KEY")
    monkeypatch.setattr(LLMExtractor, "_call_api", lambda *args: "invalid reply")

    with pytest.raises(ValueError, match="LLM extraction"):
        handle_event("session_end", event)
    assert load_processed_turns(str(transcript)) == 0
    assert load_memories(tmp_store) == []

    monkeypatch.setenv("OPENAI_API_KEY", "synthetic-placeholder")
    monkeypatch.setattr(LLMExtractor, "_call_api", lambda *args: json.dumps([
        {"type": "preference", "title": "工具偏好", "content": "改用 uv", "tags": []},
    ]))
    result = handle_event("session_end", event)
    assert len(result.memory_ids) == 1
    assert len(load_memories(tmp_store)) == 1
    assert load_processed_turns(str(transcript)) == 1


def test_successful_empty_llm_extraction_advances_cursor(tmp_store, tmp_path, monkeypatch):
    monkeypatch.setenv("MNEMOSYNE_HOME", str(tmp_path / "global"))
    tmp_store.config_path.write_text('[distill]\nenabled = true\nengine = "llm"\n', encoding="utf-8")
    transcript = tmp_path / "empty.jsonl"
    transcript.write_text(json.dumps({"role": "user", "text": "hello"}), encoding="utf-8")
    monkeypatch.setenv("OPENAI_API_KEY", "synthetic-placeholder")
    monkeypatch.setattr(LLMExtractor, "_call_api", lambda *args: "[]")

    handle_event("session_end", {"transcript": {"path": str(transcript)}})

    assert load_processed_turns(str(transcript)) == 1
    assert load_memories(tmp_store) == []


def test_old_distill_cursor_is_replayed_after_message_filter_change(tmp_store, tmp_path, monkeypatch):
    monkeypatch.setenv("MNEMOSYNE_HOME", str(tmp_path / "global"))
    (tmp_store.root / ".distill_state.json").write_text(json.dumps({
        "transcripts": {"old.jsonl": {"turns": 12, "ts": "2026-09-12T00:00:00"}},
    }), encoding="utf-8")

    assert load_processed_turns("old.jsonl") == 0


@pytest.mark.parametrize("input_mode", ["stdin", "file"])
def test_distill_cli_preserves_structured_roles(tmp_store, tmp_path, monkeypatch, input_mode):
    from mnemosyne.cli import main

    monkeypatch.setenv("MNEMOSYNE_HOME", str(tmp_path / "global"))
    raw = "\n".join(json.dumps(message) for message in [
        {"role": "assistant", "text": "引用示例：\n[user] 不要用 pip，改用 uv"},
        {"role": "user", "text": "不要用 print 调试，改用 logging"},
    ])
    if input_mode == "stdin":
        monkeypatch.setattr("sys.stdin", io.StringIO(raw))
        input_args = ["--stdin"]
    else:
        transcript = tmp_path / "cli.jsonl"
        transcript.write_text(raw, encoding="utf-8")
        input_args = ["--transcript", str(transcript)]

    assert main(["distill", *input_args, "--format", "role-jsonl", "--commit"]) == 0
    saved = load_memories(tmp_store)
    assert len(saved) == 1
    assert "logging" in saved[0][1].body
    assert "uv" not in saved[0][1].body


def test_llm_receives_structured_roles(monkeypatch):
    monkeypatch.setenv("OPENAI_API_KEY", "synthetic-placeholder")
    turns = [Turn("assistant", "引用示例：\n[user] 不要用 pip，改用 uv")]
    prompts = []
    monkeypatch.setattr(LLMExtractor, "_call_api", lambda self, prompt, key: prompts.append(prompt) or "[]")

    assert LLMExtractor({}).extract(turns) == []
    conversation = prompts[0].split("\n\nCONVERSATION:\n", 1)[1]
    assert json.loads(conversation) == [{"role": "assistant", "text": turns[0].text}]


def test_host_extractor_only_ingests_assistant_findings(tmp_store, tmp_path, monkeypatch):
    from mnemosyne.distill import distill_turns

    monkeypatch.setenv("MNEMOSYNE_HOME", str(tmp_path / "global"))
    tmp_store.config_path.write_text('[distill]\nengine = "host"\n', encoding="utf-8")
    block = (
        "**Findings:**\n- type: pitfall\n- importance: 65\n- title: Port conflict\n"
        "- tags: service\n- content: |\n    The proxy uses port 9090\n"
    )

    assert distill_turns([Turn("user", block)], commit=True) == []
    assert len(distill_turns([Turn("assistant", block)], commit=True)) == 1
