from mnemosyne.eval.corpus import EvalItem, load_corpus, save_corpus


def test_evalitem_backward_compatible_defaults():
    item = EvalItem(query="q", expected_ids=["a"], paraphrase_of="")
    assert item.instance_id == ""
    assert item.question_type == ""


def test_corpus_roundtrip_with_new_fields(tmp_path):
    path = tmp_path / "c.jsonl"
    items = [EvalItem("q", ["a"], "", "n", instance_id="i1", question_type="multi-session")]
    save_corpus(path, items)
    loaded = load_corpus(path)
    assert loaded[0].instance_id == "i1"
    assert loaded[0].question_type == "multi-session"


def test_corpus_loads_legacy_lines_without_new_fields(tmp_path):
    path = tmp_path / "legacy.jsonl"
    path.write_text(
        '{"query":"q","expected_ids":["a"],"paraphrase_of":"","notes":""}\n',
        encoding="utf-8",
    )
    loaded = load_corpus(path)
    assert loaded[0].instance_id == ""
