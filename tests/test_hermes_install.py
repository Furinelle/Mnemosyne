from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

from mnemosyne.integrations.hermes import _install

PY = "/opt/homebrew/bin/python3"


class ConfigEditTests(unittest.TestCase):
    def test_set_provider_replaces_existing(self):
        src = "memory:\n  memory_enabled: true\n  provider: ''\n  nudge_interval: 10\nmodel:\n  default: x\n"
        out = _install.set_memory_provider(src)
        self.assertIn("  provider: mnemosyne\n", out)
        self.assertNotIn("  provider: ''", out)
        self.assertIn("  memory_enabled: true\n", out)
        self.assertIn("model:\n  default: x\n", out)

    def test_set_provider_inserts_when_absent(self):
        src = "memory:\n  memory_enabled: true\nmodel:\n  default: x\n"
        out = _install.set_memory_provider(src)
        self.assertIn("  provider: mnemosyne\n", out)

    def test_set_provider_creates_block_when_no_memory(self):
        src = "model:\n  default: x\n"
        out = _install.set_memory_provider(src)
        self.assertIn("memory:\n  provider: mnemosyne\n", out)
        self.assertIn("model:\n  default: x\n", out)

    def test_upsert_plugins_appends_block_when_absent(self):
        src = "memory:\n  provider: mnemosyne\n"
        out = _install.upsert_plugins_mnemosyne(src, PY)
        self.assertIn("plugins:\n  mnemosyne:\n", out)
        self.assertIn(f"    python: {PY}\n", out)
        self.assertIn("    recall_limit: 5\n", out)

    def test_upsert_plugins_into_existing_plugins_block(self):
        src = "plugins:\n  other:\n    foo: bar\nmemory:\n  provider: mnemosyne\n"
        out = _install.upsert_plugins_mnemosyne(src, PY)
        self.assertIn("  mnemosyne:\n", out)
        self.assertIn("  other:\n    foo: bar\n", out)
        self.assertEqual(out.count("plugins:\n"), 1)

    def test_upsert_plugins_is_idempotent(self):
        src = "memory:\n  provider: mnemosyne\n"
        once = _install.upsert_plugins_mnemosyne(src, PY)
        twice = _install.upsert_plugins_mnemosyne(once, PY)
        self.assertEqual(once, twice)


class InstallTests(unittest.TestCase):
    def test_install_copies_files_and_edits_config(self):
        with tempfile.TemporaryDirectory() as tmp:
            hermes_home = Path(tmp) / ".hermes"
            hermes_home.mkdir()
            (hermes_home / "config.yaml").write_text(
                "memory:\n  memory_enabled: true\n  provider: ''\nmodel:\n  default: x\n",
                encoding="utf-8")
            result = _install.install_hermes(
                hermes_home=hermes_home, python_path=sys.executable,
                force=False, write_config=True)
            plugin_dir = hermes_home / "plugins" / "mnemosyne"
            self.assertTrue((plugin_dir / "__init__.py").exists())
            self.assertTrue((plugin_dir / "plugin.yaml").exists())
            cfg = (hermes_home / "config.yaml").read_text(encoding="utf-8")
            self.assertIn("provider: mnemosyne", cfg)
            self.assertIn("mnemosyne:", cfg)
            self.assertIn(f"python: {sys.executable}", cfg)
            backups = list(hermes_home.glob("config.yaml.mnemosyne-bak-*"))
            self.assertEqual(len(backups), 1)
            self.assertTrue(result["config_written"])

    def test_install_dry_run_writes_nothing(self):
        with tempfile.TemporaryDirectory() as tmp:
            hermes_home = Path(tmp) / ".hermes"
            hermes_home.mkdir()
            (hermes_home / "config.yaml").write_text("memory:\n  provider: ''\n", encoding="utf-8")
            _install.install_hermes(hermes_home=hermes_home, python_path=sys.executable,
                                    force=False, write_config=True, dry_run=True)
            self.assertFalse((hermes_home / "plugins" / "mnemosyne").exists())
            self.assertIn("provider: ''", (hermes_home / "config.yaml").read_text(encoding="utf-8"))

    def test_install_no_config_skips_config(self):
        with tempfile.TemporaryDirectory() as tmp:
            hermes_home = Path(tmp) / ".hermes"
            hermes_home.mkdir()
            (hermes_home / "config.yaml").write_text("memory:\n  provider: ''\n", encoding="utf-8")
            res = _install.install_hermes(hermes_home=hermes_home, python_path=sys.executable,
                                          force=False, write_config=False)
            self.assertTrue((hermes_home / "plugins" / "mnemosyne" / "__init__.py").exists())
            self.assertIn("provider: ''", (hermes_home / "config.yaml").read_text(encoding="utf-8"))
            self.assertFalse(res["config_written"])


if __name__ == "__main__":
    unittest.main()


def test_installed_plugin_is_self_contained(tmp_path):
    """Hermes venv 通常没有 mnemosyne 包：安装产物必须能独立加载。"""
    import importlib.util
    import sys

    from mnemosyne.integrations.hermes._install import install_hermes

    home = tmp_path / "hermes_home"
    result = install_hermes(hermes_home=home, write_config=False)
    plugin_dir = Path(result["plugin_dir"])
    assert (plugin_dir / "_bridge.py").exists()

    poisoned = ["mnemosyne", "mnemosyne.integrations", "mnemosyne.integrations._bridge"]
    saved = {name: sys.modules.get(name) for name in poisoned}
    try:
        for name in poisoned:
            sys.modules[name] = None  # 使 import 抛 ImportError，模拟宿主 venv 缺包
        spec = importlib.util.spec_from_file_location(
            "hermes_plugin_standalone_test", plugin_dir / "__init__.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        assert hasattr(module, "MnemosyneMemoryProvider")
        assert module.MnemosyneMemoryProvider().name == "mnemosyne"
    finally:
        for name, original in saved.items():
            if original is None and name not in poisoned:
                continue
            if saved[name] is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = original
