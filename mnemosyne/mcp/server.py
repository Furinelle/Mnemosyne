"""MCP-compatible JSON-RPC server with stdio and optional SSE transports."""

from __future__ import annotations

import json
import queue
import sys
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

from mnemosyne import __version__, api
from mnemosyne.codex import prep
from mnemosyne.fusion import search
from mnemosyne.graph import build_graph, render_graph
from mnemosyne.schema import serialize_memory
from mnemosyne.store import find_memory, load_config, read_core, stores_for_scope


class MissingMCPDependency(RuntimeError):
    """Kept for import compatibility; the stdlib server no longer raises it."""


SERVER_VERSION = __version__


READ_ONLY = {"readOnlyHint": True}
WRITES = {"readOnlyHint": False, "destructiveHint": False, "idempotentHint": False}

TOOL_SCHEMAS = [
    {
        "name": "mnemosyne_search",
        "description": "Search durable memories.",
        "annotations": READ_ONLY,
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "limit": {"type": "integer", "default": 5},
                "type": {"type": "string", "default": ""},
                "scope": {"type": "string", "enum": ["global", "project", "all"], "default": "all"},
                "include_archive": {"type": "boolean", "default": False},
            },
            "required": ["query"],
        },
    },
    {
        "name": "mnemosyne_write",
        "description": "Write one durable memory.",
        "annotations": WRITES,
        "inputSchema": {
            "type": "object",
            "properties": {
                "type": {"type": "string"},
                "importance": {"type": "integer"},
                "title": {"type": "string", "default": ""},
                "content": {"type": "string"},
                "tags": {"type": "string", "default": ""},
                "scope": {"type": "string", "enum": ["global", "project"], "default": "project"},
                "source": {"type": "string", "default": "mcp"},
            },
            "required": ["type", "importance", "content"],
        },
    },
    {
        "name": "mnemosyne_read_core",
        "description": "Read core memory markdown.",
        "annotations": READ_ONLY,
        "inputSchema": {
            "type": "object",
            "properties": {"scope": {"type": "string", "enum": ["global", "project", "all"], "default": "all"}},
        },
    },
    {
        "name": "mnemosyne_show",
        "description": "Show a memory by ID.",
        "annotations": READ_ONLY,
        "inputSchema": {
            "type": "object",
            "properties": {"id": {"type": "string"}},
            "required": ["id"],
        },
    },
    {
        "name": "mnemosyne_link",
        "description": "Link two memories with a typed relation.",
        "annotations": WRITES,
        "inputSchema": {
            "type": "object",
            "properties": {
                "id1": {"type": "string"},
                "id2": {"type": "string"},
                "rel": {"type": "string", "default": "related"},
            },
            "required": ["id1", "id2"],
        },
    },
    {
        "name": "mnemosyne_graph",
        "description": "Render linked memories as Mermaid, ASCII, or JSON.",
        "annotations": READ_ONLY,
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "depth": {"type": "integer", "default": 1},
                "format": {"type": "string", "enum": ["mermaid", "ascii", "json"], "default": "mermaid"},
            },
            "required": ["id"],
        },
    },
    {
        "name": "mnemosyne_maintain",
        "description": "Run memory lifecycle maintenance.",
        "annotations": {"readOnlyHint": False, "destructiveHint": True},
        "inputSchema": {
            "type": "object",
            "properties": {
                "scope": {"type": "string", "enum": ["global", "project", "all"], "default": "all"},
                "dry_run": {"type": "boolean", "default": True},
            },
        },
    },
    {
        "name": "mnemosyne_prep_context",
        "description": "Assemble a memory context block (core + relevant memories) for any agent task.",
        "annotations": READ_ONLY,
        "inputSchema": {
            "type": "object",
            "properties": {
                "task": {"type": "string"},
                "limit": {"type": "integer", "default": 5},
            },
            "required": ["task"],
        },
    },
]


def serve(sse: bool = False) -> int:
    if sse:
        config = load_config().get("mcp", {}).get("sse", {})
        return serve_sse(str(config.get("host", "127.0.0.1")), int(config.get("port", 3700)))
    return serve_stdio()


