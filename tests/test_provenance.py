"""Source and validity survive every public write/search interface."""

import json
from datetime import date

import pytest

from mnemosyne import api
from mnemosyne.cli import main
from mnemosyne.mcp.server import TOOL_SCHEMAS, _search, _write
from mnemosyne.store import find_memory


@pytest.mark.parametrize("writer", ["api", "cli", "mcp"])
def test_source_evidence_and_dates_round_trip(tmp_store, capsys, writer):
    fields = dict(type="codebase", importance=70, title="部署验证", content="relayprobe 出口验证通过",
                  source="codex", evidence="reports/relay-check.md:12", expires="2999-01-01")
    if writer == "api":
        api.write_entry(**fields)
    elif writer == "mcp":
        _write(fields)
    else:
        assert main(["write", "--force", *[arg for key, value in fields.items()
                                         for arg in (f"--{key}", str(value))]]) == 0
        capsys.readouterr()

    before = find_memory(api.search_entries("relayprobe", update_access=False)[0]["id"], [tmp_store])[2]
    mcp_result = _search({"query": "relayprobe", "scope": "project"})[0]
    after = find_memory(before.id, [tmp_store])[2]
    assert after.access_count == before.access_count  # MCP search remains read-only.
    assert main(["search", "relayprobe", "--format", "json"]) == 0
    cli_result = json.loads(capsys.readouterr().out)[0]
    for result in (mcp_result, cli_result):
        assert result["source"] == "codex"
        assert result["evidence"] == fields["evidence"]
        assert result["created"] == date.today().isoformat()
        assert result["expires"] == "2999-01-01"
        assert result["status"] == "active" and not result["expired"]
        assert result["invalidated_by"] == ""


def test_mcp_history_exposes_superseding_source(tmp_store):
    old = api.write_entry(type="codebase", importance=70, title="旧部署", content="relayprobe 位于机房 A")
    new = api.write_entry(type="arch_decision", importance=70, title="迁移", content="新部署改用机房 B")
    api.link_entries(new.id, old.id, "supersedes")
    assert _search({"query": "relayprobe", "scope": "project"}) == []
    history = _search({"query": "relayprobe", "scope": "project", "include_superseded": True})
    assert history[0]["status"] == "superseded"
    assert history[0]["invalidated_by"] == new.id
    schemas = {tool["name"]: tool["inputSchema"]["properties"] for tool in TOOL_SCHEMAS}
    assert "include_superseded" in schemas["mnemosyne_search"]
    assert {"expires", "evidence"} <= schemas["mnemosyne_write"].keys()
