from mnemosyne.handoff import prep


def test_prep_channel_cli_uses_python3(tmp_store):
    text = prep("some task")
    assert "python3 -m mnemosyne search" in text


def test_prep_channel_mcp_mentions_tool(tmp_store):
    text = prep("some task", channel="mcp")
    assert "mnemosyne_search" in text
    assert "python" not in text.lower()


def test_prep_channel_none_omits_cli_section(tmp_store):
    text = prep("some task", channel="none")
    assert "CLI available" not in text
    assert "mnemosyne_search" not in text


def test_codex_shim():
    import mnemosyne.codex as codex
    from mnemosyne import handoff
    assert codex.prep is handoff.prep
    assert codex.parse_findings is handoff.parse_findings
    assert codex.ingest is handoff.ingest
    assert codex.write_finding is handoff.write_finding
