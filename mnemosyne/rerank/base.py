"""Reranker protocol and disabled fallback."""

from __future__ import annotations

from typing import Protocol


class Reranker(Protocol):
    model_id: str

    def rerank(self, query: str, docs: list[str]) -> list[float]: ...


class NoneReranker:
    model_id = "none"

    def rerank(self, query: str, docs: list[str]) -> list[float]:
        return [0.0] * len(docs)
