# Copyright (c) 2026 Ivan Tugay
# SPDX-License-Identifier: GPL-3.0-or-later
# Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

"""T420: the mapping from a changed-file list to the recipes `just check` runs
(tools/selective-check.sh) and to the nextest arguments (tools/test-changed.sh). Both scripts
have a dry mode, so nothing here builds or runs a test. Run: `mise exec -- pytest tools/tests`.
"""

import os
import subprocess
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _plan(changed: list[str]) -> str:
    env = {
        **os.environ,
        "SELECTIVE_DRY": "1",
        "CHECK_BASE": "HEAD",
        "RTOK_CHANGED": "\n".join(changed),
    }
    out = subprocess.run(
        [str(REPO_ROOT / "tools" / "selective-check.sh")],
        cwd=REPO_ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return next(line for line in out.splitlines() if line.startswith("plan: "))[6:]


def test_docs_only_runs_nothing():
    assert _plan(["README.md", "docs/config.md", "plan.md", ".github/workflows/ci.yml"]) == "(nothing)"


@pytest.mark.parametrize(
    "shared",
    ["Cargo.toml", "build.rs", "Cargo.lock", "justfile", "crates/rtok-plugin-sdk/src/lib.rs", "config/default.toml"],
)
def test_shared_input_runs_everything(shared):
    assert _plan(["README.md", shared]) == "full-check"


def test_source_change_runs_lint_and_selected_tests():
    steps = _plan(["src/plugins/checkpoint.rs"]).split()
    assert steps[:2] == ["fmt-check", "lint"]
    assert "build-min" in steps and "dup" in steps
    assert steps[-2:] == ["test-changed", "HEAD"]
    assert "js" not in steps and "python" not in steps


def test_test_file_change_skips_build_min():
    assert "build-min" not in _plan(["tests/it/worktree.rs"]).split()


def test_js_and_python_changes_run_only_their_lint():
    assert _plan(["web/src/App.tsx"]) == "dup js"
    assert _plan(["tools/tests/test_x.py"]) == "dup python"


def _nextest(changed: list[str]) -> str:
    env = {**os.environ, "TEST_CHANGED_DRY": "1", "RTOK_CHANGED": "\n".join(changed)}
    return subprocess.run(
        [str(REPO_ROOT / "tools" / "test-changed.sh"), "HEAD"],
        cwd=REPO_ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=True,
    ).stdout


def test_source_change_selects_unit_tests_by_module_and_tests_that_name_it():
    out = _nextest(["src/plugins/checkpoint.rs"])
    assert "--lib" in out and "kind(lib) & (test(~checkpoint)" in out
    assert "--test it" in out and "memory_status" in out and "stats_model" in out
    assert "--workspace" not in out


def test_build_script_runs_the_whole_suite():
    assert "--workspace" in _nextest(["build.rs"])


def test_source_file_without_a_module_name_runs_every_unit_test():
    out = _nextest(["src/lib.rs"])
    assert "--lib" in out and "-E" not in out


def test_a_filter_that_matches_nothing_is_not_a_failure():
    out = _nextest(["docs/zzz-nothing-names-this.md"])
    assert "--no-tests=pass" in out and "--test " not in out
