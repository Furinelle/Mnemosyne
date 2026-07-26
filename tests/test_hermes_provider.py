from __future__ import annotations

import json
import subprocess
import sys
import unittest

from mnemosyne import store as mstore
from mnemosyne.integrations.hermes import MnemosyneMemoryProvider, register
from tests.helpers import isolated_workspace


def _provider(**cfg):
    base = {"python": sys.executable, "timeout": 30}
    base.update(cfg)
    return MnemosyneMemoryProvider(config=base)


def _seed_core(core_text="- shared core line"):
    gs = mstore.global_store()
    mstore.ensure_store(gs)
    gs.core_path.write_text("# Global Core Memory\n\n" + core_text + "\n", encoding="utf-8")
    return gs


def _cli(*args):
    subprocess.run([sys.executable, "-m", "mnemosyne", *args],
                   check=True, capture_output=True, text=True)


class ProviderBasicsTests(unittest.TestCase):
    def test_name_is_mnemosyne(self):
        self.assertEqual(MnemosyneMemoryProvider(config={}).name, "mnemosyne")

    def test_register_collects_provider(self):
        class Ctx:
            def __init__(self):
                self.provider = None

            def register_memory_provider(self, provider):
                self.provider = provider

        ctx = Ctx()
        register(ctx)
        self.assertIsNotNone(ctx.provider)
        self.assertEqual(ctx.provider.name, "mnemosyne")


class ProviderBridgeTests(unittest.TestCase):
    def test_is_available_true_with_real_python(self):
        with isolated_workspace():
            _seed_core()
            p = _provider()
            p.initialize("sess-1")
            self.assertTrue(p.is_available())

    def test_is_available_false_with_bad_python(self):
        with isolated_workspace():
            _seed_core()
            p = _provider(python="/nonexistent/python-xyz")
            self.assertFalse(p.is_available())

    def test_run_returns_empty_on_failure(self):
        with isolated_workspace():
            _seed_core()
            p = _provider(python="/nonexistent/python-xyz")
            self.assertEqual(p._run(["read", "--scope", "all"]), "")
            self.assertEqual(p._run(["search", "x", "--format", "json"], json_out=True), [])

    def test_initialize_sets_source_from_profile(self):
        with isolated_workspace():
            _seed_core()
            p = _provider()
            p.initialize("sess-1", agent_identity="coder")
            self.assertEqual(p._source, "hermes:coder")


class ProviderRecallTests(unittest.TestCase):
    def test_system_prompt_block_includes_core(self):
        with isolated_workspace():
            _seed_core("- remember the alamo")
            p = _provider()
            p.initialize("sess-1")
            block = p.system_prompt_block()
            self.assertIn("Mnemosyne Shared Memory", block)
            self.assertIn("remember the alamo", block)

    def test_system_prompt_block_empty_when_no_store(self):
        with isolated_workspace():
            p = _provider()
            p.initialize("sess-1")
            self.assertEqual(p.system_prompt_block(), "")

    def test_prefetch_returns_matching_memory(self):
        with isolated_workspace():
            _seed_core()
            _cli("write", "--scope", "global", "--type", "codebase", "--importance", "70",
                 "--source", "test", "--force", "--title", "Widget service entrypoint",
                 "--content", "The widget service starts in widget_main.py and binds port 9090.",
                 "--tags", "widget,service")
            p = _provider()
            p.initialize("sess-1")
            out = p.prefetch("widget service port")
            self.assertIn("Mnemosyne recall", out)
            self.assertIn("widget", out.lower())

    def test_prefetch_empty_query_returns_empty(self):
        with isolated_workspace():
            _seed_core()
            p = _provider()
            p.initialize("sess-1")
            self.assertEqual(p.prefetch("   "), "")


