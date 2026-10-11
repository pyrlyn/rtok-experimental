# Plugin versions and updates

How rtok versions the plugin trees under `plugins/<host>/`, records what it installed, and
decides what `rtok agents update` does with an installed plugin.

**Scope today:** only Claude Code is wired to the version decision. Codex, Copilot and Gemini
(and every other host) carry the version file but keep their previous `agents update`
behaviour; they follow later.

Every example below is a real run of rtok 0.10.0 against a throwaway `HOME` with a stub
`claude` first on `PATH` (no real Claude Code involved). `--cli --no-restart` keep the run to
the Claude Code CLI; each report is trimmed to its header, plugin and error lines.

## Why

Claude Code caches a plugin by the `version` in its manifest. Until rtok 0.10.0 every plugin
manifest said `0.0.1`, so a new rtok build shipped a plugin Claude already had "at that
version" and never picked up. rtok itself had no record of which plugin build was installed or
where it came from, so `agents update` could only reinstall every time or trust the host.

A plugin reaches a machine from one of three sources, and the scheme covers all of them:

- **GitHub**: the host installs from the repository (`claude plugin marketplace add
  pyrlyn/rtok`).
- **Local**: the host installs from a plugin tree on disk (`claude plugin marketplace add
  <path>/plugins/claude`), for development and offline installs.
- **Marketplace**: the host's catalog entry for the committed `.claude-plugin/marketplace.json`.

## The version file

Each versioned plugin tree carries `plugins/<host>/.rtok-plugin-version`, committed, one line of
JSON:

```text
$ cat plugins/claude/.rtok-plugin-version
{"schema":1,"plugin":"claude","version":"0.10.0"}
```

| Field | Meaning |
| --- | --- |
| `schema` | Format version. rtok reads only `1` and rejects any other value with an error naming the file. |
| `plugin` | The host directory name (`claude`, `codex`, …). |
| `version` | SemVer; equals the `Cargo.toml` version of the commit. An invalid version is an error naming the file. |
| `source` | Optional, reserved for a local install (`github`, `local`, `marketplace`). Committed files never carry it. |

Hosts with a file: `claude`, `codebuddy`, `codex`, `copilot`, `cursor`, `devin`, `gemini`, `grok`, `kimi`,
`opencode`, `pi`, `zcode`. `cline` has no manifest of its own and `antigravity`'s manifest has no
version field, so neither has one. The same version is written into every manifest's `version`
field (`plugins/claude/.claude-plugin/plugin.json` and the rest), so a host that caches by
manifest version sees each release as new.

