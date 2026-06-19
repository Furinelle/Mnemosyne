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


def test_classify_dedup_compares_full_body_not_truncated_summary(tmp_path, monkeypatch):
    """Regression: long findings were re-written because dedup compared the new
    finding against the stored memory's ~220-char summary, never reaching the
    threshold. Dedup must compare against the full body (observed: 9 copies)."""
    from mnemosyne.codex import Finding, write_finding
    from mnemosyne.store import ensure_store, project_store

    monkeypatch.setenv("MNEMOSYNE_HOME", str(tmp_path / "global"))
    monkeypatch.chdir(tmp_path)
    ensure_store(project_store())
    steps = [
        "检查防火墙规则是否放行所需的端口范围以及来源地址白名单配置",
        "确认证书申请使用的域名解析记录已经生效并指向目标服务器公网地址",
        "安装服务端组件并写入完整的配置文件监听参数与认证密码字段",
        "启用守护进程单元文件并设置开机自启动同时检查日志输出是否正常",
        "在客户端导入订阅链接之后测量往返延迟抖动以及丢包率是否可接受",
        "若初次握手失败则排查两端时间同步偏差与混淆插件密码一致性问题",
        "处理链路上的最大传输单元分片把数值逐步降低到合适区间反复重试",
        "确认回程路由的三网优化策略是否真正命中各运营商精品线路出口",
        "记录每一次变更的前后状态以便在出现异常时快速回滚到稳定版本",
    ]
    content = "。".join(steps) + "。"  # > 220 chars, diverse vocab -> summary truncates
    finding = Finding("pitfall", 70, "长内容去重回归用例", ["regression"], content)
    write_finding(finding, "test")

    verdict, target = classify_against_store(finding)
    assert verdict == "duplicate"
    assert target is not None


def test_find_project_store_ignores_global_dir(tmp_path, monkeypatch):
    """Regression: cwd==$HOME made find_project_store return the global store
    dir as a 'project', so project-scoped writes polluted the global store."""
    from mnemosyne.store import find_project_store

    home_like = tmp_path / "home"
    (home_like / ".mnemosyne").mkdir(parents=True)
    monkeypatch.setenv("MNEMOSYNE_HOME", str(home_like / ".mnemosyne"))

    assert find_project_store(home_like) is None


def test_heuristic_skips_long_instructional_turn():
    """Regression: step/phase instructions were captured as pitfalls (the
    SSH/Hysteria2 junk). High-precision rule must reject them."""
    junk = Turn(
        role="assistant",
        text=("好，进 **阶段 3：装 Hysteria2 + 自动申请证书**。确保你现在 ssh 在 VPS 里。"
              "第一步检查防火墙，第二步申请证书，万一配错了还有救，原因是顺序错了。" * 2),
    )
    assert HeuristicExtractor(confidence_threshold=0.6).extract([junk]) == []


def test_heuristic_skips_scattered_markers_in_long_turn():
    """A long explanation that merely mentions an error and a cause far apart
    is not a crisp pitfall."""
    far = "错误" + ("。这里是一大段与根因无关的解释说明文字铺垫上下文" * 12) + "原因是配置写错了"
    assert HeuristicExtractor(confidence_threshold=0.6).extract([Turn(role="assistant", text=far)]) == []


def test_heuristic_still_captures_concise_pitfall():
    """The tightening must not regress the genuine case: error and fix stated
    together in a short turn."""
    turn = Turn(role="assistant", text="出现 Traceback，根因是连接池未释放，修复为在 finally 中 close()")
    findings = HeuristicExtractor(confidence_threshold=0.6).extract([turn])
    assert len(findings) == 1 and findings[0].type == "pitfall"
