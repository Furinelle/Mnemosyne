"""MCP-compatible JSON-RPC server with stdio and optional SSE transports."""

from __future__ import annotations

import argparse
import io
import json
import os
import queue
import sys
import uuid
from contextlib import redirect_stdout
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Callable
from urllib.parse import parse_qs, urlparse

from mnemosyne.codex import prep
from mnemosyne.fusion import search
from mnemosyne.graph import build_graph, render_graph
from mnemosyne.schema import serialize_memory
from mnemosyne.store import find_memory, load_config, read_core, stores_for_scope


class MissingMCPDependency(RuntimeError):
    pass


TOOL_SCHEMAS = [
    {
        "name": "mnemosyne_search",
        "description": "Search durable memories.",
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
        "inputSchema": {
            "type": "object",
            "properties": {"scope": {"type": "string", "enum": ["global", "project", "all"], "default": "all"}},
        },
    },
    {
        "name": "mnemosyne_show",
        "description": "Show a memory by ID.",
        "inputSchema": {
            "type": "object",
            "properties": {"id": {"type": "string"}},
            "required": ["id"],
        },
    },
    {
        "name": "mnemosyne_link",
        "description": "Link two memories with a typed relation.",
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
        "inputSchema": {
            "type": "object",
            "properties": {
                "scope": {"type": "string", "enum": ["global", "project", "all"], "default": "all"},
                "dry_run": {"type": "boolean", "default": True},
            },
        },
    },
    {
        "name": "mnemosyne_codex_prep",
        "description": "Prepare a prompt prefix for another coding agent.",
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
    if not os.environ.get("MNEMOSYNE_MCP_ALLOW_STDLIB"):
        _require_mcp_sdk()
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
                "serverInfo": {"name": "mnemosyne", "version": "0.2.0"},
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
    stores = stores_for_scope(scope)
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
    from mnemosyne.cli import cmd_write

    args = argparse.Namespace(
        type=str(arguments["type"]),
        importance=int(arguments["importance"]),
        scope=str(arguments.get("scope", "project")),
        source=str(arguments.get("source", "mcp")),
        tags=str(arguments.get("tags", "")),
        title=str(arguments.get("title", "")),
        content=str(arguments["content"]),
        expires=str(arguments.get("expires", "")),
        force=True,
    )
    output = _capture_command(lambda: cmd_write(args))
    memory_id = output.strip().splitlines()[-1].removeprefix("Wrote ")
    return {"id": memory_id}


def _read_core(arguments: dict) -> dict:
    return {
        store.scope: read_core(store)
        for store in stores_for_scope(str(arguments.get("scope", "all")))
    }


def _show(arguments: dict) -> str:
    found = find_memory(str(arguments["id"]), stores_for_scope("all"), include_archive=True)
    if found is None:
        raise ValueError(f"Memory not found: {arguments['id']}")
    return serialize_memory(found[2])


def _link(arguments: dict) -> dict:
    from mnemosyne.cli import cmd_link

    args = argparse.Namespace(
        id1=str(arguments["id1"]),
        id2=str(arguments["id2"]),
        rel=str(arguments.get("rel", "related")),
        allow_custom=bool(arguments.get("allow_custom", False)),
    )
    _capture_command(lambda: cmd_link(args))
    return {"ok": True}


def _graph(arguments: dict) -> str:
    graph = build_graph(str(arguments["id"]), stores_for_scope("all"), depth=int(arguments.get("depth", 1)))
    return render_graph(graph, str(arguments.get("format", "mermaid")))


def _maintain(arguments: dict) -> dict:
    from mnemosyne.cli import cmd_maintain

    args = argparse.Namespace(scope=str(arguments.get("scope", "all")), dry_run=bool(arguments.get("dry_run", True)))
    output = _capture_command(lambda: cmd_maintain(args))
    summary: dict[str, int] = {}
    for line in output.splitlines():
        if ": " in line:
            key, value = line.split(": ", 1)
            if value.isdigit():
                summary[key] = int(value)
    return summary


def _codex_prep(arguments: dict) -> str:
    return prep(str(arguments["task"]), max_memories=int(arguments.get("limit", 5)))


def _capture_command(command: Callable[[], int]) -> str:
    output = io.StringIO()
    with redirect_stdout(output):
        code = command()
    if code:
        raise ValueError(output.getvalue().strip() or f"command exited {code}")
    return output.getvalue()


def _require_mcp_sdk() -> None:
    try:
        import mcp  # noqa: F401
    except ModuleNotFoundError as exc:
        raise MissingMCPDependency() from exc


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
    "mnemosyne_codex_prep": _codex_prep,
}
