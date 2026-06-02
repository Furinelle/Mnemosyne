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

    def test_cross_encoder_pair_encoding_marks_document_segment(self) -> None:
        from mnemosyne.rerank.cross_encoder import _encode_pair

        vocab = {
            "[PAD]": 0,
            "[UNK]": 1,
            "[CLS]": 2,
            "[SEP]": 3,
            "query": 4,
            "doc": 5,
        }

        input_ids, attention, token_types = _encode_pair("query", "doc", vocab, max_length=7)

        self.assertEqual([2, 4, 3, 5, 3, 0, 0], input_ids)
        self.assertEqual([1, 1, 1, 1, 1, 0, 0], attention)
        self.assertEqual([0, 0, 0, 1, 1, 0, 0], token_types)


if __name__ == "__main__":
    unittest.main()
