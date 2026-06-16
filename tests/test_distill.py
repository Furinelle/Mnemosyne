from mnemosyne.distill import Turn, parse_claude_transcript


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
