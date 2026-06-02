"""Optional local cross-encoder reranker."""

from __future__ import annotations

import sys
from pathlib import Path

from mnemosyne.embedding.onnx import _encode_text


class CrossEncoderReranker:
    def __init__(self, config: dict) -> None:
        self.model_id = str(config.get("model", "BAAI/bge-reranker-base"))
        self.onnx_path_text = str(config.get("onnx_path", "")).strip()
        self.onnx_path = Path(self.onnx_path_text).expanduser()
        self._session = None

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

        if not self.onnx_path_text:
            raise RuntimeError("rerank.onnx_path is not configured")
        self._session = onnxruntime.InferenceSession(str(self.onnx_path), providers=["CPUExecutionProvider"])
        return self._session

    def _run(self, session, query: str, docs: list[str]) -> list[float]:
        import numpy

        encoded = [_encode_text(f"{query} [SEP] {doc}") for doc in docs]
        input_ids = numpy.asarray([item[0] for item in encoded], dtype="int64")
        attention_mask = numpy.asarray([item[1] for item in encoded], dtype="int64")
        token_type_ids = numpy.zeros_like(input_ids)
        available = {item.name for item in session.get_inputs()}
        inputs = {"input_ids": input_ids, "attention_mask": attention_mask}
        if "token_type_ids" in available:
            inputs["token_type_ids"] = token_type_ids
        output = session.run(None, inputs)[0]
        return [float(row[0] if hasattr(row, "__len__") else row) for row in output]
