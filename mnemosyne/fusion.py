"""Hybrid retrieval orchestration for BM25, vectors, links, and reranking."""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable

from mnemosyne.embedding import get_embedder
from mnemosyne.embedding.base import call_with_timeout
from mnemosyne.index import cosine_similarity, fts_available, iter_embeddings, search_index
from mnemosyne.relations import warns, weight
from mnemosyne.rerank import get_reranker
from mnemosyne.schema import Memory
from mnemosyne.search import BM25, SearchDocument, memory_search_text
from mnemosyne.store import Store, find_memory, load_config, load_memories


@dataclass
class FusionSearchResult:
    store: Store
    path: Path
    memory: Memory
    score: float
    why_matched: str = ""
    score_breakdown: dict = field(default_factory=dict)

    @property
    def document_id(self) -> str:
        return f"{self.store.scope}:{self.memory.id}"


def rrf(lanes: list[list[str]], k: int = 60) -> dict[str, float]:
    scores: dict[str, float] = {}
    for lane in lanes:
        for rank, document_id in enumerate(lane, start=1):
            scores[document_id] = scores.get(document_id, 0.0) + 1.0 / (k + rank)
    return scores


def search(
    stores: Iterable[Store],
    query: str,
    limit: int = 5,
    type_filter: str = "",
    include_archive: bool = False,
    config: dict | None = None,
    embedder=None,
    reranker=None,
) -> list[FusionSearchResult]:
    stores = list(stores)
    if not query.strip() or limit <= 0:
        return []
    config = config or load_config(stores[-1] if stores else None)
    fusion_config = config.get("fusion", {})
    bm25_pool = max(limit, int(fusion_config.get("bm25_pool_size", limit * 10)))
    vec_pool = max(limit, int(fusion_config.get("vec_pool_size", limit * 10)))
    bm25_results = _bm25_lane(stores, query, bm25_pool, type_filter, include_archive, config)
    candidates = {result.document_id: result for result in bm25_results}

    embedder = embedder or get_embedder(config)
    vector_results: list[FusionSearchResult] = []
    if embedder.model_id != "none":
        vector_results = _vector_lane(stores, query, vec_pool, include_archive, embedder)
        for result in vector_results:
            existing = candidates.get(result.document_id)
            if existing is None:
                candidates[result.document_id] = result
            else:
                existing.score_breakdown.update(result.score_breakdown)

    if vector_results:
        fused_scores = rrf(
            [
                [result.document_id for result in bm25_results],
                [result.document_id for result in vector_results],
            ],
            k=int(fusion_config.get("rrf_k", 60)),
        )
        for document_id, score in fused_scores.items():
            candidates[document_id].score = score

    if fusion_config.get("link_expansion", True):
        candidates = expand_links(candidates, stores, fusion_config)

    results = _sorted_results(candidates.values())
    reranker = reranker or get_reranker(config)
    if reranker.model_id != "none":
        results = _rerank(query, results, reranker, int(config.get("rerank", {}).get("top_n", 5)))
    return results[:limit]


