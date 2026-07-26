"""Adapter conformance suite: any new adapter's event mapping must pass these."""

import json

from mnemosyne import api
from mnemosyne.events import EVENTS, handle_event


def test_all_events_return_injection_result(tmp_store):
    payloads = {
        "session_start": {},
        "turn_start": {"prompt": "a prompt that is long enough"},
        "file_touch": {"files": ["a.py"]},
        "session_end": {"text": "[user] hi\n[assistant] done"},
    }
    for event in EVENTS:
        result = handle_event(event, payloads[event], session="contract")
        assert isinstance(result.context, str)
        assert isinstance(result.memory_ids, list)
        assert isinstance(result.approx_tokens, int)


def test_findings_roundtrip_markdown_and_json(tmp_store):
    from mnemosyne.handoff import ingest

    md = "**Findings:**\n- type: pitfall\n- importance: 60\n- title: T\n- tags: a\n- content: |\n    body md\n"
    js = json.dumps({"findings": [{"type": "pitfall", "importance": 60, "title": "T2", "tags": [], "content": "body js"}]})
    assert ingest(md, commit=False)[0]["title"] == "T"
    assert ingest(js, commit=False, fmt="auto")[0]["title"] == "T2"


def test_distill_idempotent(tmp_store):
    from mnemosyne.distill import distill_text

    text = "[user] 记住：以后都用 uv 不要用 pip，这是长期偏好\n[assistant] 好的，已了解这个偏好"
    first = distill_text(text, source="agent", commit=True)
    second = distill_text(text, source="agent", commit=True)
    saved_first = [a for a in first if a.get("id")]
    if saved_first:  # heuristic engine extracted something -> rerun must dedup
        assert all(a["verdict"] == "duplicate" for a in second if a.get("id"))


def test_prep_ingest_full_cycle(tmp_store):
    api.write_entry(type="codebase", importance=70, content="widget service binds port 9090", title="widget service")
    context = api.prep_context("investigate the widget service", channel="none")
    assert "widget" in context.lower()
    reply = "done.\n\n**Findings:**\n- type: pitfall\n- importance: 65\n- title: Port conflict\n- tags: widget\n- content: |\n    9090 clashes with the dev proxy\n"
    actions = api.ingest_findings(reply, source="contract-agent", commit=True)
    assert actions and actions[0].get("id")
    hits = api.search_entries("port conflict dev proxy", update_access=False)
    assert any(a["id"].startswith("pitfall-") for a in hits)
