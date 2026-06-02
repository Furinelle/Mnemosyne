"""Local ONNX embedding backend with lazy optional imports."""

from __future__ import annotations

import math
import re
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
        self.vocab_path = self.onnx_path.with_name("vocab.txt")
        self._session = None
        self._vocab: dict[str, int] | None = None

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
        if not self.vocab_path.exists():
            self._download_vocab()
        self._session = onnxruntime.InferenceSession(str(self.onnx_path), providers=["CPUExecutionProvider"])
        return self._session

    def _download_model(self) -> None:
        url = f"https://huggingface.co/{self.model_id}/resolve/main/onnx/model.onnx"
        _download_file(url, self.onnx_path)

    def _download_vocab(self) -> None:
        url = f"https://huggingface.co/{self.model_id}/resolve/main/vocab.txt"
        _download_file(url, self.vocab_path)

    def _run_batch(self, session, texts: list[str]) -> list[list[float]]:
        import numpy

        vocab = self._load_vocab()
        encoded = [_encode_text(text, vocab) for text in texts]
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

    def _load_vocab(self) -> dict[str, int]:
        if self._vocab is None:
            if not self.vocab_path.exists():
                self._download_vocab()
            self._vocab = _load_vocab(self.vocab_path)
        return self._vocab


def _default_model_path(model_id: str) -> Path:
    safe_model = model_id.replace("/", "--")
    return Path("~/.cache/mnemosyne/models").expanduser() / safe_model / "model.onnx"


def _encode_text(text: str, vocab: dict[str, int], max_length: int = 64) -> tuple[list[int], list[int]]:
    token_ids = [vocab.get("[CLS]", 101)]
    token_ids.extend(_text_token_ids(text, vocab)[: max_length - 2])
    token_ids.append(vocab.get("[SEP]", 102))
    attention = [1] * len(token_ids)
    padding = max_length - len(token_ids)
    return token_ids + [vocab.get("[PAD]", 0)] * padding, attention + [0] * padding


def _text_token_ids(text: str, vocab: dict[str, int]) -> list[int]:
    tokens: list[str] = []
    for token in re.findall(r"[\u4e00-\u9fff]|[a-z0-9_]+|[^\w\s]", text.lower()):
        tokens.extend(_wordpiece(token, vocab))
    return [vocab.get(token, vocab.get("[UNK]", 100)) for token in tokens]


def _wordpiece(token: str, vocab: dict[str, int]) -> list[str]:
    if token in vocab:
        return [token]
    pieces: list[str] = []
    start = 0
    while start < len(token):
        end = len(token)
        matched = ""
        while end > start:
            candidate = token[start:end]
            if start:
                candidate = f"##{candidate}"
            if candidate in vocab:
                matched = candidate
                break
            end -= 1
        if not matched:
            return ["[UNK]"]
        pieces.append(matched)
        start = end
    return pieces


def _load_vocab(path: Path) -> dict[str, int]:
    return {
        token.rstrip("\n"): index
        for index, token in enumerate(path.read_text(encoding="utf-8").splitlines())
    }


def _download_file(url: str, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with urlopen(url, timeout=10.0) as response:
        path.write_bytes(response.read())


def _normalize(vector: list[float]) -> list[float]:
    norm = math.sqrt(sum(value * value for value in vector))
    return [value / norm for value in vector] if norm else vector
