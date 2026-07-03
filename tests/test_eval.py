from __future__ import annotations

import io
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

from mnemosyne.cli import main
from mnemosyne.eval import legacy_tokenize, run_evaluation
from mnemosyne.eval.corpus import EvalItem, load_corpus, save_corpus
from mnemosyne.eval.metrics import latency_percentiles, mrr, recall_at_k


class EvalTests(unittest.TestCase):
    def test_metrics(self) -> None:
        self.assertEqual(0.5, recall_at_k(["a", "x"], ["a", "b"], 2))
        self.assertEqual(0.5, mrr(["x", "a"], ["a"]))
        self.assertEqual({"p50": 2.0, "p99": 4.0}, latency_percentiles([4.0, 1.0, 3.0, 2.0]))

    def test_corpus_round_trip(self) -> None:
        items = [EvalItem(query="q", expected_ids=["a"], paraphrase_of="a", notes="note")]
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "corpus.jsonl"
            save_corpus(path, items)
            self.assertEqual(items, load_corpus(path))

    def test_default_corpus_has_fifty_items_and_bigram_improves_recall(self) -> None:
        corpus = Path(__file__).resolve().parents[1] / "mnemosyne" / "eval" / "default_corpus.jsonl"

        legacy = run_evaluation(corpus, tokenizer=legacy_tokenize)
        current = run_evaluation(corpus)

        self.assertEqual(50, int(current["count"]))
        self.assertGreater(current["recall@5"], legacy["recall@5"])

    def test_eval_run_cli_prints_baseline_metrics(self) -> None:
        corpus = Path(__file__).resolve().parents[1] / "mnemosyne" / "eval" / "default_corpus.jsonl"
        output = io.StringIO()

        with redirect_stdout(output):
            code = main(["eval", "run", "--corpus", str(corpus)])

        self.assertEqual(0, code)
        self.assertIn("backend: bm25-only", output.getvalue())
        self.assertIn("bigram recall@5 delta=+", output.getvalue())


if __name__ == "__main__":
    unittest.main()


def test_eval_run_min_recall_gate():
    from mnemosyne.eval.__main__ import main as eval_main

    assert eval_main(["run", "--min-recall", "1.01"]) == 1
    assert eval_main(["run", "--min-recall", "0.5"]) == 0
