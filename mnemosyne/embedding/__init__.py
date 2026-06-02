"""Embedder factory."""

from __future__ import annotations

from mnemosyne.embedding.base import Embedder, NoneEmbedder


def get_embedder(config: dict) -> Embedder:
    settings = config.get("embedding", {})
    if not settings.get("enabled"):
        return NoneEmbedder()
    backend = settings.get("backend", "onnx")
    if backend == "onnx":
        from mnemosyne.embedding.onnx import LocalONNXEmbedder

        return LocalONNXEmbedder(settings)
    if backend == "openai":
        from mnemosyne.embedding.openai import OpenAICompatEmbedder

        return OpenAICompatEmbedder(settings)
    raise ValueError(f"unknown embedding backend: {backend}")


__all__ = ["Embedder", "NoneEmbedder", "get_embedder"]
