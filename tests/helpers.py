from __future__ import annotations

import os
import tempfile
from contextlib import contextmanager
from pathlib import Path


@contextmanager
def isolated_workspace():
    old_cwd = Path.cwd()
    old_home = os.environ.get("MNEMOSYNE_HOME")
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        project = root / "project"
        home = root / "home"
        project.mkdir()
        home.mkdir()
        os.environ["MNEMOSYNE_HOME"] = str(home)
        os.chdir(project)
        try:
            yield project, home
        finally:
            os.chdir(old_cwd)
            if old_home is None:
                os.environ.pop("MNEMOSYNE_HOME", None)
            else:
                os.environ["MNEMOSYNE_HOME"] = old_home

