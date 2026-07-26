"""Reusable subprocess bridge for host-process adapters.

Any plugin that lives inside another agent's process (Hermes today) needs the
same plumbing: find a Python interpreter that can import mnemosyne, shell out
to the CLI with a timeout, and degrade to empty results on any failure so the
host never blocks on memory. CLIBridge packages that pattern.
"""

from __future__ import annotations

import json
import logging
import os
import shutil
import subprocess
import sys

logger = logging.getLogger(__name__)

ENV_PYTHON_OVERRIDE = "MNEMOSYNE_BRIDGE_PYTHON"


def default_python_candidates() -> list[str]:
    return [
        candidate
        for candidate in (shutil.which("python3"), "/opt/homebrew/bin/python3", sys.executable)
        if candidate
    ]


class CLIBridge:
    def __init__(
        self,
        python_candidates: list[str] | None = None,
        timeout: float = 10.0,
        allow_env_override: bool = True,
    ):
        candidates = list(python_candidates) if python_candidates is not None else default_python_candidates()
        override = os.environ.get(ENV_PYTHON_OVERRIDE) if allow_env_override else None
        self.candidates = ([override] if override else []) + candidates
        self.timeout = timeout
        self._resolved = False
        self._python: str | None = None

    def resolve(self) -> str | None:
        """First candidate whose interpreter can import mnemosyne (cached)."""
        if self._resolved:
            return self._python
        self._resolved = True
        for candidate in self.candidates:
            if candidate and self._python_has_mnemosyne(candidate):
                self._python = candidate
                return candidate
        self._python = None
        return None

    def _python_has_mnemosyne(self, py: str) -> bool:
        try:
            proc = subprocess.run(
                [py, "-c", "import mnemosyne"],
                capture_output=True, text=True, timeout=10,
            )
            return proc.returncode == 0
        except Exception:
            return False

    def run(
        self,
        args: list[str],
        stdin: str | None = None,
        raw_python: bool = False,
    ) -> tuple[int, str, str]:
        """Run `python -m mnemosyne <args>` (or `python <args>` with raw_python).

        Returns (returncode, stdout, stderr); (-1, "", message) when no usable
        interpreter exists or the subprocess itself fails.
        """
        py = self.resolve()
        if not py:
            return (-1, "", "no python interpreter with mnemosyne available")
        argv = [py, *args] if raw_python else [py, "-m", "mnemosyne", *args]
        try:
            proc = subprocess.run(
                argv,
                input=stdin,
                capture_output=True, text=True,
                timeout=self.timeout, cwd=os.getcwd(),
            )
        except Exception as exc:
            logger.debug("bridge %s failed: %s", args[:1], exc)
            return (-1, "", str(exc))
        return (proc.returncode, proc.stdout, proc.stderr)

    def run_json(
        self,
        args: list[str],
        stdin: str | None = None,
        raw_python: bool = False,
    ):
        """Like run(), parsing stdout as JSON; None on any failure."""
        code, out, err = self.run(args, stdin=stdin, raw_python=raw_python)
        if code != 0:
            logger.debug("bridge %s exit %s: %s", args[:1], code, err[:200])
            return None
        if not out.strip():
            return None
        try:
            return json.loads(out)
        except Exception:
            return None
