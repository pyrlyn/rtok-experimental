# T931. Done

The card's two shapes already passed through before this change: windows joined by `;`/`&&`
(`sed -n 1,80p a; sed -n 1,40p b`) and a pipe into `sed` (`git show x | sed -n 1,100p`). The
elided chains had a literal label between windows (`sed -n 1,80p a; echo ---; sed -n 1,40p b`),
and the `echo` stage made `bounded::is_bounded` call the whole chain unbounded.

- `src/plugins/cmd/bounded.rs`: a single-stage `echo`/`printf` of literal words is silent like
  `cd`/`export`. Any `$` or backtick in its words (an expansion whose output could be unbounded)
  disqualifies it, and a label alone never bounds a chain.
- Tests: rstest cases in `bounded.rs` for `;`, `&&`, newline and pipe joins and `echo`/`printf`
  labels, plus the cases that must still be cut (`echo ---; cargo test`,
  `sed -n 1,5p a; echo $(cat big)`); `a_chain_of_bounded_sed_windows_passes_through_whole` in
  `formatters.rs` runs `compress` on a 240-line body in the hook's single-argv shape.
- `src/plugins/cmd/AGENTS.md`: the label rule.

PR: the `t931-sed-chain` branch on pyrlyn/rtok.

Check: the `formatters.rs` test failed with the label check disabled (the chain came back as a
rule with "220 lines omitted") and passed with it
(`cargo nextest run --lib -E 'test(a_chain_of_bounded_sed_windows)'`). Locally `cli_trycmd`
failed on `config-init` and `report-md` with one extra config key from a stale `rtok-config`
artifact in the APFS-cloned `target/`; this change touches no config, and CI runs the full gate on
a clean build.
