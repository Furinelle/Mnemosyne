from __future__ import annotations

import io
import json
import os
import subprocess
import sys
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from unittest.mock import patch

from mnemosyne.cli import main
from mnemosyne.schema import Memory
from mnemosyne.store import ensure_store, project_store, working_path, write_memory
from tests.helpers import isolated_workspace


def _result_payload(response: dict) -> object:
    return json.loads(response["result"]["content"][0]["text"])


class MCPServerTests(unittest.TestCase):
    def test_tools_list_matches_documented_surface(self) -> None:
        from mnemosyne.mcp.server import TOOL_SCHEMAS, handle_request

        response = handle_request({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})

        names = [tool["name"] for tool in response["result"]["tools"]]
        self.assertEqual(
            [
                "mnemosyne_search",
                "mnemosyne_write",
                "mnemosyne_read_core",
                "mnemosyne_show",
                "mnemosyne_link",
                "mnemosyne_graph",
                "mnemosyne_maintain",
                "mnemosyne_prep_context",
            ],
            names,
        )
        self.assertEqual(names, [tool["name"] for tool in TOOL_SCHEMAS])

    def test_search_tool_end_to_end(self) -> None:
        from mnemosyne.mcp.server import handle_request

        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            memory = Memory(
                id="mcp-search",
                type="codebase",
                strength=70,
                canonical_summary="searchable mcp needle",
                injection_summary="searchable mcp needle",
                body="searchable mcp needle",
            )
            write_memory(working_path(store, memory), memory)

            response = handle_request(
                {
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "tools/call",
                    "params": {"name": "mnemosyne_search", "arguments": {"query": "mcp needle", "scope": "project"}},
                }
            )

            self.assertEqual(["mcp-search"], [item["id"] for item in _result_payload(response)])

    def test_write_tool_can_be_searched(self) -> None:
        from mnemosyne.mcp.server import handle_request

        with isolated_workspace():
            write_response = handle_request(
                {
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "tools/call",
                    "params": {
                        "name": "mnemosyne_write",
                        "arguments": {
                            "type": "codebase",
                            "importance": 70,
                            "content": "written through protocol unique-mcp-write",
                            "scope": "project",
                        },
                    },
                }
            )
            memory_id = _result_payload(write_response)["id"]
            search_response = handle_request(
                {
                    "jsonrpc": "2.0",
                    "id": 4,
                    "method": "tools/call",
                    "params": {
                        "name": "mnemosyne_search",
                        "arguments": {"query": "unique-mcp-write", "scope": "project"},
                    },
                }
            )

            self.assertEqual(memory_id, _result_payload(search_response)[0]["id"])

    def test_repeated_write_returns_structured_duplicate_result(self) -> None:
        from mnemosyne.mcp.server import handle_request

        request = {
            "jsonrpc": "2.0",
            "id": 31,
            "method": "tools/call",
            "params": {
                "name": "mnemosyne_write",
                "arguments": {
                    "type": "pitfall",
                    "importance": 70,
                    "title": "Stable MCP write",
                    "content": "Clear the stale MCP cache before retrying.",
                    "scope": "project",
                },
            },
        }
        with isolated_workspace():
            first = handle_request(request)
            request["id"] = 32
            second = handle_request(request)

            self.assertNotIn("error", first)
            self.assertNotIn("error", second)
            first_payload = _result_payload(first)
            second_payload = _result_payload(second)
            self.assertEqual("created", first_payload["status"])
            self.assertEqual("duplicate", second_payload["status"])
            self.assertEqual(first_payload["id"], second_payload["id"])

    def test_exposure_flags_apply_to_all_mcp_surfaces(self) -> None:
        from mnemosyne.mcp.server import handle_request
        from mnemosyne.store import global_store

        with isolated_workspace():
            project = project_store()
            global_scope = global_store()
            ensure_store(project)
            ensure_store(global_scope)
            project.core_path.write_text("PROJECT_ONLY_CORE", encoding="utf-8")
            global_scope.core_path.write_text("HIDDEN_GLOBAL_CORE", encoding="utf-8")
            project.config_path.write_text(
                project.config_path.read_text(encoding="utf-8").replace(
                    "expose_global = true", "expose_global = false"
                ),
                encoding="utf-8",
            )
            for store, memory_id in ((project, "visible-project"), (global_scope, "hidden-global")):
                memory = Memory(
                    id=memory_id,
                    type="codebase",
                    strength=70,
                    canonical_summary="shared exposure needle",
                    injection_summary="shared exposure needle",
                    body="shared exposure needle",
                )
                write_memory(working_path(store, memory), memory)

            def request(name: str, arguments: dict, request_id: int) -> dict:
                return handle_request(
                    {
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "method": "tools/call",
                        "params": {"name": name, "arguments": arguments},
                    }
                )

            search = _result_payload(request(
                "mnemosyne_search", {"query": "exposure needle", "scope": "all"}, 40
            ))
            self.assertEqual(["visible-project"], [item["id"] for item in search])

            core = _result_payload(request("mnemosyne_read_core", {"scope": "all"}, 41))
            self.assertEqual({"project": "PROJECT_ONLY_CORE"}, core)
            self.assertEqual(-32602, request("mnemosyne_show", {"id": "hidden-global"}, 42)["error"]["code"])
            self.assertEqual(-32602, request(
                "mnemosyne_write",
                {"type": "pitfall", "importance": 60, "content": "hidden write", "scope": "global"},
                43,
            )["error"]["code"])
            self.assertEqual(-32602, request(
                "mnemosyne_link", {"id1": "visible-project", "id2": "hidden-global"}, 44
            )["error"]["code"])
            self.assertEqual(-32602, request(
                "mnemosyne_graph", {"id": "hidden-global"}, 45
            )["error"]["code"])

            maintain = _result_payload(request(
                "mnemosyne_maintain", {"scope": "all", "dry_run": True}, 46
            ))
            self.assertEqual(1, maintain["processed"])
            prepared = _result_payload(request(
                "mnemosyne_codex_prep", {"task": "exposure needle"}, 47
            ))
            self.assertIn("PROJECT_ONLY_CORE", prepared)
            self.assertNotIn("HIDDEN_GLOBAL_CORE", prepared)
            self.assertNotIn("hidden-global", prepared)

    def test_disabled_project_scope_is_rejected_and_all_keeps_global(self) -> None:
        from mnemosyne.mcp.server import handle_request
        from mnemosyne.store import global_store

        with isolated_workspace():
            project = project_store()
            ensure_store(project)
            project.config_path.write_text(
                project.config_path.read_text(encoding="utf-8").replace(
                    "expose_project = true", "expose_project = false"
                ),
                encoding="utf-8",
            )
            global_scope = global_store()
            ensure_store(global_scope)
            memory = Memory(
                id="global-visible",
                type="codebase",
                strength=70,
                canonical_summary="global policy needle",
                injection_summary="global policy needle",
                body="global policy needle",
            )
            write_memory(working_path(global_scope, memory), memory)

            project_read = handle_request({
                "jsonrpc": "2.0",
                "id": 48,
                "method": "tools/call",
                "params": {"name": "mnemosyne_read_core", "arguments": {"scope": "project"}},
            })
            all_search = handle_request({
                "jsonrpc": "2.0",
                "id": 49,
                "method": "tools/call",
                "params": {
                    "name": "mnemosyne_search",
                    "arguments": {"query": "policy needle", "scope": "all"},
                },
            })

            self.assertEqual(-32602, project_read["error"]["code"])
            self.assertEqual(["global-visible"], [item["id"] for item in _result_payload(all_search)])

    def test_invalid_tool_and_json_return_standard_errors(self) -> None:
        from mnemosyne.mcp.server import handle_request, process_line

        invalid_tool = handle_request(
            {
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {"name": "missing", "arguments": {}},
            }
        )
        invalid_json = json.loads(process_line("{"))

        self.assertEqual(-32601, invalid_tool["error"]["code"])
        self.assertEqual(-32700, invalid_json["error"]["code"])

    def test_read_show_link_graph_maintain_and_codex_prep_tools(self) -> None:
        from mnemosyne.mcp.server import handle_request

        with isolated_workspace():
            store = project_store()
            ensure_store(store)
            store.core_path.write_text("# Shared core", encoding="utf-8")
            for memory_id in ("first", "second"):
                memory = Memory(id=memory_id, type="codebase", strength=70, body=f"## {memory_id}")
                write_memory(working_path(store, memory), memory)

            def call(name: str, arguments: dict, request_id: int) -> object:
                return _result_payload(
                    handle_request(
                        {
                            "jsonrpc": "2.0",
                            "id": request_id,
                            "method": "tools/call",
                            "params": {"name": name, "arguments": arguments},
                        }
                    )
                )

            self.assertIn("# Shared core", call("mnemosyne_read_core", {"scope": "project"}, 10)["project"])
            self.assertIn("id: first", call("mnemosyne_show", {"id": "first"}, 11))
            self.assertEqual({"ok": True, "rel": "related"}, call("mnemosyne_link", {"id1": "first", "id2": "second"}, 12))
            self.assertIn("graph LR", call("mnemosyne_graph", {"id": "first"}, 13))
            self.assertEqual(2, call("mnemosyne_maintain", {"scope": "project"}, 14)["processed"])
            self.assertIn("Project memory", call("mnemosyne_prep_context", {"task": "linked graph"}, 15))
            # legacy alias keeps working for one minor cycle
            self.assertIn("Project memory", call("mnemosyne_codex_prep", {"task": "linked graph"}, 16))

    def test_stdio_child_responds_and_exits_on_eof(self) -> None:
        root = Path(__file__).resolve().parents[1]
        env = dict(os.environ)
        env["MNEMOSYNE_MCP_ALLOW_STDLIB"] = "1"
        env["PYTHONPATH"] = str(root)
        with isolated_workspace() as (project, _home):
            process = subprocess.Popen(
                [sys.executable, "-m", "mnemosyne", "mcp", "serve"],
                cwd=project,
                env=env,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            assert process.stdin is not None
            assert process.stdout is not None
            process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": 6, "method": "tools/list"}) + "\n")
            process.stdin.flush()
            response = json.loads(process.stdout.readline())
            process.stdin.close()
            process.wait(timeout=5)
            process.stdout.close()
            assert process.stderr is not None
            process.stderr.close()

        self.assertEqual(8, len(response["result"]["tools"]))
        self.assertEqual(0, process.returncode)


if __name__ == "__main__":
    unittest.main()


class TestUniversalMcpSurface:
    def test_prep_context_tool_listed_with_alias(self) -> None:
        from mnemosyne.mcp.server import TOOL_HANDLERS, TOOL_SCHEMAS
        names = {schema["name"] for schema in TOOL_SCHEMAS}
        assert "mnemosyne_prep_context" in names
        assert "mnemosyne_codex_prep" not in names
        assert "mnemosyne_codex_prep" in TOOL_HANDLERS

    def test_tools_have_annotations(self) -> None:
        from mnemosyne.mcp.server import TOOL_SCHEMAS
        by_name = {schema["name"]: schema for schema in TOOL_SCHEMAS}
        assert by_name["mnemosyne_search"]["annotations"]["readOnlyHint"] is True
        assert by_name["mnemosyne_write"]["annotations"]["readOnlyHint"] is False

    def test_serve_no_longer_requires_mcp_sdk(self, monkeypatch) -> None:
        import mnemosyne.mcp.server as server
        monkeypatch.delenv("MNEMOSYNE_MCP_ALLOW_STDLIB", raising=False)
        assert not hasattr(server, "_require_mcp_sdk")

    def test_write_returns_structured_result(self, tmp_store) -> None:
        from mnemosyne.mcp.server import _write
        payload = _write({"type": "pitfall", "importance": 60, "content": "mcp body one", "title": "M"})
        assert payload["status"] == "created" and payload["id"].startswith("pitfall-")