def serve_stdio(stdin=None, stdout=None) -> int:
    stdin = stdin or sys.stdin
    stdout = stdout or sys.stdout
    for line in stdin:
        response = process_line(line)
        if response:
            stdout.write(response + "\n")
            stdout.flush()
    return 0


def process_line(line: str) -> str:
    try:
        request = json.loads(line)
    except json.JSONDecodeError as exc:
        return json.dumps(_error(None, -32700, f"Parse error: {exc.msg}"))
    response = handle_request(request)
    return "" if response is None else json.dumps(response, ensure_ascii=False)


def handle_request(request: dict) -> dict | None:
    if not isinstance(request, dict):
        return _error(None, -32600, "Invalid Request")
    request_id = request.get("id")
    method = request.get("method")
    if method == "initialize":
        return _result(
            request_id,
            {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "mnemosyne", "version": SERVER_VERSION},
            },
        )
    if method == "notifications/initialized":
        return None
    if method == "ping":
        return _result(request_id, {})
    if method == "tools/list":
        return _result(request_id, {"tools": TOOL_SCHEMAS})
    if method != "tools/call":
        return _error(request_id, -32601, f"Method not found: {method}")
    params = request.get("params")
    if not isinstance(params, dict):
        return _error(request_id, -32602, "tools/call params must be an object")
    name = params.get("name")
    arguments = params.get("arguments", {})
    if not isinstance(arguments, dict):
        return _error(request_id, -32602, "tool arguments must be an object")
    handler = TOOL_HANDLERS.get(str(name))
    if handler is None:
        return _error(request_id, -32601, f"Tool not found: {name}")
    try:
        payload = handler(arguments)
    except (KeyError, TypeError, ValueError) as exc:
        return _error(request_id, -32602, f"Invalid tool arguments: {exc}")
    except Exception as exc:
        return _error(request_id, -32603, f"Internal error: {exc}")
    return _result(
        request_id,
        {
            "content": [{"type": "text", "text": json.dumps(payload, ensure_ascii=False)}],
            "structuredContent": payload,
        },
    )


def serve_sse(host: str, port: int) -> int:
    sessions: dict[str, queue.Queue[str]] = {}

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            parsed = urlparse(self.path)
            if parsed.path != "/sse":
                self.send_error(404)
                return
            session_id = uuid.uuid4().hex
            messages: queue.Queue[str] = queue.Queue()
            sessions[session_id] = messages
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.end_headers()
            self.wfile.write(f"event: endpoint\ndata: /messages?session_id={session_id}\n\n".encode())
            self.wfile.flush()
            try:
                while True:
                    message = messages.get()
                    self.wfile.write(f"event: message\ndata: {message}\n\n".encode())
                    self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                sessions.pop(session_id, None)

        def do_POST(self) -> None:
            parsed = urlparse(self.path)
            session_id = parse_qs(parsed.query).get("session_id", [""])[0]
            if parsed.path != "/messages" or session_id not in sessions:
                self.send_error(404)
                return
            length = int(self.headers.get("Content-Length", "0"))
            response = process_line(self.rfile.read(length).decode("utf-8"))
            if response:
                sessions[session_id].put(response)
            self.send_response(202)
            self.end_headers()

        def log_message(self, _format: str, *_args) -> None:
            return None

    server = ThreadingHTTPServer((host, port), Handler)
    print(f"mnemosyne MCP SSE listening on http://{host}:{port}/sse", file=sys.stderr)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


def _search(arguments: dict) -> list[dict]:
    scope = str(arguments.get("scope", "all"))
    stores = _allowed_stores(scope)
    config = load_config(stores[-1] if stores else None)
    limit = int(arguments.get("limit", config.get("mcp", {}).get("default_search_limit", 5)))
    results = search(
        stores,
        str(arguments["query"]),
        limit=limit,
        type_filter=str(arguments.get("type", "")),
        include_archive=bool(arguments.get("include_archive", False)),
        config=config,
    )
    return [
        {
            "id": result.memory.id,
            "scope": result.store.scope,
            "type": result.memory.type,
            "score": round(result.score, 4),
            "strength": result.memory.strength,
            "tags": result.memory.tags,
            "links": result.memory.links,
            "summary": result.memory.injection_summary,
            "path": str(result.path),
            "why_matched": result.why_matched,
            "score_breakdown": result.score_breakdown,
        }
        for result in results
    ]


