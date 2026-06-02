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
                "mnemosyne_codex_prep",
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
            self.assertEqual({"ok": True}, call("mnemosyne_link", {"id1": "first", "id2": "second"}, 12))
            self.assertIn("graph LR", call("mnemosyne_graph", {"id": "first"}, 13))
            self.assertEqual(2, call("mnemosyne_maintain", {"scope": "project"}, 14)["processed"])
            self.assertIn("Project memory", call("mnemosyne_codex_prep", {"task": "linked graph"}, 15))

    def test_missing_extra_has_friendly_cli_error(self) -> None:
        from mnemosyne.mcp.server import MissingMCPDependency

        stderr = io.StringIO()
        with patch(
            "mnemosyne.mcp.server._require_mcp_sdk",
            side_effect=MissingMCPDependency(),
        ), redirect_stderr(stderr):
            code = main(["mcp", "serve"])

        self.assertEqual(1, code)
        self.assertIn("pip install mnemosyne[mcp]", stderr.getvalue())

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
