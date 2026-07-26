import pytest

from mnemosyne import api


def test_write_entry_created(tmp_store):
    result = api.write_entry(type="pitfall", importance=70, content="unique content alpha", title="T1")
    assert result.status == "created"
    assert result.id.startswith("pitfall-")
    assert result.path


def test_write_entry_duplicate(tmp_store):
    first = api.write_entry(type="pitfall", importance=70, content="same body twice", title="T")
    second = api.write_entry(type="pitfall", importance=70, content="same body twice", title="T")
    assert second.status == "duplicate"
    assert second.duplicate_of == first.id
    assert second.id == first.id


def test_link_entries_unknown_rel(tmp_store):
    a = api.write_entry(type="pitfall", importance=60, content="aaa body", title="A")
    b = api.write_entry(type="codebase", importance=60, content="bbb body", title="B")
    with pytest.raises(api.MnemosyneError):
        api.link_entries(a.id, b.id, rel="nonsense")


def test_link_entries_ok(tmp_store):
    a = api.write_entry(type="pitfall", importance=60, content="aaa body", title="A")
    b = api.write_entry(type="codebase", importance=60, content="bbb body", title="B")
    result = api.link_entries(a.id, b.id, rel="related")
    assert result["ok"] is True and result["rel"] == "related"
    assert result["id1"] == a.id and result["id2"] == b.id


def test_maintain_dry_run(tmp_store):
    api.write_entry(type="pitfall", importance=70, content="ccc body", title="C")
    summary = api.maintain(dry_run=True)
    assert summary["processed"] >= 1
    assert set(summary) >= {"processed", "decayed", "deprecated", "archived", "core_candidates"}


def test_search_entries(tmp_store):
    api.write_entry(type="pitfall", importance=70, content="portalocker deadlock on windows", title="portalocker deadlock")
    results = api.search_entries("portalocker deadlock", update_access=False)
    assert results and results[0]["id"].startswith("pitfall-")
    assert "why_matched" in results[0]
