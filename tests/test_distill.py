from mnemosyne.codex import Finding
from mnemosyne.distill import Turn, classify_against_store, distill_text, jaccard, parse_claude_transcript
from mnemosyne.distill.heuristic import HeuristicExtractor
from mnemosyne.distill.llm import LLMExtractor, _parse_llm_json


def test_parse_claude_transcript_extracts_role_and_text(tmp_path):
    lines = [
        '{"type":"user","message":{"role":"user","content":"用 ruff 不要用 flake8"}}',
        '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"好的，已切换"},{"type":"tool_use","name":"Edit","input":{}}]}}',
        '{"type":"system","message":{"role":"system","content":"ignored"}}',
    ]
    path = tmp_path / "transcript.jsonl"
    path.write_text("\n".join(lines), encoding="utf-8")

    turns = parse_claude_transcript(path)

    assert turns == [
        Turn(role="user", text="用 ruff 不要用 flake8"),
        Turn(role="assistant", text="好的，已切换"),
    ]


def test_heuristic_extracts_preference_from_user_correction():
    turns = [Turn(role="user", text="不要用 print 调试，改用 logging 模块")]
    findings = HeuristicExtractor(confidence_threshold=0.6).extract(turns)
    assert len(findings) == 1
    assert findings[0].type == "preference"
    assert "logging" in findings[0].content


def test_heuristic_extracts_pitfall_from_error_plus_fix():
    turns = [
        Turn(
            role="assistant",
            text="出现 Traceback，根因是连接池未释放，修复为在 finally 中 close()",
        )
    ]
    findings = HeuristicExtractor(confidence_threshold=0.6).extract(turns)
    assert len(findings) == 1
    assert findings[0].type == "pitfall"


def test_heuristic_drops_below_confidence_threshold():
    turns = [Turn(role="user", text="今天天气不错")]
    findings = HeuristicExtractor(confidence_threshold=0.6).extract(turns)
    assert findings == []


def test_heuristic_respects_max_findings():
    turns = [Turn(role="user", text=f"不要用 a{i}，改用 b{i}") for i in range(10)]
    findings = HeuristicExtractor(confidence_threshold=0.6, max_findings=3).extract(turns)
    assert len(findings) == 3


def test_jaccard_basic():
    assert jaccard(["a", "b", "c"], ["a", "b"]) == 2 / 3
    assert jaccard([], []) == 0.0


def test_classify_new_when_store_empty(tmp_path, monkeypatch):
    monkeypatch.setenv("MNEMOSYNE_HOME", str(tmp_path / "global"))
    monkeypatch.chdir(tmp_path)  # isolate from the repo's real .mnemosyne
    finding = Finding("pitfall", 70, "全新的坑", ["x"], "一段独特内容 zzz")
    verdict, target = classify_against_store(
        finding, dedup_threshold=0.85, subject_threshold=0.5
    )
    assert verdict == "new"
    assert target is None


def test_distill_text_dry_run_writes_nothing(tmp_path, monkeypatch):
    monkeypatch.setenv("MNEMOSYNE_HOME", str(tmp_path / "global"))
    monkeypatch.chdir(tmp_path)
    transcript = "[user] 不要用 print 调试，改用 logging"
    actions = distill_text(transcript, source="claude-code", commit=False)
    assert len(actions) == 1
    assert actions[0]["verdict"] == "new"
    assert "id" not in actions[0]  # dry-run: not persisted


def test_distill_text_commit_persists(tmp_path, monkeypatch):
    from mnemosyne.store import ensure_store, project_store

    monkeypatch.setenv("MNEMOSYNE_HOME", str(tmp_path / "global"))
    monkeypatch.chdir(tmp_path)
    ensure_store(project_store())
    transcript = "[user] 不要用 print 调试，改用 logging"
    actions = distill_text(transcript, source="claude-code", commit=True)
    assert actions[0]["verdict"] == "new"
    assert actions[0]["id"].startswith("preference-")


def test_parse_llm_json_extracts_findings():
    payload = '[{"type":"pitfall","importance":80,"title":"X","tags":["a"],"content":"because Y"}]'
    findings = _parse_llm_json(payload)
    assert len(findings) == 1
    assert findings[0].type == "pitfall"
    assert findings[0].importance == 80


def test_parse_llm_json_drops_unknown_type():
    payload = '[{"type":"nonsense","importance":50,"title":"X","tags":[],"content":"Y"}]'
    assert _parse_llm_json(payload) == []


def test_parse_role_lines_preserves_multiline_turn():
    from mnemosyne.distill import _parse_role_lines, turns_to_text

    turns = [Turn(role="user", text="先说背景\n不要用 print，改用 logging")]
    # round-trip through the flat text format used by every distill entry point
    assert _parse_role_lines(turns_to_text(turns)) == turns


def test_findings_from_text_multiline_user_turn_keeps_preference():
    from mnemosyne.distill import _findings_from_text, turns_to_text

    # the preference phrase lands on a continuation line; it must still be
    # attributed to the user turn and produce a preference finding.
    text = turns_to_text([Turn(role="user", text="背景说明\n不要用 print，改用 logging")])
    findings = _findings_from_text(text, {})
    assert len(findings) == 1
    assert findings[0].type == "preference"
