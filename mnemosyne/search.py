"""Small stdlib-only BM25 search implementation."""

from __future__ import annotations

import math
import re
from collections import Counter
from dataclasses import dataclass
from typing import Iterable


TOKEN_RE = re.compile(r"[\w\u4e00-\u9fff]+", re.UNICODE)


@dataclass
class SearchDocument:
    id: str
    text: str
    payload: object | None = None


@dataclass
class SearchResult:
    document: SearchDocument
    score: float


def tokenize(text: str) -> list[str]:
    return [match.group(0).lower() for match in TOKEN_RE.finditer(text)]


class BM25:
    def __init__(self, documents: Iterable[SearchDocument], k1: float = 1.5, b: float = 0.75):
        self.documents = list(documents)
        self.k1 = k1
        self.b = b
        self.tokens = [tokenize(document.text) for document in self.documents]
        self.frequencies = [Counter(tokens) for tokens in self.tokens]
        self.lengths = [len(tokens) for tokens in self.tokens]
        self.average_length = sum(self.lengths) / len(self.lengths) if self.lengths else 0.0
        self.document_frequency: Counter[str] = Counter()
        for tokens in self.tokens:
            self.document_frequency.update(set(tokens))

    def search(self, query: str, limit: int = 5) -> list[SearchResult]:
        query_tokens = tokenize(query)
        if not query_tokens or not self.documents:
            return []
        scored: list[SearchResult] = []
        for index, document in enumerate(self.documents):
            score = self._score(query_tokens, index)
            if score > 0:
                scored.append(SearchResult(document=document, score=score))
        scored.sort(key=lambda result: result.score, reverse=True)
        return scored[:limit]

    def _score(self, query_tokens: list[str], index: int) -> float:
        score = 0.0
        frequencies = self.frequencies[index]
        document_length = self.lengths[index]
        total_documents = len(self.documents)
        for token in query_tokens:
            frequency = frequencies.get(token, 0)
            if frequency == 0:
                continue
            documents_with_token = self.document_frequency[token]
            idf = math.log(1 + (total_documents - documents_with_token + 0.5) / (documents_with_token + 0.5))
            denominator = frequency + self.k1 * (
                1 - self.b + self.b * document_length / (self.average_length or 1)
            )
            score += idf * (frequency * (self.k1 + 1)) / denominator
        return score


def memory_search_text(memory: object) -> str:
    parts = [
        getattr(memory, "canonical_summary", ""),
        getattr(memory, "injection_summary", ""),
        getattr(memory, "body", ""),
        " ".join(getattr(memory, "tags", []) or []),
        getattr(memory, "type", ""),
        getattr(memory, "id", ""),
    ]
    return "\n".join(part for part in parts if part)
