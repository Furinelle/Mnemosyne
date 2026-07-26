"""Optional local cross-encoder reranker."""

from __future__ import annotations

import sys
from pathlib import Path

from mnemosyne.embedding.onnx import _default_model_path, _download_file, _load_vocab, _text_token_ids


class CrossEncoderReranker:
    def __init__(self, config: dict) -> None:
        self.model_id = str(config.get("model", "BAAI/bge-reranker-base"))
        configured_path = str(config.get("onnx_path", "")).strip()
        self.onnx_path = Path(configured_path).expanduser() if configured_path else _default_model_path(self.model_id)
        self.vocab_path = self.onnx_path.with_name("vocab.txt")
        self._session = None
        self._vocab: dict[str, int] | None = None

    def rerank(self, query: str, docs: list[str]) -> list[float]:
        if not docs:
            return []
        try:
            session = self._load_session()
            return self._run(session, query, docs)
        except Exception as exc:
            print(f"mnemosyne: cross-encoder reranker unavailable: {exc}", file=sys.stderr)
            return [0.0] * len(docs)

    def _load_session(self):
        if self._session is not None:
            return self._session
        import onnxruntime

        if not self.onnx_path.exists():
            _download_file(f"https://huggingface.co/{self.model_id}/resolve/main/onnx/model.onnx", self.onnx_path)
        if not self.vocab_path.exists():
            _download_file(f"https://huggingface.co/{self.model_id}/resolve/main/vocab.txt", self.vocab_path)
        self._session = onnxruntime.InferenceSession(str(self.onnx_path), providers=["CPUExecutionProvider"])
        return self._session

    def _run(self, session, query: str, docs: list[str]) -> list[float]:
        import numpy

        encoded = [_encode_pair(query, doc, self._load_vocab()) for doc in docs]
        input_ids = numpy.asarray([item[0] for item in encoded], dtype="int64")
        attention_mask = numpy.asarray([item[1] for item in encoded], dtype="int64")
        token_type_ids = numpy.asarray([item[2] for item in encoded], dtype="int64")
        available = {item.name for item in session.get_inputs()}
        inputs = {"input_ids": input_ids, "attention_mask": attention_mask}
        if "token_type_ids" in available:
            inputs["token_type_ids"] = token_type_ids
        output = session.run(None, inputs)[0]
        return [float(row[0] if hasattr(row, "__len__") else row) for row in output]

    def _load_vocab(self) -> dict[str, int]:
        if self._vocab is None:
            self._vocab = _load_vocab(self.vocab_path)
        return self._vocab


def _encode_pair(
    query: str,
    doc: str,
    vocab: dict[str, int],
    max_length: int = 256,
    query_limit: int = 64,
) -> tuple[list[int], list[int], list[int]]:
    cls_id = vocab.get("[CLS]", 101)
    sep_id = vocab.get("[SEP]", 102)
    pad_id = vocab.get("[PAD]", 0)
    query_ids = _text_token_ids(query, vocab)
    doc_ids = _text_token_ids(doc, vocab)
    content_limit = max(0, max_length - 3)
    # Cap the query separately so a long question cannot starve the document:
    # with a shared budget the doc was left with a few tokens of summary and the
    # reranker scored noise.
    query_ids = query_ids[: min(content_limit, query_limit)]
    doc_ids = doc_ids[: max(0, content_limit - len(query_ids))]
    input_ids = [cls_id, *query_ids, sep_id, *doc_ids, sep_id]
    token_types = [0] * (len(query_ids) + 2) + [1] * (len(doc_ids) + 1)
    attention = [1] * len(input_ids)
    padding = max_length - len(input_ids)
    return (
        input_ids + [pad_id] * padding,
        attention + [0] * padding,
        token_types + [0] * padding,
    )
