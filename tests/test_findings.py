from mnemosyne.findings import Finding, allowed_types


def test_finding_fields():
    f = Finding(type="pitfall", importance=70, title="t", tags=["a"], content="c")
    assert f.evidence == ""


def test_allowed_types_fallback():
    assert "arch_decision" in allowed_types(None)
    assert "session_summary" in allowed_types({})


def test_allowed_types_from_config():
    cfg = {"memory": {"types": ["foo", "bar"]}}
    assert allowed_types(cfg) == ("foo", "bar")


def test_codex_reexport_is_same_class():
    from mnemosyne.codex import Finding as CodexFinding
    assert CodexFinding is Finding
