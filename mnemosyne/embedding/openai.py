"""OpenAI-compatible HTTP embedding backend."""

from __future__ import annotations

import os
import sys


class OpenAICompatEmbedder:
    def __init__(self, config: dict) -> None:
        self.api_base = str(config.get("api_base", "https://api.openai.com/v1")).rstrip("/")
        self.api_key_env = str(config.get("api_key_env", "OPENAI_API_KEY"))
        self.model_id = str(config.get("model", "text-embedding-3-small"))
        self.dimensions = int(config.get("dimensions", 384))

    def embed(self, texts: list[str]) -> list[list[float] | None]:
        if not texts:
            return []
        try:
            return self._request(texts)
        except Exception as exc:
            print(f"mnemosyne: embedding backend unavailable: {exc}", file=sys.stderr)
            return [None] * len(texts)

    def embed_one(self, text: str) -> list[float] | None:
        vectors = self.embed([text])
        return vectors[0] if vectors else None

    def _request(self, texts: list[str]) -> list[list[float]]:
        import httpx

        api_key = os.environ.get(self.api_key_env, "")
        if not api_key:
            raise RuntimeError(f"environment variable {self.api_key_env} is not set")
        headers = {"Authorization": f"Bearer {api_key}"}
        body = {"model": self.model_id, "input": texts}
        last_error: Exception | None = None
        for _attempt in range(2):
            try:
                with httpx.Client(timeout=10.0) as client:
                    response = client.post(f"{self.api_base}/embeddings", headers=headers, json=body)
                    response.raise_for_status()
                    data = response.json().get("data", [])
                    return [list(map(float, item["embedding"])) for item in data]
            except Exception as exc:
                last_error = exc
        assert last_error is not None
        raise last_error
