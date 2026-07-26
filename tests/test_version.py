from __future__ import annotations

import re
import tomllib
from pathlib import Path

import mnemosyne
from mnemosyne.mcp.server import SERVER_VERSION


ROOT = Path(__file__).resolve().parents[1]


def test_release_version_has_one_runtime_source() -> None:
    pyproject = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))
    plugin = (ROOT / "mnemosyne/integrations/hermes/plugin.yaml").read_text(encoding="utf-8")
    plugin_match = re.search(r"^version:\s*([^\s]+)$", plugin, re.MULTILINE)

    # Derived from mnemosyne.__version__ on purpose: pinning the literal here
    # means every release bump has to edit this test, which is the opposite of
    # having one source of truth.
    assert re.fullmatch(r"\d+\.\d+\.\d+", mnemosyne.__version__)
    assert SERVER_VERSION == mnemosyne.__version__
    assert "version" not in pyproject["project"]
    assert "version" in pyproject["project"]["dynamic"]
    assert pyproject["tool"]["hatch"]["version"]["path"] == "mnemosyne/__init__.py"
    assert plugin_match is not None
    assert plugin_match.group(1) == mnemosyne.__version__


def test_release_notes_and_readme_name_current_version() -> None:
    changelog = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    readme = (ROOT / "README.md").read_text(encoding="utf-8")

    readme_zh = (ROOT / "README.zh.md").read_text(encoding="utf-8")

    assert f"## [{mnemosyne.__version__}] - " in changelog
    assert f"Current version: {mnemosyne.__version__}" in readme
    assert f"当前版本 {mnemosyne.__version__}" in readme_zh
