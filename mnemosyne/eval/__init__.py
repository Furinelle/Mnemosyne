"""Deterministic retrieval evaluation harness."""

from __future__ import annotations

import json
import re
import time
from pathlib import Path
from typing import Callable

from mnemosyne.eval.corpus import EvalItem, load_corpus
from mnemosyne.eval.metrics import latency_percentiles, mrr, recall_at_k
from mnemosyne.search import BM25, SearchDocument
from mnemosyne.tokenizer import tokenize


def legacy_tokenize(text: str) -> list[str]:
    return [match.group(0).lower() for match in re.finditer(r"[\w\u4e00-\u9fff]+", text)]


def run_evaluation(
    corpus_path: Path,
    tokenizer: Callable[[str], list[str]] = tokenize,
) -> dict[str, float]:
    items = load_corpus(corpus_path)
    documents = _load_seed_documents(corpus_path.with_name("seed_memories.jsonl"))
    engine = BM25(documents, tokenizer=tokenizer)
    recalls: list[float] = []
    reciprocal_ranks: list[float] = []
    durations: list[float] = []
    for item in items:
        started = time.perf_counter()
        results = engine.search(item.query, limit=5)
        durations.append((time.perf_counter() - started) * 1000)
        result_ids = [result.document.id for result in results]
        recalls.append(recall_at_k(result_ids, item.expected_ids, 5))
        reciprocal_ranks.append(mrr(result_ids, item.expected_ids))
    latency = latency_percentiles(durations)
    return {
        "count": float(len(items)),
        "recall@5": _mean(recalls),
        "MRR": _mean(reciprocal_ranks),
        **latency,
    }


def format_metrics(backend: str, metrics: dict[str, float]) -> str:
    return (
        f"backend: {backend:<14} "
        f"recall@5={metrics['recall@5']:.3f}  "
        f"MRR={metrics['MRR']:.3f}  "
        f"p50={metrics['p50']:.3f}ms  "
        f"p99={metrics['p99']:.3f}ms  "
        f"queries={int(metrics['count'])}"
    )


def _load_seed_documents(path: Path) -> list[SearchDocument]:
    documents: list[SearchDocument] = []
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            if not line.strip():
                continue
            data = json.loads(line)
            documents.append(SearchDocument(id=str(data["id"]), text=str(data["text"])))
    return documents


def _mean(values: list[float]) -> float:
    return sum(values) / len(values) if values else 0.0


__all__ = [
    "EvalItem",
    "format_metrics",
    "legacy_tokenize",
    "run_evaluation",
]
