# The report's PDF renderer (T22.0)

> Since the dependency slim-down, the PDF renderer is behind the non-default `pdf` feature:
> `cargo build --features pdf`. See [dependencies](dependencies.md).

`rtok report --format pdf` needs a PDF renderer linked into the one binary (D1). This page is
the D15 survey the task asks for: three candidates priced on what they do to the release
binary that P17 gates, whether they keep "one static binary, no runtime dependency" true, and
whether the HTML and the PDF come from one document (D24) rather than two layouts. Every size
below was measured on this machine — the scratch rows on 2026-09-09, under `/tmp`, without
touching the repo's `Cargo.toml`; the linked row on 2026-09-10, from worktrees of this repo.

## Problem

The report has one content source — the D23 operator model — and three renderings: Markdown,
HTML, PDF. Markdown and HTML cost nothing to link. A PDF renderer is the only candidate that
can add real bytes to a binary the P17 gate exists to keep small (its recorded baseline: release
`rtok` 19 157 712 B (18.27 MiB), 2026-09-04, `done.md` P17 and `research.md` §2; dist build
17.4 MB, 2026-09-07, `research.md` §2 "Where a hook's milliseconds go"). P17 published no byte
cap; it published the numbers and the goal — "what a user downloads stops growing with the
dependency list". So the question is not only which renderer wins, it is what the winner's
delta is, stated with the command that produced it.

## Alternatives

| Candidate | Version | Date | Gets right | Gets wrong |
|-----------|---------|------|------------|------------|
| `typst` as a library (+`typst-pdf`, `typst-layout`, `typst-assets`) | 0.15.1 | 2026-09-09 | one markup source compiles to paged PDF; HTML export (`typst-html`) exists in the same tree; real line breaking, tables, TOC | measured **+44.35 MiB** on the release binary — 2.3× today's whole `rtok` binary; full typesetting engine (image 0.25.10, comemo, fontdb) for a report of tables and short findings |
| `printpdf` + `svg2pdf` | 0.12.8 / 0.13.0 | 2026-09-09 | measured **+5.99 MiB** release, +5.12 MiB dist-shaped; static; text ops use the PDF viewer's builtin fonts, so no font files ship | second layout by construction: raw `Op` streams, no line breaking — wrapping is rtok's job; `printpdf` 0.12 hard-depends on `azul-layout` 0.0.16 (its text engine), which rtok barely uses; `svg2pdf` needs fonts for SVG `<text>` (`fontdb`, system fonts) |
| rendering through a browser (headless Chrome/Safari, `wkhtmltopdf`, `weasyprint`) | — | 2026-09-09 | perfect layout, HTML and PDF from one source by definition | fails criterion (2): a runtime dependency on a third-party tool spawned per report — not one static binary (D1, D6). Rejected on the criterion; no build, no numbers claimed |

Charts are the same decision. `plotters` 0.3.7 (SVG backend, `ttf`, `all_series`,
`all_elements`) measured **+0.24 MiB** and produces the same SVG the HTML format inlines and
the PDF embeds — one chart, three formats. A JS charting library makes the HTML the only real
format: a paged PDF cannot embed live JS, so the other two formats get screenshots or nothing.

## Measured (2026-09-09, macOS arm64, rustc 1.97.1 via mise)

Every scratch carries `[profile.release] strip = "symbols"` — the line P17 put in rtok's
release profile — so the deltas compare against like. The baseline is a no-dependency hello;
the delta is the honest per-candidate cost. Sizes are `stat -f%z` bytes.

| Scratch | Binary | Bytes | Delta vs hello | Command |
|---------|--------|------:|---------------:|---------|
| `/tmp/rtok-t22-hello` | baseline | 341 488 | — | `mise exec -- cargo build --release --manifest-path /tmp/rtok-t22-hello/Cargo.toml` |
| `/tmp/rtok-t22-typst` | typst 0.15.1 + typst-layout + typst-pdf + typst-assets 0.15.1 (fonts) | 44 696 240 | **+44 354 752 B (+42.30 MiB)** | `mise exec -- cargo build --release` in the scratch (1 m 04 s wall) |
| `/tmp/rtok-t22-printpdf` | printpdf 0.12.8 + svg2pdf 0.13.0 (+usvg 0.45.1, azul-layout 0.0.16) | 6 619 104 | **+6 277 616 B (+5.99 MiB)** | `CARGO_TARGET_DIR=/tmp/rtok-t22-target-printpdf mise exec -- cargo build --release --manifest-path …` |
| `/tmp/rtok-t22-printpdf` | same, `[profile.dist]` (thin LTO, 1 cgu) | 5 711 904 | **+5 370 416 B (+5.12 MiB)** | same with `--profile dist` (2 m 28 s) |
| `/tmp/rtok-t22-plotters` | plotters 0.3.7, SVG backend | 591 136 | +249 648 B (+0.24 MiB) | `CARGO_TARGET_DIR=/tmp/rtok-t22-target-plotters mise exec -- cargo build --release --manifest-path …` |

Each scratch binary was run, not just linked: the typst one compiled a one-page report with a
table (`report.pdf`, 22 236 B, 17 embedded fonts); the printpdf one wrote a text page
(`report-printpdf.pdf`, 1 415 B) and an svg2pdf chart (`chart-svg2pdf.pdf`, 13 550 B); the
plotters one wrote a line chart (`savings.svg`, 15 896 B). Of typst's delta, ~9.6 MiB is the
`typst-assets` font payload (`du -sk`, 9 780 KB) — embedded precisely so the binary stays
self-contained; a fonts-from-system build would be smaller and would give the runtime
dependency back.