Only the release writes these files ([Releasing](#releasing)). The file sits at the plugin root,
so every source copies it along with the plugin into the host's install (for Claude, its plugin
cache under `~/.claude/plugins/cache/rtok/`). rtok does not yet read the file back from the
installed copy, nor write a `+g<sha>` copy into it: for a local build the build string lives in
the receipt only.

## The receipt `plugins.json`

rtok keeps one row per host of what it installed (only `claude` writes one today):

| OS | Path |
| --- | --- |
| macOS | `~/Library/Application Support/rtok/plugins.json` |
| Linux | `$XDG_STATE_HOME/rtok/plugins.json`, else `~/.local/state/rtok/plugins.json` |
| Windows | `%LOCALAPPDATA%\rtok\plugins.json`, else `%USERPROFILE%\AppData\Local\rtok\plugins.json` |

After an update from GitHub (home path shortened to `~`):

```json
{
  "claude": {
    "source": "github",
    "ref": "v0.10.0",
    "marketplace": "rtok",
    "path": "~/.claude/plugins/installed_plugins.json",
    "version": "0.10.0",
    "installed_at": "2026-09-27T20:17:52Z"
  }
}
```

| Field | Meaning |
| --- | --- |
| `source` | `github`, `local` or `marketplace`. |
| `ref` | `v<version>` for GitHub and marketplace; the local plugin tree's path for a local install. |
| `marketplace` | `rtok` for GitHub and marketplace; absent for local. |
| `path` | The host record the install is visible in (for Claude, `installed_plugins.json`). |
| `version` | The version installed; a local install adds `+g<sha>` or `+g<sha>.dirty`. |
| `installed_at` | UTC, RFC 3339, second precision. |

A local install's row, from a checkout with uncommitted changes (checkout path shortened):

```json
    "source": "local",
    "ref": "<checkout>/plugins/claude",
    "version": "0.10.0+g84a4b607.dirty",
```

The row is written after a successful install, reinstall or in-place update, and left alone on
`--dry-run` and on any `claude` failure that removed nothing. It is deleted when a reinstall has
already uninstalled the old plugin and the install then fails ([The decision](#the-decision)).
A missing file reads as "no rows".

## Sources

For Claude, `agents update` picks the source in this order: `--source`, else the receipt row's
`source`, else GitHub.

| Source | `rtok` marketplace points at | Available version |
| --- | --- | --- |
| GitHub | `pyrlyn/rtok` | the running rtok's own version |
| Marketplace | `pyrlyn/rtok` | the running rtok's own version |
| Local | the plugin tree rtok resolves (`plugins/claude` beside the binary, in a `share/rtok` prefix, in the ketch store, or in the source checkout) | that tree's `.rtok-plugin-version` plus `+g<sha>[.dirty]` from `git describe --always --dirty` |

GitHub and marketplace need no network call: the release keeps every version file equal to
`Cargo.toml`, so the tag `v<version>` matching the binary carries exactly that version. Only a
local tree can differ from the binary, so it alone is read from disk. GitHub and marketplace
differ only in the recorded source today.

The installed version comes from the receipt row, else from Claude's own record
(`~/.claude/plugins/installed_plugins.json`, `plugins["rtok@rtok"][0].version`) when that is
SemVer, else `0.0.0`. Claude counts as having the plugin when that file lists `rtok@rtok`.

The `rtok` entry in `~/.claude/plugins/known_marketplaces.json` is checked against the chosen
source: `{"source":"github","repo":"pyrlyn/rtok"}` for GitHub and marketplace,
`{"source":"directory","path":"<local tree>"}` for local. Any other value (such as a pre-0.10
ketch store path) is stale and forces a reinstall that re-points it.

The repository moved from the `listepo` account to the `pyrlyn` organization. A marketplace
added before the move still records `{"source":"github","repo":"listepo/rtok"}` for Claude, or
`source = "https://github.com/listepo/rtok.git"` under `[marketplaces.rtok]` for Codex. Both
are stale under the rule above. The next `rtok agents install` or `agents update` removes the
`rtok` marketplace, adds it again from `pyrlyn/rtok` and reinstalls `rtok@rtok` once. The
marketplace and plugin names stay `rtok`, so `rtok@rtok` keeps meaning the same plugin. To move
by hand instead: `claude plugin marketplace remove rtok && claude plugin marketplace add
pyrlyn/rtok && claude plugin install rtok@rtok` (Codex: the same with `codex plugin`, and
`plugin add` for the last step).

## The decision

One pure function (`decide` in `src/agents/plugin_version.rs`) compares the installed and the
available version by SemVer precedence, which ignores build metadata, then compares the build
metadata separately. Rows are checked top to bottom; the first match wins.

| Case | Result | `claude` calls |
| --- | --- | --- |
| Plugin not installed | install | `plugin marketplace add` (when not current), `plugin install rtok@rtok` |
| `--force` | reinstall | `plugin uninstall`, then as above |
| Source changed, or marketplace entry stale | reinstall | `plugin uninstall`, `plugin marketplace remove rtok` and `add` (when not current), `plugin install` |
| Available newer | update in place | `plugin marketplace update rtok`, `plugin update rtok@rtok` |
| Same base version, different build metadata | update in place | as above |
| Legacy: no receipt, host record `0.0.1` or not SemVer (`0.0.0`) | update in place (older than any release) | as above |
| Equal version and build metadata | skip | none |
| Available older | skip, warn | none |

If the in-place update fails, rtok falls back to a reinstall and appends the error.

A legacy install updates once and writes the receipt; the rerun calls no `claude` command:

```text
$ rtok agents update claude --cli --no-restart
CLI: Claude Code
~ plugin rtok@rtok updated to 0.10.0
$ rtok agents update claude --cli --no-restart
CLI: Claude Code — already current
```

`--dry-run` shows the decision and writes nothing. An update or a skip names the version; an
install or reinstall prints the `claude` commands it would run:

```text
$ rtok agents update claude --cli --no-restart --dry-run
CLI: Claude Code — dry run, nothing written
plugin rtok@rtok 0.10.0 up to date (github)
$ rtok agents update claude --cli --no-restart --dry-run --force
CLI: Claude Code — dry run, nothing written
offer plugins/claude → claude plugin uninstall rtok@rtok && claude plugin install rtok@rtok ketch install pyrlyn/rtok
```

An older available version (receipt at `0.11.0`, rtok at `0.10.0`) is left in place:

```text
$ rtok agents update claude --cli --no-restart
CLI: Claude Code
plugin rtok@rtok: available 0.10.0 is older than installed 0.11.0
```

`--force` reinstalls whatever the versions say, downgrades included, and rewrites the receipt.
`--source github|local|marketplace` compares against that source instead of the recorded one;
a different source is a reinstall, and the receipt switches to it. After that, a new local
commit or an edit to a tracked file is an in-place update (the second run below came after an
edit):

```text
$ rtok agents update claude --cli --no-restart --source local
CLI: Claude Code
+ plugin plugins/claude → rtok@rtok
$ rtok agents update claude --cli --no-restart
CLI: Claude Code
~ plugin rtok@rtok updated to 0.10.0+g84a4b607.dirty
```

**When a reinstall fails.** A `claude` failure before anything was removed keeps the old plugin
and the receipt, prints an `offer … (claude failed: …)` line and exits 0. If the uninstall
succeeded and the install failed, the host has no plugin: rtok says so, deletes the receipt row
and exits non-zero. The next `agents update` sees nothing installed and installs:

```text
$ rtok agents update claude --cli --no-restart --force
CLI: Claude Code
plugin rtok@rtok removed, reinstall failed: install failed
  ✗ plugin  not installed
  warning: plugin did not read back as installed
Error: a plugin reinstall failed
$ rtok agents update claude --cli --no-restart
CLI: Claude Code
+ plugin plugins/claude → rtok@rtok
```

## Listing outdated plugins

`rtok agents outdated` lists the hosts whose rtok plugin is older than the running rtok, and
only those. `rtok agents update --check` prints exactly the same thing, for anyone who looks
under `update`. It reads local files only: no network, no host CLI call and no marketplace
refresh, so it is fast and works offline. The target is always the running binary's own
version (`rtok --version`).

```console
$ rtok agents outdated
agent  installed available source
claude 0.0.1     0.14.0    github

run: rtok agents update claude
```

**What is listed.** Every host rtok supports is checked (the same registry as `agents list`),
not only the ones in the receipt, so a plugin installed by hand or by an older rtok is found
too. A host variant is a row when the plugin is installed there and its version is lower than
the running rtok by SemVer precedence. Build metadata is ignored: a local `0.14.0+g12c7e91`
on rtok `0.14.0` is current. The installed version is looked up in the same order as
`agents update` ([the decision](#the-decision)): the `.rtok-plugin-version` file in the installed
copy, then the receipt, then the host's own record (Claude's `installed_plugins.json`). The
`source` column comes from the same lookup (`github`, `local`, `marketplace`).

**What is hidden.** Hosts without the plugin, with the same version and with a newer version
are not printed. A host with an older version that is installed but whose version nothing
records (no version file, no receipt row, no usable host record) counts as `0.0.0` and shows as
`legacy`; a host whose record does say a version, like `0.0.1` above, shows that version.

```console
$ rtok agents outdated
agent  installed available source
claude legacy    0.14.0    github

run: rtok agents update claude
```

**Nothing to do.** Two messages, depending on whether anything is installed:

```console
$ rtok agents outdated
all rtok plugins are up to date (1 installed, rtok 0.14.0)
$ rtok agents outdated gemini
no rtok plugins installed
```

**Selecting hosts.** Like `update`: an optional comma-separated host list
(`rtok agents outdated claude,cursor`), and `--cli` / `--desktop` for one variant.

**`--json`** prints one object and no human message, also when there is nothing to update
(`outdated` is then empty, and `installed` counts the plugins that were checked):

```console
$ rtok agents outdated --json
{"rtok":"0.14.0","outdated":[{"agent":"claude","variant":"cli","installed":"0.0.1","available":"0.14.0","source":"github","legacy":false}],"installed":1}
```

| Field | Meaning |
| --- | --- |
| `rtok` | The running rtok version, the one every row is compared with. |
| `outdated[].agent`, `.variant` | Host id and variant (`cli` or `desktop`). |
| `outdated[].installed` | The installed version, or `legacy` when nothing records one. |
| `outdated[].available` | The running rtok version. |
| `outdated[].source` | `github`, `local` or `marketplace`. |
| `outdated[].legacy` | `true` for a `legacy` row. |
| `installed` | How many plugin installs were checked, outdated or not. |

**`--exit-code`** exits 10 when at least one host is outdated and 0 otherwise. Without it the
exit code is 0 either way, so a script that only reads the output keeps working:

```console
$ rtok agents outdated --json --exit-code; echo "exit=$?"
{"rtok":"0.14.0","outdated":[{"agent":"claude","variant":"cli","installed":"0.0.1","available":"0.14.0","source":"github","legacy":false}],"installed":1}
exit=10
```

**Why it works offline.** The available version is the running binary's own: the tag or
catalog entry that matches this build carries the same `.rtok-plugin-version`
(`tools/plugin-versions.sh --check` keeps it so), so there is nothing to ask a server. Only a
local checkout can differ from the binary, and `update` reads that; `outdated` does not.
The command never changes anything: to act on the list, run the `run:` line it prints.

## Installed version in `agents list`

The `plugin` row of `rtok agents list` and `rtok agents info <host>` names the installed
version and its source, read with the same lookup as `agents outdated`. A version that
differs from the running rtok says so, and an older one names the command that fixes it; an
install no file dates shows `legacy`. `rtok web` shows the same text on the Hosts page, with an
older plugin marked `outdated`.

```console
$ rtok agents list
CLI: Claude Code
  ✓ plugin  installed 0.14.0 (marketplace), rtok is 0.15.1 — rtok agents update claude
$ rtok agents list
CLI: Claude Code
  ✓ plugin  installed (legacy, no version)
```

With `--json`, each host variant carries `plugin_version` and `plugin_source`; both are
absent when no plugin is installed or no version is recorded.

## Releasing

`tools/plugin-versions.sh` is the one place that writes and checks every version file and
manifest:

```text
$ tools/plugin-versions.sh --set 0.10.1
$ cat plugins/claude/.rtok-plugin-version
{"schema":1,"plugin":"claude","version":"0.10.1"}
$ tools/plugin-versions.sh --check 0.10.1
$ echo $?
0
```

`--check <version>` prints each file that differs and exits 1; `--files` prints every file the
script touches.

- `tools/release.sh` (run by `just release` and by `bump.yml`) calls `--set` in the same
  `release: v<version>` commit that raises `Cargo.toml` and `Cargo.lock`, and stages the files
  `--files` lists.
- `ci.yml` runs `--check` against the `Cargo.toml` version on every non-draft pull request and
  every push to `main`.
- `release.yml` runs `--check` against the tag, `v` stripped, before building; a mismatch fails
  the release.
- `tests/plugin_versions.rs` asserts every file `--files` lists equals `CARGO_PKG_VERSION`, so
  `just check` catches drift locally.

Nothing else raises the version: release-plz no longer runs in CI, and the Bump workflow's
pull request carries the `release.sh` commit, so the `ci.yml` check runs on it before the merge
and the `release.yml` check stops a tag that slipped through anyway. See
[Releasing rtok](release.md) for the release flow itself.

## Troubleshooting

**The plugin stays old after `agents update`.** Look at the decision first:

```text
$ rtok agents update claude --cli --no-restart --dry-run
CLI: Claude Code — dry run, nothing written
~ plugin rtok@rtok → 0.10.0 (claude plugin marketplace update rtok && claude plugin update rtok@rtok)
```

If the dry run wants an update but a real run says `already current`, `claude` is not on
`PATH`: the update is skipped and the receipt is left as it was. Put `claude` on `PATH` and run
again. If the dry run says `up to date` but the copy Claude runs is old, the receipt is wrong
(rtok trusts it over the installed copy): `rtok agents update claude --force`.

**Legacy install.** An install from before rtok 0.10.0 has no receipt and a host record of
`0.0.1` (or no SemVer at all). The first `rtok agents update claude` updates it once and writes
the receipt; nothing else is needed. If that update and its reinstall fallback both fail after
the uninstall, see "When a reinstall fails" above: rerun `rtok agents update claude`.

**Version mismatch in CI.** The `plugin-versions.sh --check` step or `tests/plugin_versions.rs`
fails when a manifest or version file was edited by hand:

```text
$ tools/plugin-versions.sh --check 0.10.1
plugins/gemini/gemini-extension.json: 0.0.2 (want 0.10.1)
$ echo $?
1
```

Rewrite every file from `Cargo.toml` and commit the result:

```bash
tools/plugin-versions.sh --set "$(grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2)"
```

**`--source local` fails with `git describe`.** The local source reads its build metadata from
git, so it needs the plugin tree inside a git checkout; an installed rtok's `plugins/` is not
one. Run the rtok binary built from the checkout, or use `--source github`.
