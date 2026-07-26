import json

from mnemosyne.transcripts import Turn, detect_format, parse_transcript


def test_role_jsonl(tmp_path):
    p = tmp_path / "t.jsonl"
    p.write_text('\n'.join(json.dumps(x, ensure_ascii=False) for x in [
        {"role": "user", "text": "如何修复认证失败"},
        {"role": "assistant", "text": "根因是 token 过期"},
        {"role": "system", "text": "ignored"},
    ]), encoding="utf-8")
    turns = parse_transcript(p)
    assert [t.role for t in turns] == ["user", "assistant"]
    assert turns[0].text == "如何修复认证失败"


def test_claude_jsonl(tmp_path):
    p = tmp_path / "c.jsonl"
    p.write_text(json.dumps({"message": {"role": "user", "content": "hello there"}}) + "\n", encoding="utf-8")
    turns = parse_transcript(p)
    assert turns == [Turn(role="user", text="hello there")]


def test_plain_text(tmp_path):
    p = tmp_path / "t.txt"
    p.write_text("[user] hello\n[assistant] hi there", encoding="utf-8")
    turns = parse_transcript(p)
    assert [t.role for t in turns] == ["user", "assistant"]


def test_detect_claude_jsonl():
    line = json.dumps({"message": {"role": "user", "content": "hi"}})
    assert detect_format(line) == "claude-jsonl"


def test_detect_role_jsonl():
    line = json.dumps({"role": "user", "text": "hi"})
    assert detect_format(line) == "role-jsonl"


def test_detect_plain_text():
    assert detect_format("[user] hello\n[assistant] hi") == "text"


def test_explicit_format_overrides_detection(tmp_path):
    p = tmp_path / "t.jsonl"
    p.write_text(json.dumps({"role": "user", "text": "hi"}), encoding="utf-8")
    assert parse_transcript(p, fmt="text")  # parsed as raw text, still non-empty


def test_distill_source_default_neutral():
    from mnemosyne.cli import build_parser
    args = build_parser().parse_args(["distill", "--stdin"])
    assert args.source == "agent"


def test_distill_transcript_format_flag():
    from mnemosyne.cli import build_parser
    args = build_parser().parse_args(["distill", "--transcript", "x.jsonl", "--format", "role-jsonl"])
    assert args.fmt == "role-jsonl"


def test_legacy_parse_claude_transcript_reexport(tmp_path):
    from mnemosyne.distill import Turn as LegacyTurn, parse_claude_transcript
    p = tmp_path / "c.jsonl"
    p.write_text(json.dumps({"message": {"role": "assistant", "content": "done"}}), encoding="utf-8")
    assert parse_claude_transcript(p) == [LegacyTurn(role="assistant", text="done")]