def expand_links(
    candidates: dict[str, FusionSearchResult],
    stores: list[Store],
    fusion_config: dict,
) -> dict[str, FusionSearchResult]:
    if not fusion_config.get("link_expansion", True):
        return candidates
    overrides = fusion_config.get("relation_weight_override", {})
    decay = float(fusion_config.get("link_expansion_decay_fallback", 0.5))
    max_hops = max(0, int(fusion_config.get("link_expansion_max_hops", 1)))
    frontier = list(candidates.values())
    visited_sources: set[str] = set()
    for hop in range(max_hops):
        next_frontier: list[FusionSearchResult] = []
        for source in frontier:
            if source.document_id in visited_sources:
                continue
            visited_sources.add(source.document_id)
            for link in source.memory.links:
                target_id = str(link.get("id", ""))
                relation = str(link.get("rel", ""))
                found = find_memory(target_id, stores, include_archive=True)
                if found is None:
                    continue
                target_store, target_path, target_memory = found
                document_id = f"{target_store.scope}:{target_memory.id}"
                boost = source.score * weight(relation, overrides) * (decay ** hop)
                target = candidates.get(document_id)
                if target is None:
                    target = FusionSearchResult(
                        store=target_store,
                        path=target_path,
                        memory=target_memory,
                        score=boost,
                        score_breakdown={"link_boost": boost},
                    )
                    candidates[document_id] = target
                    next_frontier.append(target)
                else:
                    target.score += boost
                    target.score_breakdown["link_boost"] = target.score_breakdown.get("link_boost", 0.0) + boost
                if warns(relation):
                    contradictions = target.score_breakdown.setdefault("contradicts_with", [])
                    if source.memory.id not in contradictions:
                        contradictions.append(source.memory.id)
        frontier = next_frontier
    return candidates


def _bm25_lane(
    stores: list[Store],
    query: str,
    limit: int,
    type_filter: str,
    include_archive: bool,
    config: dict,
) -> list[FusionSearchResult]:
    if config.get("search", {}).get("index_enabled", True) and fts_available():
        indexed = search_index(
            stores,
            query,
            limit=limit,
            memory_type=type_filter,
            include_archive=include_archive,
        )
        if indexed:
            return [
                FusionSearchResult(
                    store=result.store,
                    path=result.path,
                    memory=result.memory,
                    score=result.score,
                    why_matched=result.why_matched,
                    score_breakdown={"bm25": result.score},
                )
                for result in indexed
            ]

    documents: list[SearchDocument] = []
    lookup: dict[str, tuple[Store, Path, Memory]] = {}
    for store in stores:
        for path, memory in load_memories(store, include_archive=include_archive):
            if type_filter and memory.type != type_filter:
                continue
            document_id = f"{store.scope}:{memory.id}"
            documents.append(SearchDocument(document_id, memory_search_text(memory), memory))
            lookup[document_id] = (store, path, memory)
    return [
        FusionSearchResult(
            store=lookup[result.document.id][0],
            path=lookup[result.document.id][1],
            memory=lookup[result.document.id][2],
            score=result.score,
            score_breakdown={"bm25": result.score},
        )
        for result in BM25(documents).search(query, limit)
    ]


def _vector_lane(
    stores: list[Store],
    query: str,
    limit: int,
    include_archive: bool,
    embedder,
) -> list[FusionSearchResult]:
    vector = call_with_timeout(lambda: embedder.embed_one(query), timeout=10.0, fallback=None)
    if vector is None:
        return []
    results: list[FusionSearchResult] = []
    for indexed in iter_embeddings(stores, embedder.model_id, embedder.dimensions, include_archive):
        score = cosine_similarity(vector, indexed.vector)
        if score <= 0:
            continue
        results.append(
            FusionSearchResult(
                store=indexed.store,
                path=indexed.path,
                memory=indexed.memory,
                score=score,
                score_breakdown={"vec": score},
            )
        )
    return _sorted_results(results)[:limit]


def _rerank(query: str, results: list[FusionSearchResult], reranker, top_n: int) -> list[FusionSearchResult]:
    count = max(0, top_n * 2)
    selected = results[:count]
    if not selected:
        return results
    docs = [memory_search_text(result.memory) for result in selected]
    scores = call_with_timeout(lambda: reranker.rerank(query, docs), timeout=10.0, fallback=[])
    if len(scores) != len(selected) or not any(scores):
        return results
    for result, score in zip(selected, scores):
        result.score = float(score)
        result.score_breakdown["rerank"] = float(score)
    return _sorted_results(results)


def _sorted_results(results: Iterable[FusionSearchResult]) -> list[FusionSearchResult]:
    return sorted(results, key=lambda result: (result.score, result.document_id), reverse=True)

