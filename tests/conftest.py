import pytest


@pytest.fixture
def tmp_store(tmp_path, monkeypatch):
    """Isolated global+project stores: HOME and cwd both point into tmp_path."""
    home = tmp_path / "home"
    project = tmp_path / "project"
    (project / ".git").mkdir(parents=True)
    home.mkdir()
    monkeypatch.setenv("HOME", str(home))
    monkeypatch.setenv("USERPROFILE", str(home))
    monkeypatch.chdir(project)
    from mnemosyne.store import Store, ensure_store, template_text
    store = Store("project", project / ".mnemosyne")
    ensure_store(store, template_text("core_project.md"))
    return store