class ProviderToolTests(unittest.TestCase):
    def test_tool_schema_is_single_mnemosyne_tool(self):
        with isolated_workspace():
            _seed_core()
            p = _provider()
            p.initialize("sess-1")
            schemas = p.get_tool_schemas()
            self.assertEqual(len(schemas), 1)
            self.assertEqual(schemas[0]["name"], "mnemosyne")
            self.assertIn("action", schemas[0]["parameters"]["properties"])

    def test_write_then_search_roundtrip(self):
        with isolated_workspace():
            _seed_core()
            p = _provider()
            p.initialize("sess-1", agent_identity="coder")
            w = json.loads(p.handle_tool_call("mnemosyne", {
                "action": "write", "type": "preference", "importance": 65,
                "scope": "global", "title": "Indentation preference",
                "content": "User prefers tabs over spaces in Go code.",
                "tags": "style,go",
            }))
            self.assertEqual(w["status"], "ok")
            s = json.loads(p.handle_tool_call("mnemosyne", {"action": "search", "query": "tabs spaces go"}))
            self.assertGreaterEqual(s["count"], 1)

    def test_write_requires_type_and_importance(self):
        with isolated_workspace():
            _seed_core()
            p = _provider()
            p.initialize("sess-1")
            r = json.loads(p.handle_tool_call("mnemosyne", {"action": "write", "title": "x"}))
            self.assertIn("error", r)

    def test_unknown_action_errors(self):
        with isolated_workspace():
            _seed_core()
            p = _provider()
            p.initialize("sess-1")
            r = json.loads(p.handle_tool_call("mnemosyne", {"action": "frobnicate"}))
            self.assertIn("error", r)

    def test_on_memory_write_mirrors_when_enabled(self):
        with isolated_workspace():
            _seed_core()
            p = _provider(mirror_builtin_writes=True)
            p.initialize("sess-1")
            p.on_memory_write("add", "user", "User lives in the UTC+8 timezone.")
            s = json.loads(p.handle_tool_call("mnemosyne", {"action": "search", "query": "timezone UTC+8"}))
            self.assertGreaterEqual(s["count"], 1)


class ProviderSessionEndTests(unittest.TestCase):
    def test_on_session_end_noop_when_distill_disabled(self):
        with isolated_workspace() as (project, _home):
            _seed_core()
            p = _provider()
            p.initialize("sess-1")
            p.on_session_end([
                {"role": "user", "content": "不要用 print 调试，改用 logging"},
            ])
            s = json.loads(p.handle_tool_call("mnemosyne", {"action": "search", "query": "logging print"}))
            self.assertEqual(s["count"], 0)

    def test_on_session_end_distills_when_enabled(self):
        with isolated_workspace() as (project, _home):
            _seed_core()
            from mnemosyne.store import ensure_store, project_store

            ensure_store(project_store())
            (project / ".mnemosyne" / "config.toml").write_text(
                "[distill]\nenabled = true\n", encoding="utf-8"
            )
            p = _provider()
            p.initialize("sess-1")
            p.on_session_end([
                {"role": "user", "content": "不要用 print 调试，改用 logging"},
            ])
            s = json.loads(p.handle_tool_call("mnemosyne", {"action": "search", "query": "logging print"}))
            self.assertGreaterEqual(s["count"], 1)

    def test_on_session_end_empty_messages_noop(self):
        with isolated_workspace():
            _seed_core()
            p = _provider()
            p.initialize("sess-1")
            p.on_session_end([])  # should not raise


if __name__ == "__main__":
    unittest.main()


class CLIBridgeTests(unittest.TestCase):
    def test_clibridge_run_json_roundtrip(self):
        from mnemosyne.integrations._bridge import CLIBridge
        bridge = CLIBridge()
        payload = bridge.run_json(["-c", "import json; print(json.dumps({'ok': 1}))"], raw_python=True)
        self.assertEqual(payload, {"ok": 1})

    def test_bridge_python_env_override(self):
        import os
        from mnemosyne.integrations._bridge import CLIBridge
        old = os.environ.get("MNEMOSYNE_BRIDGE_PYTHON")
        os.environ["MNEMOSYNE_BRIDGE_PYTHON"] = "/nonexistent/python3"
        try:
            self.assertEqual(CLIBridge().candidates[0], "/nonexistent/python3")
        finally:
            if old is None:
                os.environ.pop("MNEMOSYNE_BRIDGE_PYTHON", None)
            else:
                os.environ["MNEMOSYNE_BRIDGE_PYTHON"] = old

    def test_hermes_tool_description_neutral(self):
        from mnemosyne.integrations.hermes import MNEMOSYNE_TOOL
        self.assertNotIn("Claude Code, Codex", str(MNEMOSYNE_TOOL))
