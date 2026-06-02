"""Local ONNX embedding backend with lazy optional imports."""

from __future__ import annotations

import math
import sys
from pathlib import Path
from urllib.request import urlopen


class LocalONNXEmbedder:
    def __init__(self, config: dict) -> None:
        self.model_id = str(config.get("model", "BAAI/bge-small-zh-v1.5"))
        self.dimensions = int(config.get("dimensions", 384))
        self.batch_size = int(config.get("batch_size", 32))
        configured_path = str(config.get("onnx_path", "")).strip()
        self.onnx_path = Path(configured_path).expanduser() if configured_path else _default_model_path(self.model_id)
        self._session = None

    def embed(self, texts: list[str]) -> list[list[float] | None]:
        if not texts:
            return []
        try:
            session = self._load_session()
            vectors: list[list[float]] = []
            for start in range(0, len(texts), self.batch_size):
                vectors.extend(self._run_batch(session, texts[start : start + self.batch_size]))
            return vectors
        except Exception as exc:
            print(f"mnemosyne: local ONNX embedder unavailable: {exc}", file=sys.stderr)
            return [None] * len(texts)

    def embed_one(self, text: str) -> list[float] | None:
        vectors = self.embed([text])
        return vectors[0] if vectors else None

    def _load_session(self):
        if self._session is not None:
            return self._session
        import onnxruntime

        if not self.onnx_path.exists():
            self._download_model()
        self._session = onnxruntime.InferenceSession(str(self.onnx_path), providers=["CPUExecutionProvider"])
        return self._session

    def _download_model(self) -> None:
        self.onnx_path.parent.mkdir(parents=True, exist_ok=True)
        url = f"https://huggingface.co/{self.model_id}/resolve/main/onnx/model.onnx"
        with urlopen(url, timeout=10.0) as response:
            self.onnx_path.write_bytes(response.read())

    def _run_batch(self, session, texts: list[str]) -> list[list[float]]:
        import numpy

        encoded = [_encode_text(text) for text in texts]
        input_ids = numpy.asarray([item[0] for item in encoded], dtype="int64")
        attention_mask = numpy.asarray([item[1] for item in encoded], dtype="int64")
        token_type_ids = numpy.zeros_like(input_ids)
        available = {item.name for item in session.get_inputs()}
        inputs = {"input_ids": input_ids, "attention_mask": attention_mask}
        if "token_type_ids" in available:
            inputs["token_type_ids"] = token_type_ids
        output = session.run(None, inputs)[0]
        if getattr(output, "ndim", 0) == 3:
            output = output[:, 0, :]
        return [_normalize([float(value) for value in row]) for row in output]


def _default_model_path(model_id: str) -> Path:
    safe_model = model_id.replace("/", "--")
    return Path("~/.cache/mnemosyne/models").expanduser() / safe_model / "model.onnx"


def _encode_text(text: str, max_length: int = 64) -> tuple[list[int], list[int]]:
    # This stdlib tokenizer keeps the optional backend dependency-light. Users
    # can point onnx_path at a compatible exported model without affecting the
    # base installation.
    token_ids = [101]
    token_ids.extend(100 + (ord(character) % 30000) for character in text[: max_length - 2])
    token_ids.append(102)
    attention = [1] * len(token_ids)
    padding = max_length - len(token_ids)
    return token_ids + [0] * padding, attention + [0] * padding


def _normalize(vector: list[float]) -> list[float]:
    norm = math.sqrt(sum(value * value for value in vector))
    return [value / norm for value in vector] if norm else vector
