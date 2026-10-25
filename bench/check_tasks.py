"""Prove the T9.1 checks in bench/tasks.toml are real.

Each check must fail on the untouched fixture (bench/repos/t9) and pass after a reference
edit. The check itself restores the fixture, so the next case starts clean. Run from the repo
root with the fixture committed: python3 bench/check_tasks.py
"""
from __future__ import annotations

import subprocess
import sys
import tomllib
from pathlib import Path

LIB = Path("bench/repos/t9/src/lib.rs")
TEST_MOD = "mod tests {\n    use super::*;\n"


def add_fn(s: str) -> str:
    return s.replace("#[cfg(test)]", "pub fn foo() -> i32 {\n    1\n}\n\n#[cfg(test)]")


def fix_bug(s: str) -> str:
    return s.replace("self.value < n - 1", "self.value < n")


def write_test(s: str) -> str:
    test = '\n    #[test]\n    fn parse_port_rejects_zero() {\n        assert_eq!(parse_port("0"), None);\n    }\n'
    return s.replace(TEST_MOD, TEST_MOD + test)


def rename(s: str) -> str:
    return s.replace("parse_port", "parse_tcp_port")


def explain(_: str) -> None:
    Path("bench/repos/t9/EXPLAIN.md").write_text(
        "A Counter that counts from 1 to n, and parse_port, which parses a TCP port.\n"
    )


REFERENCE = {
    "add-fn": add_fn,
    "fix-bug": fix_bug,
    "write-test": write_test,
    "rename": rename,
    "explain": explain,
    "run-tests": fix_bug,
}


def passes(check: str) -> bool:
    return subprocess.run(["sh", "-c", check], capture_output=True).returncode == 0


def main() -> int:
    if subprocess.run(["git", "diff", "--quiet", "HEAD", "--", "bench/repos/t9"]).returncode:
        print("bench/repos/t9 has uncommitted changes; commit or restore them first")
        return 2
    tasks = tomllib.loads(Path("bench/tasks.toml").read_text())["tasks"]
    bad = 0
    for t in tasks:
        before = passes(t["check"])
        edit = REFERENCE[t["id"]]
        out = edit(LIB.read_text())
        if out is not None:
            LIB.write_text(out)
        after = passes(t["check"])
        ok = not before and after
        bad += not ok
        print(f"{t['id']:<11} untouched={'pass' if before else 'fail'} reference={'pass' if after else 'fail'} {'ok' if ok else 'BAD'}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
