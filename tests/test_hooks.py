from __future__ import annotations

import unittest

from mnemosyne.hooks._common import format_for_injection


class InjectionFormattingTests(unittest.TestCase):
    def test_format_for_injection_respects_approx_token_budget(self) -> None:
        results = [
            {
                "id": "memory-1",
                "scope": "project",
                "type": "codebase",
                "tags": ["alpha"],
                "summary": "A" * 240,
                "score": 3.0,
                "strength": 90,
            },
            {
                "id": "memory-2",
                "scope": "project",
                "type": "pitfall",
                "tags": ["beta"],
                "summary": "B" * 240,
                "score": 2.0,
                "strength": 40,
            },
        ]

        text = format_for_injection(results, max_tokens=80)

        self.assertIn("memory-1", text)
        self.assertNotIn("memory-2", text)
        self.assertLessEqual(len(text) // 4, 80)

    def test_format_for_injection_ranks_relevance_over_strength(self) -> None:
        results = [
            {
                "id": "strong-stale", "scope": "global", "type": "preference",
                "tags": [], "summary": "irrelevant but strong", "score": 0.5, "strength": 95,
            },
            {
                "id": "weak-relevant", "scope": "project", "type": "pitfall",
                "tags": [], "summary": "exact topical hit", "score": 8.0, "strength": 35,
            },
        ]

        text = format_for_injection(results)

        self.assertLess(text.index("weak-relevant"), text.index("strong-stale"))


if __name__ == "__main__":
    unittest.main()


import io
import json

from mnemosyne.hooks import stop
from mnemosyne.store import ensure_store, load_memories, project_store


def test_stop_hook_distills_when_enabled(tmp_path, monkeypatch, capsys):
    monkeypatch.setenv("MNEMOSYNE_HOME", str(tmp_path / "global"))
    monkeypatch.chdir(tmp_path)
    store = project_store()
    ensure_store(store)
    # enable distill in project config
    store.config_path.write_text(
        store.config_path.read_text(encoding="utf-8").replace(
            "enabled = false\nengine", "enabled = true\nengine"
        ),
        encoding="utf-8",
    )
    transcript = tmp_path / "t.jsonl"
    transcript.write_text(
        '{"type":"user","message":{"role":"user","content":"不要用 print 调试，改用 logging"}}',
        encoding="utf-8",
    )
    event = {"transcript_path": str(transcript), "stop_hook_active": False}
    monkeypatch.setattr("sys.stdin", io.StringIO(json.dumps(event)))

    stop.main()

    saved = load_memories(store)
    assert any(m.type == "preference" for _, m in saved)


def test_injected_ids_roundtrip(tmp_path, monkeypatch):
    from mnemosyne.hooks import _common

    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setenv("MNEMOSYNE_HOME", str(home))
    monkeypatch.chdir(tmp_path)  # no project store -> state lives in global root

    _common.record_injected_ids("s1", ["m-1", "m-2"])
    _common.record_injected_ids("s1", ["m-3"])

    assert _common.load_injected_ids("s1") == {"m-1", "m-2", "m-3"}
    assert _common.load_injected_ids("s2") == set()
    assert _common.load_injected_ids("") == set()


def test_user_prompt_submit_skips_already_injected(tmp_path, monkeypatch, capsys):
    from mnemosyne.hooks import user_prompt_submit
    from mnemosyne.schema import Memory
    from mnemosyne.store import working_path, write_memory

    monkeypatch.setenv("MNEMOSYNE_HOME", str(tmp_path / "home"))
    project = tmp_path / "project"
    project.mkdir()
    monkeypatch.chdir(project)
    store = project_store()
    ensure_store(store)
    memory = Memory(
        id="pitfall-1", type="pitfall", strength=80,
        injection_summary="JWT 认证失败排查记录",
        body="## JWT 认证失败排查\n\ntoken 过期导致认证失败",
        tags=["jwt"],
    )
    write_memory(working_path(store, memory), memory)

    event = json.dumps({"session_id": "s1", "prompt": "JWT 认证失败怎么排查一下"})

    monkeypatch.setattr("sys.stdin", io.StringIO(event))
    user_prompt_submit.main()
    first = capsys.readouterr().out
    assert "pitfall-1" in first

    monkeypatch.setattr("sys.stdin", io.StringIO(event))
    user_prompt_submit.main()
    second = capsys.readouterr().out
    assert "pitfall-1" not in second


def test_maybe_run_maintain_throttles_global_across_projects(tmp_path, monkeypatch):
    from mnemosyne.hooks import session_start

    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setenv("MNEMOSYNE_HOME", str(home))
    calls: list[list[str]] = []
    monkeypatch.setattr(
        session_start.subprocess, "Popen", lambda cmd, **kwargs: calls.append(cmd)
    )

    for name in ("project_a", "project_b"):
        project = tmp_path / name
        (project / ".mnemosyne" / "working").mkdir(parents=True)
        (project / ".mnemosyne" / "core.md").write_text("core", encoding="utf-8")
        monkeypatch.chdir(project)
        session_start.maybe_run_maintain()

    global_calls = [cmd for cmd in calls if "global" in cmd]
    project_calls = [cmd for cmd in calls if "project" in cmd]
    assert len(global_calls) == 1, "global store must be maintained once, not once per project"
    assert len(project_calls) == 2
    assert (home / ".last_maintain").exists()


def test_maybe_run_maintain_runs_again_after_interval(tmp_path, monkeypatch):
    from datetime import datetime, timedelta

    from mnemosyne.hooks import session_start

    home = tmp_path / "home"
    home.mkdir()
    stale = (datetime.now() - timedelta(hours=25)).isoformat()
    (home / ".last_maintain").write_text(stale, encoding="utf-8")
    monkeypatch.setenv("MNEMOSYNE_HOME", str(home))
    monkeypatch.chdir(tmp_path)  # no project store here
    calls: list[list[str]] = []
    monkeypatch.setattr(
        session_start.subprocess, "Popen", lambda cmd, **kwargs: calls.append(cmd)
    )

    session_start.maybe_run_maintain()

    assert [cmd for cmd in calls if "global" in cmd], "stale marker must trigger a new run"


def test_stop_hook_skips_when_stop_hook_active(tmp_path, monkeypatch):
    monkeypatch.setenv("MNEMOSYNE_HOME", str(tmp_path / "global"))
    monkeypatch.chdir(tmp_path)
    store = project_store()
    ensure_store(store)
    store.config_path.write_text(
        store.config_path.read_text(encoding="utf-8").replace(
            "enabled = false\nengine", "enabled = true\nengine"
        ),
        encoding="utf-8",
    )
    transcript = tmp_path / "t.jsonl"
    transcript.write_text(
        '{"type":"user","message":{"role":"user","content":"不要用 print，改用 logging"}}',
        encoding="utf-8",
    )
    event = {"transcript_path": str(transcript), "stop_hook_active": True}
    monkeypatch.setattr("sys.stdin", io.StringIO(json.dumps(event)))

    stop.main()

    assert load_memories(store) == []

