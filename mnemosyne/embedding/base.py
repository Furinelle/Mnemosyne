"""Embedder protocol and disabled fallback."""

from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor, TimeoutError as FutureTimeoutError
from typing import Protocol
from typing import Callable, TypeVar


T = TypeVar("T")


class Embedder(Protocol):
    dimensions: int
    model_id: str

    def embed(self, texts: list[str]) -> list[list[float] | None]: ...

    def embed_one(self, text: str) -> list[float] | None: ...


class NoneEmbedder:
    dimensions = 0
    model_id = "none"

    def embed(self, texts: list[str]) -> list[None]:
        return [None] * len(texts)

    def embed_one(self, text: str) -> None:
        return None


def call_with_timeout(function: Callable[[], T], timeout: float, fallback: T) -> T:
    """Run optional backends behind a bounded wait and degrade on failure."""
    executor = ThreadPoolExecutor(max_workers=1)
    future = executor.submit(function)
    try:
        return future.result(timeout=timeout)
    except (FutureTimeoutError, Exception):
        return fallback
    finally:
        executor.shutdown(wait=False, cancel_futures=True)
