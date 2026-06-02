from __future__ import annotations

import unittest


class RerankerTests(unittest.TestCase):
    def test_none_reranker_returns_zero_scores(self) -> None:
        from mnemosyne.rerank.base import NoneReranker

        reranker = NoneReranker()

        self.assertEqual([0.0, 0.0], reranker.rerank("query", ["one", "two"]))
        self.assertEqual("none", reranker.model_id)

    def test_factory_defaults_to_none_reranker(self) -> None:
        from mnemosyne.rerank import get_reranker

        self.assertEqual("none", get_reranker({}).model_id)

    def test_factory_rejects_unknown_backend(self) -> None:
        from mnemosyne.rerank import get_reranker

        with self.assertRaisesRegex(ValueError, "unknown rerank backend"):
            get_reranker({"rerank": {"enabled": True, "backend": "mystery"}})


if __name__ == "__main__":
    unittest.main()
