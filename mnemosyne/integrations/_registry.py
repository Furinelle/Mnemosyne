"""Installer registry for `mnemosyne install <agent>`.

Each installer takes the parsed argparse namespace and returns an exit code.
Register new adapters here; docs/adapters.md describes the contract.
"""

from __future__ import annotations

import argparse


def _install_hermes(args: argparse.Namespace) -> int:
    import os
    from pathlib import Path

    from mnemosyne.integrations.hermes import _install

    home = getattr(args, "hermes_home", None) or os.environ.get("HERMES_HOME") or str(Path.home() / ".hermes")
    try:
        result = _install.install_hermes(
            hermes_home=Path(home),
            python_path=getattr(args, "python", None),
            force=bool(getattr(args, "force", False)),
            write_config=not getattr(args, "no_config", False),
            dry_run=bool(getattr(args, "dry_run", False)),
        )
    except FileExistsError as exc:
        print(f"error: {exc}")
        return 1
    if getattr(args, "dry_run", False):
        print("[dry-run] would install to", result["plugin_dir"])
        print("[dry-run] bridge python:", result["python"])
        if "config_preview" in result:
            print("[dry-run] config.yaml after edit:\n")
            print(result["config_preview"])
    else:
        print("Installed Mnemosyne provider to", result["plugin_dir"])
        print("Bridge python:", result["python"])
        if result["config_written"]:
            print("Updated config.yaml (backup:", result["backup"], ")")
            print("Restart the host to activate (memory.provider: mnemosyne).")
        else:
            print("Skipped config.yaml — set memory.provider: mnemosyne manually.")
    return 0


def _install_claude_code(args: argparse.Namespace) -> int:
    from mnemosyne.store import templates_dir

    settings_path = templates_dir() / "agents" / "claude_code" / "settings.json"
    claude_md_path = templates_dir() / "agents" / "claude_code" / "CLAUDE.md"
    print("Claude Code integration is hook-based; two manual merge steps:")
    print()
    print("1. Merge this hooks config into ~/.claude/settings.json")
    print("   (or the project's .claude/settings.json):")
    print(f"     {settings_path}")
    print("2. Optionally append the usage rules to your CLAUDE.md:")
    print(f"     {claude_md_path}")
    print()
    print("Hooks call `python3 -m mnemosyne.hooks.<event>` — ensure mnemosyne")
    print("is importable from any cwd (pip install -e . or a global install).")
    return 0


INSTALLERS = {
    "hermes": _install_hermes,
    "claude-code": _install_claude_code,
}
