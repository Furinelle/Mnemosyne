from mnemosyne.injection import _approx_tokens, extract_keywords, format_for_injection, resolve_show_hint


def test_extract_keywords_skips_stopwords():
    assert "认证" in " ".join(extract_keywords("调试认证失败的问题"))
    assert "the" not in extract_keywords("the quick brown fox")


def test_extract_keywords_keeps_project_after_chinese_preamble():
    keywords = extract_keywords("对比github上同类项目，结合我的具体需求，帮我改进更新mnemosyne记忆系统")
    assert "mnemosyne" in keywords
    assert "记忆系统" in keywords
    assert len(keywords) <= 8
    assert extract_keywords("mnemosyne", limit=0) == []


def test_format_for_injection_no_footer():
    results = [{"id": "x-1", "scope": "project", "type": "pitfall", "tags": [], "summary": "s", "strength": 50, "score": 1.0}]
    text = format_for_injection(results, show_hint=None)
    assert "mnemosyne show" not in text and "x-1" in text


def test_format_for_injection_default_footer():
    results = [{"id": "x-1", "scope": "project", "type": "pitfall", "tags": [], "summary": "s", "strength": 50, "score": 1.0}]
    assert "mnemosyne show" in format_for_injection(results)


def test_resolve_show_hint_channels():
    assert "mnemosyne_show tool" in resolve_show_hint({}, "mcp")
    assert resolve_show_hint({}, "none") is None
    assert resolve_show_hint({"injection": {"show_command_template": "custom"}}, "cli") == "custom"
    assert "python3 -m mnemosyne show" in resolve_show_hint({}, "cli")


def test_common_shim_still_exports():
    from mnemosyne.hooks._common import extract_keywords as legacy
    assert legacy is extract_keywords


def test_session_state_roundtrip(tmp_store):
    from mnemosyne.session_state import load_injected_ids, record_injected_ids
    record_injected_ids("sess-a", ["m-1", "m-2"])
    assert load_injected_ids("sess-a") == {"m-1", "m-2"}
    assert load_injected_ids("sess-b") == set()


def test_format_chinese_budget_includes_prefix_and_provenance():
    results = [{"id": "memory-1", "scope": "project", "type": "pitfall", "tags": [],
                "summary": "并发记忆系统中文运维检索" * 30, "strength": 50, "score": 1,
                "source": "codex", "created": "2026-09-12"}]
    for budget in (60, 80, 100):
        ids = []
        text = format_for_injection(results, max_tokens=budget, summary_chars=1000,
                                    prefix="## Memories relevant to 服务器配置.py", emitted_ids=ids)
        assert text and _approx_tokens(text) <= budget
        assert ids == ["memory-1"]
        assert "[codex; recorded 2026-09-12]" in text


def test_format_budget_never_marks_unrendered_memory():
    results = [{"id": f"memory-{i}", "scope": "project", "type": "pitfall", "tags": [],
                "summary": "A" * 120, "strength": 50, "score": 4 - i} for i in (1, 2, 3)]
    ids = []
    text = format_for_injection(results, max_tokens=80, emitted_ids=ids)
    assert ids == ["memory-1"]
    assert "memory-1" in text and "memory-2" not in text
    ids = []
    assert format_for_injection(results, max_tokens=5, emitted_ids=ids) == ""
    assert not ids


def test_session_state_expired_and_malformed_entries_are_ignored(tmp_path, monkeypatch):
    import json
    from datetime import datetime, timedelta, timezone
    from mnemosyne.session_state import load_injected_ids, record_injected_ids

    state = tmp_path / "state.json"
    monkeypatch.setattr("mnemosyne.session_state._session_state_path", lambda: state)
    now = datetime.now(timezone.utc)
    state.write_text(json.dumps({"sessions": {
        "old": {"ts": (now - timedelta(hours=49)).isoformat(), "ids": ["expired"]},
        "new": {"ts": now.isoformat(), "ids": ["fresh"]},
        "bad": [],
    }}))
    assert not load_injected_ids("old")
    assert load_injected_ids("new") == {"fresh"}
    record_injected_ids("new", ["another"])
    assert load_injected_ids("new") == {"fresh", "another"}
    assert "old" not in json.loads(state.read_text())["sessions"]
    for bad in ([], {"sessions": []}, {"sessions": {"new": {"ids": [1]}}}):
        state.write_text(json.dumps(bad))
        assert not load_injected_ids("new")
        record_injected_ids("new", ["recovered"])
        assert load_injected_ids("new") == {"recovered"}


def test_session_state_concurrent_writes_keep_all_ids(tmp_path, monkeypatch):
    from concurrent.futures import ThreadPoolExecutor
    from mnemosyne.session_state import load_injected_ids, record_injected_ids

    monkeypatch.setattr("mnemosyne.session_state._session_state_path", lambda: tmp_path / "state.json")
    with ThreadPoolExecutor(max_workers=4) as executor:
        list(executor.map(lambda i: record_injected_ids(f"s-{i % 2}", [f"m-{i}"]), range(20)))
    for session in (0, 1):
        assert load_injected_ids(f"s-{session}") == {f"m-{i}" for i in range(session, 20, 2)}
