# Copyright (c) 2026 Ivan Tugay
# SPDX-License-Identifier: GPL-3.0-or-later
# Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

"""T279: tests for tools/plugin-versions.sh, the one place that writes and checks every plugin
manifest version and `.rtok-plugin-version` file. Run: `mise exec -- pytest tools/tests` from
the repo root, or `just check` (wired into the `python` recipe).
"""

import re
import shutil
import stat
import subprocess
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPT = REPO_ROOT / "tools" / "plugin-versions.sh"


def _current_version() -> str:
    text = (REPO_ROOT / "Cargo.toml").read_text()
    m = re.search(r'^version = "([^"]+)"', text, re.MULTILINE)
    assert m, "Cargo.toml has no top-level version"
    return m.group(1)


CURRENT_VERSION = _current_version()


def _copy_tree(dest: Path) -> Path:
    """A minimal, writable copy of the repo: the script plus the plugin trees it touches, so a
    test can break one file without disturbing the real working tree."""
    tools_dir = dest / "tools"
    tools_dir.mkdir(parents=True)
    script = tools_dir / "plugin-versions.sh"
    shutil.copy(SCRIPT, script)
    script.chmod(script.stat().st_mode | stat.S_IEXEC)
    shutil.copytree(REPO_ROOT / "plugins", dest / "plugins")
    return dest


def _run(root: Path, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [str(root / "tools" / "plugin-versions.sh"), *args],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )


def test_check_passes_on_the_real_tree():
    result = _run(REPO_ROOT, "--check", CURRENT_VERSION)
    assert result.returncode == 0, result.stdout + result.stderr


def test_check_fails_after_editing_one_version_file_and_names_it(tmp_path):
    root = _copy_tree(tmp_path)
    target = root / "plugins" / "grok" / ".rtok-plugin-version"
    target.write_text('{"schema":1,"plugin":"grok","version":"0.0.2"}\n')

    result = _run(root, "--check", CURRENT_VERSION)

    assert result.returncode == 1
    assert "plugins/grok/.rtok-plugin-version" in result.stdout


def test_check_fails_after_editing_one_manifest_and_names_it(tmp_path):
    root = _copy_tree(tmp_path)
    target = root / "plugins" / "kimi" / "kimi.plugin.json"
    target.write_text(
        target.read_text().replace(f'"version": "{CURRENT_VERSION}"', '"version": "0.0.2"')
    )

    result = _run(root, "--check", CURRENT_VERSION)

    assert result.returncode == 1
    assert "plugins/kimi/kimi.plugin.json" in result.stdout


def test_set_fixes_a_broken_version_file_and_check_passes_again(tmp_path):
    root = _copy_tree(tmp_path)
    target = root / "plugins" / "grok" / ".rtok-plugin-version"
    target.write_text('{"schema":1,"plugin":"grok","version":"0.0.2"}\n')
    assert _run(root, "--check", CURRENT_VERSION).returncode == 1

    set_result = _run(root, "--set", CURRENT_VERSION)
    assert set_result.returncode == 0, set_result.stdout + set_result.stderr

    assert _run(root, "--check", CURRENT_VERSION).returncode == 0
    assert target.read_text() == f'{{"schema":1,"plugin":"grok","version":"{CURRENT_VERSION}"}}\n'


def test_set_preserves_manifest_formatting_outside_the_version_line(tmp_path):
    root = _copy_tree(tmp_path)
    manifest = root / "plugins" / "kimi" / "kimi.plugin.json"
    # Force an actual change: seed a different version than the one --set will write.
    manifest.write_text(
        manifest.read_text().replace(f'"version": "{CURRENT_VERSION}"', '"version": "0.0.1"')
    )
    before = manifest.read_text().splitlines()

    result = _run(root, "--set", CURRENT_VERSION)
    assert result.returncode == 0, result.stdout + result.stderr

    after = manifest.read_text().splitlines()
    assert len(before) == len(after)
    diff_lines = [i for i, (b, a) in enumerate(zip(before, after)) if b != a]
    assert len(diff_lines) == 1  # only the "version" line changes
    assert f'"version": "{CURRENT_VERSION}"' in after[diff_lines[0]]


def test_invalid_args_exit_nonzero():
    assert _run(REPO_ROOT, "--set").returncode != 0
    assert _run(REPO_ROOT, "--frobnicate", "1.2.3").returncode != 0
    assert _run(REPO_ROOT, "--set", "not-semver").returncode != 0


def test_files_lists_every_version_file_and_manifest():
    result = _run(REPO_ROOT, "--files")
    assert result.returncode == 0, result.stdout + result.stderr
    lines = result.stdout.splitlines()

    version_hosts = [
        "claude",
        "codebuddy",
        "codex",
        "copilot",
        "cursor",
        "devin",
        "gemini",
        "grok",
        "kimi",
        "opencode",
        "pi",
        "qwen",
        "zcode",
    ]
    for host in version_hosts:
        assert f"plugins/{host}/.rtok-plugin-version" in lines

    manifests = [
        "plugins/claude/.claude-plugin/plugin.json",
        "plugins/codebuddy/.codebuddy-plugin/plugin.json",
        "plugins/codex/.codex-plugin/plugin.json",
        "plugins/cursor/plugin.json",
        "plugins/cursor/.cursor-plugin/plugin.json",
        "plugins/copilot/plugin.json",
        "plugins/gemini/gemini-extension.json",
        "plugins/qwen/qwen-extension.json",
        "plugins/devin/.devin-plugin/plugin.json",
        "plugins/zcode/.zcode-plugin/plugin.json",
        "plugins/grok/.grok-plugin/plugin.json",
        "plugins/kimi/kimi.plugin.json",
        "plugins/pi/package.json",
    ]
    for manifest in manifests:
        assert manifest in lines

    assert len(lines) == len(version_hosts) + len(manifests)