def _write(arguments: dict) -> dict:
    scope = str(arguments.get("scope", "project"))
    _allowed_stores(scope)
    tags = [tag.strip() for tag in str(arguments.get("tags", "")).split(",") if tag.strip()]
    result = api.write_entry(
        type=str(arguments["type"]),
        importance=int(arguments["importance"]),
        content=str(arguments["content"]),
        title=str(arguments.get("title", "")),
        tags=tags,
        scope=scope,
        source=str(arguments.get("source", "mcp")),
        expires=str(arguments.get("expires", "")),
    )
    payload = {"status": result.status, "id": result.id}
    if result.superseded:
        payload["superseded"] = result.superseded
    return payload


def _read_core(arguments: dict) -> dict:
    return {
        store.scope: read_core(store)
        for store in _allowed_stores(str(arguments.get("scope", "all")))
    }


def _show(arguments: dict) -> str:
    found = find_memory(str(arguments["id"]), _allowed_stores("all"), include_archive=True)
    if found is None:
        raise ValueError(f"Memory not found: {arguments['id']}")
    return serialize_memory(found[2])


def _link(arguments: dict) -> dict:
    stores = _allowed_stores("all")
    for key in ("id1", "id2"):
        if find_memory(str(arguments[key]), stores, include_archive=True) is None:
            raise ValueError(f"Memory not found in exposed stores: {arguments[key]}")
    try:
        result = api.link_entries(
            str(arguments["id1"]),
            str(arguments["id2"]),
            rel=str(arguments.get("rel", "related")),
            allow_custom=bool(arguments.get("allow_custom", False)),
            stores=stores,
        )
    except api.MnemosyneError as exc:
        raise ValueError(str(exc)) from exc
    return {"ok": True, "rel": result["rel"]}


def _graph(arguments: dict) -> str:
    graph = build_graph(str(arguments["id"]), _allowed_stores("all"), depth=int(arguments.get("depth", 1)))
    return render_graph(graph, str(arguments.get("format", "mermaid")))


def _maintain(arguments: dict) -> dict:
    scope = str(arguments.get("scope", "all"))
    summary = api.maintain(
        stores=_allowed_stores(scope),
        dry_run=bool(arguments.get("dry_run", True)),
    )
    return {key: value for key, value in summary.items() if key != "core_candidates"}


def _prep_context(arguments: dict) -> str:
    return prep(
        str(arguments["task"]),
        max_memories=int(arguments.get("limit", 5)),
        stores=_allowed_stores("all"),
    )


def _allowed_stores(scope: str) -> list:
    """Resolve an MCP scope after applying project-configured exposure policy."""
    config = load_config().get("mcp", {})
    expose = {
        "global": bool(config.get("expose_global", True)),
        "project": bool(config.get("expose_project", True)),
    }
    requested = stores_for_scope(scope)
    allowed = [store for store in requested if expose.get(store.scope, False)]
    if scope != "all" and not allowed:
        raise ValueError(f"MCP access to {scope} scope is disabled")
    return allowed


def _result(request_id, result: object) -> dict:
    return {"jsonrpc": "2.0", "id": request_id, "result": result}


def _error(request_id, code: int, message: str) -> dict:
    return {"jsonrpc": "2.0", "id": request_id, "error": {"code": code, "message": message}}


TOOL_HANDLERS = {
    "mnemosyne_search": _search,
    "mnemosyne_write": _write,
    "mnemosyne_read_core": _read_core,
    "mnemosyne_show": _show,
    "mnemosyne_link": _link,
    "mnemosyne_graph": _graph,
    "mnemosyne_maintain": _maintain,
    "mnemosyne_prep_context": _prep_context,
    # Legacy alias kept for one minor cycle; clients should migrate to
    # mnemosyne_prep_context.
    "mnemosyne_codex_prep": _prep_context,
}
