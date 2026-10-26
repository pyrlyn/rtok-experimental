# Copyright (c) 2026 Ivan Tugay
# SPDX-License-Identifier: GPL-3.0-or-later
# Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html
"""tools/gen_openapi.py: committed models match the spec."""
import importlib.util
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("gen_openapi", ROOT / "tools" / "gen_openapi.py")
gen = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gen)


def test_every_spec_has_a_committed_module():
    for name, (out, module) in gen.SPECS.items():
        assert (ROOT / "openapi" / name).is_file()
        assert (out / "mod.rs").is_file()
        assert module.startswith("crate::")


def test_size_bytes_is_in_spec_and_model():
    assert "size_bytes" in (ROOT / "openapi" / "skills.yaml").read_text()
    assert "pub size_bytes: i64" in (ROOT / "src" / "skills" / "api" / "skill.rs").read_text()


@pytest.mark.skipif(not (shutil.which("npx") and shutil.which("java")), reason="needs npx and java")
def test_check_passes():
    r = subprocess.run([sys.executable, str(ROOT / "tools" / "gen_openapi.py"), "--check"])
    assert r.returncode == 0
