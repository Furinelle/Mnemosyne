from __future__ import annotations

import os
import sys
import time
import types
import unittest
from unittest.mock import patch


class EmbeddingTests(unittest.TestCase):
    def test_none_embedder_returns_none_vectors(self) -> None:
        from mnemosyne.embedding.base import NoneEmbedder

        embedder = NoneEmbedder()

        self.assertEqual([None, None], embedder.embed(["one", "two"]))
        self.assertIsNone(embedder.embed_one("one"))
        self.assertEqual("none", embedder.model_id)

    def test_factory_defaults_to_none_embedder(self) -> None:
        from mnemosyne.embedding import get_embedder

        self.assertEqual("none", get_embedder({}).model_id)

    def test_factory_rejects_unknown_backend(self) -> None:
        from mnemosyne.embedding import get_embedder

        with self.assertRaisesRegex(ValueError, "unknown embedding backend"):
            get_embedder({"embedding": {"enabled": True, "backend": "mystery"}})

    def test_openai_compat_embedder_retries_once_with_timeout(self) -> None:
        from mnemosyne.embedding.openai import OpenAICompatEmbedder

        calls: list[dict] = []

        class Response:
            def raise_for_status(self) -> None:
                return None

            def json(self) -> dict:
                return {"data": [{"embedding": [0.1, 0.2]}]}

        class Client:
            def __init__(self, **kwargs) -> None:
                calls.append({"init": kwargs})

            def __enter__(self):
                return self

            def __exit__(self, *_args) -> None:
                return None

            def post(self, url: str, **kwargs):
                calls.append({"url": url, **kwargs})
                if len([call for call in calls if "url" in call]) == 1:
                    raise RuntimeError("temporary")
                return Response()

        fake_httpx = types.SimpleNamespace(Client=Client)
        with patch.dict(sys.modules, {"httpx": fake_httpx}), patch.dict(os.environ, {"TEST_EMBED_KEY": "secret"}):
            embedder = OpenAICompatEmbedder(
                {
                    "api_base": "https://example.test/v1/",
                    "api_key_env": "TEST_EMBED_KEY",
                    "model": "embed-test",
                    "dimensions": 2,
                }
            )
            vector = embedder.embed_one("hello")

        self.assertEqual([0.1, 0.2], vector)
        posts = [call for call in calls if "url" in call]
        self.assertEqual(2, len(posts))
        self.assertEqual("https://example.test/v1/embeddings", posts[0]["url"])
        self.assertEqual(10.0, calls[0]["init"]["timeout"])

    def test_wordpiece_encoder_uses_vocab_ids_and_padding(self) -> None:
        from mnemosyne.embedding.onnx import _encode_text

        vocab = {"[PAD]": 0, "[UNK]": 1, "[CLS]": 2, "[SEP]": 3, "认": 4, "证": 5, "api": 6}

        input_ids, attention = _encode_text("认证 api", vocab, max_length=8)

        self.assertEqual([2, 4, 5, 6, 3, 0, 0, 0], input_ids)
        self.assertEqual([1, 1, 1, 1, 1, 0, 0, 0], attention)

    def test_wordpiece_encoder_uses_unknown_token(self) -> None:
        from mnemosyne.embedding.onnx import _encode_text

        vocab = {"[PAD]": 0, "[UNK]": 1, "[CLS]": 2, "[SEP]": 3}

        input_ids, _attention = _encode_text("missing", vocab, max_length=4)

        self.assertEqual([2, 1, 3, 0], input_ids)

    def test_optional_backend_timeout_returns_fallback(self) -> None:
        from mnemosyne.embedding.base import call_with_timeout

        result = call_with_timeout(lambda: time.sleep(0.02), timeout=0.001, fallback="fallback")

        self.assertEqual("fallback", result)


if __name__ == "__main__":
    unittest.main()
