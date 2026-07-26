from mnemosyne.cli import build_parser, main


def test_prep_and_alias_share_handler():
    parser = build_parser()
    prep_args = parser.parse_args(["prep", "task"])
    legacy_args = parser.parse_args(["codex-prep", "task"])
    assert prep_args.func is legacy_args.func


def test_ingest_and_alias_share_handler():
    parser = build_parser()
    new = parser.parse_args(["ingest"])
    legacy = parser.parse_args(["codex-ingest"])
    assert new.func is legacy.func
    assert new.fmt == "auto"


def test_install_hermes_alias():
    parser = build_parser()
    new = parser.parse_args(["install", "hermes"])
    legacy = parser.parse_args(["install-hermes"])
    assert new.agent == "hermes"
    assert hasattr(legacy, "func")


def test_init_no_agent_files(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    assert main(["init", "--no-agent-files"]) == 0
    assert not (tmp_path / "AGENTS.md").exists()
    assert (tmp_path / ".mnemosyne").is_dir()


def test_init_default_writes_generic_agents_md(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    assert main(["init"]) == 0
    text = (tmp_path / "AGENTS.md").read_text(encoding="utf-8")
    assert "Long-Term Memory via Mnemosyne" in text


def test_init_agent_codex_writes_codex_protocol(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    assert main(["init", "--agent", "codex"]) == 0
    text = (tmp_path / "AGENTS.md").read_text(encoding="utf-8")
    assert "Agent Coordination via Mnemosyne" in text


def test_write_source_normalized(tmp_store, capsys):
    assert main(["write", "--type", "pitfall", "--importance", "60",
                 "--title", "T", "--content", "src body", "--force",
                 "--source", "  MyAgent:Dev  "]) == 0
    out = capsys.readouterr().out
    memory_id = out.strip().splitlines()[-1].removeprefix("Wrote ")
    from mnemosyne.store import find_memory, stores_for_scope
    found = find_memory(memory_id, stores_for_scope("all"), include_archive=False)
    assert found[2].source == "myagent:dev"
