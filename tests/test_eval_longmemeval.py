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


from pathlib import Path

from mnemosyne.eval.adapters.longmemeval import convert

FIXTURE = Path("mnemosyne/eval/fixtures/longmemeval_sample.json")


def test_convert_produces_seed_and_corpus(tmp_path):
    convert(FIXTURE, tmp_path)
    corpus = load_corpus(tmp_path / "corpus.jsonl")
    assert len(corpus) == 2
    q1 = next(i for i in corpus if i.instance_id == "q1")
    assert q1.expected_ids == ["lme-q1-s1"]
    assert q1.question_type == "single-session-user"
    seed_lines = (tmp_path / "seed_memories.jsonl").read_text(encoding="utf-8").strip().splitlines()
    assert len(seed_lines) == 4  # 2 instances x 2 sessions
    import json

    ids = {json.loads(line)["id"] for line in seed_lines}
    assert "lme-q1-s1" in ids and "lme-q2-s4" in ids


def test_convert_respects_max_instances(tmp_path):
    convert(FIXTURE, tmp_path, max_instances=1)
    assert len(load_corpus(tmp_path / "corpus.jsonl")) == 1


from mnemosyne.eval import run_longmemeval


def test_run_longmemeval_isolates_instances(tmp_path):
    convert(FIXTURE, tmp_path)
    report = run_longmemeval(tmp_path / "corpus.jsonl")
    # s1 holds the ruff answer for q1; isolation means q1's query only sees q1 docs.
    assert report["recall@5"] == 1.0
    assert "by_type" in report
    assert "single-session-user" in report["by_type"]
    assert report["by_type"]["single-session-user"]["recall@5"] == 1.0


def test_run_longmemeval_reports_multiple_k(tmp_path):
    convert(FIXTURE, tmp_path)
    report = run_longmemeval(tmp_path / "corpus.jsonl")
    for key in ("recall@1", "recall@5", "recall@10", "MRR"):
        assert key in report
