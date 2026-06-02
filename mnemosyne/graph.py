"""Breadth-first traversal and rendering for linked memories."""

from __future__ import annotations

import json
import re
from collections import deque

from mnemosyne.store import Store, find_memory


def build_graph(memory_id: str, stores: list[Store], depth: int = 1) -> dict:
    found = find_memory(memory_id, stores, include_archive=True)
    if found is None:
        raise ValueError(f"Memory not found: {memory_id}")
    root_store, _root_path, root_memory = found
    nodes: list[dict] = [_node(root_store, root_memory)]
    edges: list[dict] = []
    seen_nodes = {f"{root_store.scope}:{root_memory.id}"}
    seen_edges: set[tuple[str, str, str]] = set()
    queue = deque([(root_store, root_memory, 0)])
    while queue:
        _store, memory, distance = queue.popleft()
        if distance >= max(0, depth):
            continue
        for link in memory.links:
            target_id = str(link.get("id", ""))
            relation = str(link.get("rel", "related"))
            target = find_memory(target_id, stores, include_archive=True)
            if target is None:
                continue
            target_store, _target_path, target_memory = target
            edge_key = (memory.id, target_memory.id, relation)
            if edge_key not in seen_edges:
                edges.append({"source": memory.id, "target": target_memory.id, "rel": relation})
                seen_edges.add(edge_key)
            document_id = f"{target_store.scope}:{target_memory.id}"
            if document_id in seen_nodes:
                continue
            seen_nodes.add(document_id)
            nodes.append(_node(target_store, target_memory))
            queue.append((target_store, target_memory, distance + 1))
    return {"root": memory_id, "nodes": nodes, "edges": edges}


def render_graph(graph: dict, output_format: str) -> str:
    if output_format == "mermaid":
        return render_mermaid(graph)
    if output_format == "ascii":
        return render_ascii(graph)
    if output_format == "json":
        return json.dumps(graph, ensure_ascii=False, indent=2)
    raise ValueError(f"unknown graph format: {output_format}")


def render_mermaid(graph: dict) -> str:
    nodes = {node["id"]: node for node in graph["nodes"]}
    lines = ["graph LR"]
    rendered: set[str] = set()
    for edge in graph["edges"]:
        source = _mermaid_node(nodes[edge["source"]])
        target = _mermaid_node(nodes[edge["target"]])
        lines.append(f"  {source} -- {_escape(edge['rel'])} --> {target}")
        rendered.update((edge["source"], edge["target"]))
    for node in graph["nodes"]:
        if node["id"] not in rendered:
            lines.append(f"  {_mermaid_node(node)}")
    return "\n".join(lines)


def render_ascii(graph: dict) -> str:
    adjacency: dict[str, list[dict]] = {}
    for edge in graph["edges"]:
        adjacency.setdefault(edge["source"], []).append(edge)
    lines = [str(graph["root"])]
    visited = {str(graph["root"])}

    def append_children(memory_id: str, indent: str) -> None:
        for edge in adjacency.get(memory_id, []):
            target = str(edge["target"])
            suffix = " (cycle)" if target in visited else ""
            lines.append(f"{indent}-> [{edge['rel']}] {target}{suffix}")
            if target not in visited:
                visited.add(target)
                append_children(target, indent + "  ")

    append_children(str(graph["root"]), "  ")
    return "\n".join(lines)


def _node(store: Store, memory) -> dict:
    return {
        "id": memory.id,
        "title": memory.title,
        "type": memory.type,
        "scope": store.scope,
    }


def _mermaid_node(node: dict) -> str:
    return f'{_identifier(node["id"])}["{_escape(node["title"])}"]'


def _identifier(memory_id: str) -> str:
    identifier = re.sub(r"[^a-zA-Z0-9_]", "_", memory_id)
    return identifier if identifier and not identifier[0].isdigit() else f"memory_{identifier}"


def _escape(text: str) -> str:
    return str(text).replace('"', '\\"').replace("\n", " ")
