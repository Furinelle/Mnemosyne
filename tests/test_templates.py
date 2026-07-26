from mnemosyne.store import template_text


def test_generic_agents_template():
    text = template_text("agents/generic/AGENTS.md")
    assert "mnemosyne search" in text
    assert "Findings" in text
    assert "mcp serve" in text


def test_per_agent_template_dirs():
    assert template_text("agents/codex/AGENTS.md")
    assert template_text("agents/claude_code/CLAUDE.md")
    assert template_text("agents/claude_code/settings.json")


def test_legacy_template_names_still_work():
    assert template_text("AGENTS.md")
    assert template_text("CLAUDE.md")
    assert template_text("core_project.md")
    assert template_text("settings.json")
