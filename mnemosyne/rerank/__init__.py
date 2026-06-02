"""Reranker factory."""

from __future__ import annotations

from mnemosyne.rerank.base import NoneReranker, Reranker


def get_reranker(config: dict) -> Reranker:
    settings = config.get("rerank", {})
    if not settings.get("enabled"):
        return NoneReranker()
    backend = settings.get("backend", "cross_encoder")
    if backend == "none":
        return NoneReranker()
    if backend == "cross_encoder":
        from mnemosyne.rerank.cross_encoder import CrossEncoderReranker

        return CrossEncoderReranker(settings)
    raise ValueError(f"unknown rerank backend: {backend}")


__all__ = ["NoneReranker", "Reranker", "get_reranker"]
