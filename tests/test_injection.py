from mnemosyne.injection import extract_keywords, format_for_injection, resolve_show_hint


def test_extract_keywords_skips_stopwords():
    assert "认证" in extract_keywords("调试认证失败的问题")
    assert "the" not in extract_keywords("the quick brown fox")


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