## Choice

**`printpdf` + `svg2pdf` for the PDF; `plotters` SVG for charts.** It is the only candidate
that keeps P17's number in sight: +5.99 MiB release / +5.12 MiB dist-shaped, measured. The
mechanism: `src/report/pdf.rs` walks the same D23 model the other renderings walk and lays it
out with printpdf's low-level `Op` stream (builtin Helvetica, A4 pages); the chart SVG
plotters produced — the same bytes the HTML inlines — goes through `svg2pdf::to_pdf` into the
page. The section set, the numbers and their order come from the model, so the PDF cannot
disagree with the HTML; what is per-format is layout only (D24).

On criterion (3), typst looks like the winner and is not, once T22.2's HTML is fixed: the HTML
format is one self-contained file generated from the model (inline CSS, inline SVG, openable
from `file://`). Routing it through typst's HTML export would make every format pay typst's
42 MiB; keeping T22.2's HTML means model→HTML and model→typst→PDF are still two layouts. The
one thing both formats genuinely share — the chart SVG — is shared under the choice made here.

## Rejected

- **typst as a library** — the measured cost. +42.30 MiB over a binary that is 18.27 MiB
  today; the projection below puts a dist build near 62 MB, ~3.5×, which fails the P17 clause
  under any reading of its goal. Criterion (3) does not rescue it (above).
- **Rendering through a browser** — criterion (2): a headless browser is a runtime dependency
  and a third-party tool on the report path (D1, D6). Rejected without a build; no size claim
  is made for it.
- **JS charting library** — makes the HTML the only real format and the PDF second-class; a
  second language inlined into a self-contained file for bytes none of the other formats can
  use.

## Budget arithmetic

Measured, cited from the record: P17's release baseline 19 157 712 B (18.27 MiB, 2026-09-04)
and dist binary 17.4 MB (2026-09-07). Measured here: the winner's deltas in the table above.
**P17 published no byte cap**, so the Check's "stays inside the P17 budget" is applied as
arithmetic on the recorded numbers, published here, with the real linked measurement due when
T22.3 links the renderer:

- dist projected: 17.4 MB + 5.37 MB (5.12 MiB delta) ≈ **22.8 MB, +31 %** over the recorded
  dist binary — the same size class P17 published, for the one format that cannot exist
  without a layout engine in the binary.
- release projected: 19 157 712 + 6 277 616 = 25 435 328 B (24.26 MiB), +32.8 %.
- typst, for contrast (arithmetic, not built with dist): 17.4 MB + 44.35 MB ≈ **61.8 MB**,
  ~3.5×. Outside the budget under any reading; this is why the smaller candidate wins.

**Linked, measured 2026-09-10** (macOS arm64, rustc 1.97.1 via mise, release profile with the
repo's `strip = "symbols"`, isolated target dirs): pre-T22.3 `bcc5da0` 21 089 200 B
(20.11 MiB) → T22.3 `ba4c90a` 23 262 832 B (22.19 MiB; built at `df68bf4` — the two commits
above `ba4c90a` touch `.gitignore`/`plan.md`/`roadmap.md` only) — delta **+2 173 632 B
(+2.07 MiB, +10.3 %)**, 2 172 496 B under the 25 435 328 B projection. Commands:
`git worktree add --detach /tmp/rtok-t223-base bcc5da0`, then in each tree
`CARGO_TARGET_DIR=$PWD/target mise exec -- cargo build --release`; sizes
`stat -f%z target/release/rtok`. The delta lands materially under the scratch's +5.99 MiB,
which fires this page's first falsification clause as a re-price: the scratch priced
printpdf + svg2pdf + usvg against a no-dep hello, the landed tree links printpdf only with
its hard `azul-layout` dep (`svg2pdf`/`usvg` are absent from `Cargo.lock`; the charts are
drawn natively, `done.md` T22.3), and rtok's real feature set shares more of that tree than
a hello baseline does. The gate holds on the measured numbers — +10.3 % over the pre-report
binary, same size class — and the release binary renders the empty-store PDF (25 954 B,
`%PDF-1.3`…`%%EOF`) end-to-end.

If a cap is ever written into the gate, +31 % is the number it has to be checked against —
and the fallback is not typst, it is dropping paged PDF or trimming printpdf's
`azul-layout` dependency (printpdf 0.8 predates it and would likely cost less; unmeasured
here, noted as the lever).

## Falsified by

- T22.3's linked build measuring materially outside +5.0–6.3 MiB over the pre-report binary
  (rtok's real feature set can pull different paths than a scratch). Then re-price, or drop
  paged PDF. Fired 2026-09-10, below the band (+2.07 MiB): re-priced in `Budget
  arithmetic`, paged PDF kept.
- A report requirement printpdf cannot meet without rtok growing its own line breaker or
  shaper (CJK text, unbroken strings wider than the page). Then the honest answer is a
  degraded PDF (truncated cell, landscape table), not 42 MiB of typesetting engine.
- svg2pdf's `fontdb` system-font dependence breaking chart text on a bare Linux runner. Then
  one font is embedded into `fontdb` and its bytes are measured at T22.3, or chart text is
  dropped for numbers outside the chart.
- A trimmed, fonts-optional typst build measured inside the winner's delta — then criterion
  (3) flips the decision back to typst.

## Recommendations (T22.5)

Rule **top-sinks** (T59.8) ranks the ten largest token sinks from `Measurement.before_bytes` — file paths for `read`, command stems for `cmd`, `server/tool` for MCP wrap — and names the rtok switch that would shorten each (`[plugins.read] default_mode`, a `[stem]` rule, or `rtok mcp --`).
