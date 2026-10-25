#!/usr/bin/env python3
# Copyright (c) 2026 Ivan Tugay
# SPDX-License-Identifier: GPL-3.0-or-later
"""Evaluate `rtok review` on real merged commits.

For each commit: build the prompt in three context modes (none, digest, full) with
`rtok review --print-prompt`, count tokens with tiktoken `o200k_base`, and, unless
`--tokens-only`, run the reviewer model in each mode with `rtok review --json`.
Results go to `results/<sha>/` (prompts, reviews) and `results/tokens.json`.

Run from the repo root:
    uv run --with tiktoken bench/review-eval/eval.py [--tokens-only] SHA...
Needs a built `rtok` (env RTOK, default `rtok` on PATH) and, for reviews, the provider key
`rtok review` reads (ANTHROPIC_API_KEY or OPENAI_API_KEY). Keys are never printed.
"""

import argparse
import json
import os
import subprocess
import time
from pathlib import Path

import tiktoken

MODES = ["none", "digest", "full"]
OUT = Path(__file__).parent / "results"


def rtok(*args: str) -> str:
    """One `rtok review` call; retried because a busy rtok store answers `database is locked`."""
    exe = os.environ.get("RTOK", "rtok")
    for attempt in range(4):
        p = subprocess.run([exe, "review", *args], capture_output=True, text=True)
        if p.returncode == 0:
            return p.stdout
        if "database is locked" not in p.stderr or attempt == 3:
            raise SystemExit(f"rtok review {' '.join(args)} failed: {p.stderr.strip()[-300:]}")
        time.sleep(2 + attempt * 3)
    raise AssertionError("unreachable")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("shas", nargs="+")
    ap.add_argument("--tokens-only", action="store_true")
    ap.add_argument("--model")
    ap.add_argument("--provider")
    ap.add_argument("--base-url")
    ap.add_argument("--review-modes", default="none,digest,full", help="modes sent to the model")
    ap.add_argument("--max-prompt", type=int, help="skip a review whose prompt exceeds this many tokens")
    ap.add_argument("--tag", default="", help="suffix for review files, e.g. the model name")
    a = ap.parse_args()
    enc = tiktoken.get_encoding("o200k_base")
    table = []
    for sha in a.shas:
        rng = f"{sha}~1..{sha}"
        d = OUT / sha
        d.mkdir(parents=True, exist_ok=True)
        row = {"sha": sha, "range": rng}
        for mode in MODES:
            prompt = rtok("--range", rng, "--context", mode, "--print-prompt")
            (d / f"prompt-{mode}.md").write_text(prompt)
            row[f"tokens_{mode}"] = len(enc.encode(prompt, disallowed_special=()))
            if a.tokens_only or mode not in a.review_modes.split(","):
                continue
            if a.max_prompt and row[f"tokens_{mode}"] > a.max_prompt:
                row[f"review_{mode}"] = "skipped: prompt exceeds the model context"
                continue
            extra = []
            for flag, val in (("--model", a.model), ("--provider", a.provider), ("--base-url", a.base_url)):
                if val:
                    extra += [flag, val]
            t0 = time.time()
            rep = json.loads(rtok("--range", rng, "--context", mode, "--json", *extra))
            row[f"review_{mode}_secs"] = round(time.time() - t0)
            name = f"review-{mode}{'-' + a.tag if a.tag else ''}.md"
            (d / name).write_text(f"reviewer: {rep['reviewer']}\n\n{rep['review']}\n")
            row[f"review_{mode}"] = name
        row["saving_vs_full_pct"] = round(100 * (1 - row["tokens_digest"] / row["tokens_full"]), 1)
        table.append(row)
        print(json.dumps(row))
    tot = {m: sum(r[f"tokens_{m}"] for r in table) for m in MODES}
    summary = {"rows": table, "total": tot, "total_saving_vs_full_pct": round(100 * (1 - tot["digest"] / tot["full"]), 1)}
    out_name = f"tokens{'-' + a.tag if a.tag else ''}.json" if not a.tokens_only else "tokens.json"
    (OUT / out_name).write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary["total"]), summary["total_saving_vs_full_pct"])


if __name__ == "__main__":
    main()
