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


def test_parse_findings_json_roundtrip(tmp_store):
    from mnemosyne.handoff import parse_findings_auto
    text = '{"findings": [{"type": "pitfall", "importance": 66, "title": "T", "tags": ["a"], "content": "C"}]}'
    findings = parse_findings_auto(text)
    assert findings[0].type == "pitfall" and findings[0].importance == 66
    assert findings[0].tags == ["a"] and findings[0].content == "C"


def test_parse_findings_json_bare_array(tmp_store):
    from mnemosyne.handoff import parse_findings_json
    text = '[{"type": "codebase", "importance": 55, "title": "B", "tags": [], "content": "body"}]'
    assert parse_findings_json(text)[0].type == "codebase"


def test_parse_findings_respects_custom_types(tmp_store):
    from mnemosyne.handoff import parse_findings
    block = "**Findings:**\n- type: custom_kind\n- importance: 60\n- title: T\n- tags: a\n- content: |\n    body\n"
    assert parse_findings(block, allowed=("custom_kind",))[0].type == "custom_kind"
    assert parse_findings(block, allowed=("pitfall",)) == []


def test_ingest_json_format(tmp_store):
    from mnemosyne.handoff import ingest
    text = '{"findings": [{"type": "pitfall", "importance": 60, "title": "J", "tags": [], "content": "json body"}]}'
    actions = ingest(text, commit=False, fmt="auto")
    assert actions and actions[0]["title"] == "J"


def test_llm_prompt_uses_config_types():
    from mnemosyne.distill.llm import LLMExtractor
    extractor = LLMExtractor({"memory": {"types": ["alpha", "beta"]}, "distill": {"llm": {}}})
    assert "alpha|beta" in extractor.prompt_preview()
