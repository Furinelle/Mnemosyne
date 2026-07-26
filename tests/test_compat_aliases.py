"""Backward-compat regression: every legacy surface keeps working."""


def test_legacy_imports():
    from mnemosyne.codex import (  # noqa: F401
        ALLOWED_TYPES,
        Finding,
        ingest,
        parse_findings,
        prep,
        write_finding,
    )
    from mnemosyne.distill import Turn, parse_claude_transcript  # noqa: F401
    from mnemosyne.hooks._common import (  # noqa: F401
        extract_keywords,
        format_for_injection,
        hook_safe,
        load_injected_ids,
        read_event,
        record_injected_ids,
        run_search,
    )
    from mnemosyne.hooks.pre_tool_use import main as pre_main  # noqa: F401
    from mnemosyne.hooks.session_start import main as session_main  # noqa: F401
    from mnemosyne.hooks.stop import main as stop_main  # noqa: F401
    from mnemosyne.hooks.user_prompt_submit import main as prompt_main  # noqa: F401


def test_legacy_cli_helper_reexports():
    from mnemosyne import api
    from mnemosyne.cli import (
        DEMOTE_ON_SUPERSEDE,
        add_link,
        make_memory_id,
        summarize,
        update_memory_index_file,
    )

    assert make_memory_id is api.make_memory_id
    assert summarize is api.summarize
    assert add_link is api.add_link
    assert update_memory_index_file is api.update_memory_index_file
    assert DEMOTE_ON_SUPERSEDE == api.DEMOTE_ON_SUPERSEDE


def test_legacy_cli_names():
    from mnemosyne.cli import build_parser

    parser = build_parser()
    for argv in (["codex-prep", "t"], ["codex-ingest"], ["install-hermes"]):
        assert hasattr(parser.parse_args(argv), "func")


def test_legacy_mcp_tool_name():
    from mnemosyne.mcp.server import TOOL_HANDLERS

    assert "mnemosyne_codex_prep" in TOOL_HANDLERS
    assert TOOL_HANDLERS["mnemosyne_codex_prep"] is TOOL_HANDLERS["mnemosyne_prep_context"]


def test_legacy_findings_header_still_parses(tmp_store):
    from mnemosyne.codex import parse_findings

    block = "**新发现:**\n- type: pitfall\n- importance: 60\n- title: 旧格式\n- tags: legacy\n- content: |\n    中文头部仍然可用\n"
    findings = parse_findings(block)
    assert findings and findings[0].title == "旧格式"
