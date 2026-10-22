// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { type ChildProcess, type SpawnSyncReturns, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { spawnGroup, stopGroup } from "./group";

/** Two rotated log generations past `[log] files`: the junk the card offers to clear. */
export const STALE_LOGS = ["rtok.log.7", "rtok.log.8"];

/** First line of the memory body the fixture stores; the expand test looks for it. */
export const ARCHIVED_MARKER = "line 0 NEEDLE-0";

const BIN = process.env["RTOK_BIN"] ?? resolve(import.meta.dirname, "../../target/debug/rtok");
// System tools only (git for the worktrees page): a real `claude` or `codex` on the developer's PATH
// must not be detected, let alone run, by the hosts page.
const SAFE_PATH = "/usr/bin:/bin";
const SEED_TIMEOUT_MS = 30_000;
const HEALTH_TIMEOUT_MS = 20_000;
/** `rtok web`'s startup line on stderr: `rtok web http://<host>:<port>  (ws://…/ws)`. */
const LISTENING = /^rtok web http:\/\/\S+:(\d+)(?:\s|$)/;
/** How much of the server's stderr an early-exit error carries. */
const STDERR_TAIL = 20;
// Not `os.tmpdir()`: on a Mac that per-user dir collects tens of thousands of entries, and under
// this suite's load realpath(3) on a home there stalled for seconds, past the seed timeout (T643).
const HOME_BASE = process.platform === "win32" ? tmpdir() : "/tmp";

/** Why a `spawnSync` run failed, or `undefined` when it exited 0. */
export function failure(what: string, out: SpawnSyncReturns<string>, timeoutMs: number) {
  // A null status is a kill (spawnSync's own timeout included), never an exit code.
  if (out.error || out.signal) {
    const timedOut = (out.error as NodeJS.ErrnoException | undefined)?.code === "ETIMEDOUT";
    const why = timedOut
      ? `timed out after ${timeoutMs / 1000} s`
      : (out.error?.message ?? `killed by ${out.signal}`);
    return `${what} ${why}: ${out.stderr}`;
  }
  if (out.status !== 0) return `${what} exited ${out.status}: ${out.stderr}`;
  return undefined;
}

/** Where `Rtok.create` makes its throwaway home. */
export function makeHome() {
  return mkdtempSync(join(HOME_BASE, "rtok-spa-e2e-"));
}

/**
 * One real `rtok web` process on a throwaway store. `HOME` and `RTOK_HOME` both point at a
 * fresh temp dir and nothing else of this machine's environment is passed on, so the snapshot
 * never reads the real `~/.claude`; `RTOK_HOST_SANDBOX` also isolates absolute desktop app
 * paths. No real agent runs, and a stray `RTOK_WEB_DIST` cannot
 * replace the SPA the binary embeds.
 */
export class Rtok {
  private child: ChildProcess | undefined;
  private bound: number | undefined;

  private constructor(readonly home: string) {}

  static async create(): Promise<Rtok> {
    const home = makeHome();
    const rtok = new Rtok(home);
    try {
      rtok.seed();
    } catch (e) {
      rmSync(home, { recursive: true, force: true });
      throw e;
    }
    return rtok;
  }

  /** The port `rtok web` bound on its first start; a restart binds it again. */
  get port() {
    if (this.bound === undefined) throw new Error("rtok web has not started yet");
    return this.bound;
  }

  /** The `/ws` access token of the running server, read from the file it wrote (S2). */
  get token() {
    return readFileSync(join(this.home, "web-token"), "utf8").trim();
  }

  get url() {
    return `http://127.0.0.1:${this.port}`;
  }

  /** Where a stale log generation lives: beside `rtok.log` in `$RTOK_HOME/logs`. */
  logPath(name: string) {
    return join(this.home, "logs", name);
  }

  get configPath() {
    return join(this.home, "config.toml");
  }

  private env() {
    return { PATH: SAFE_PATH, HOME: this.home, RTOK_HOME: this.home, RTOK_HOST_SANDBOX: "1" };
  }

  private run(args: string[], input: string) {
    const out = spawnSync(BIN, args, {
      cwd: this.home,
      env: this.env(),
      input,
      timeout: SEED_TIMEOUT_MS,
      encoding: "utf8",
    });
    const failed = failure(`rtok ${args.join(" ")}`, out, SEED_TIMEOUT_MS);
    if (failed) throw new Error(failed);
    return out.stdout;
  }

  /**
   * Ledger rows through the product's own entry points: a hook event and an MCP `mem_save`
   * whose request is larger than `core.call_io_inline_bytes`, so the ledger archives it and
   * the calls page gets a call with an archive id to expand.
   */
  private seed() {
    this.run(
      ["hook", "SessionStart"],
      JSON.stringify({ hook_event_name: "SessionStart", session_id: "e2e", cwd: this.home }),
    );
    const body = Array.from({ length: 6000 }, (_, i) => `line ${i} NEEDLE-${i}`).join("\n");
    // Aged past the one-minute settle window, or `clear` would keep them as in use.
    mkdirSync(join(this.home, "logs"), { recursive: true });
    const old = new Date(Date.now() - 2 * 86_400_000);
    for (const name of STALE_LOGS) {
      writeFileSync(this.logPath(name), "stale");
      utimesSync(this.logPath(name), old, old);
    }
    const reply = this.mcp("mem_save", { title: "fixture", body, kind: "note" });
    if (!reply.includes('"isError":false')) throw new Error("fixture mem_save failed");
  }

  /** Tool calls through `rtok mcp` on stdio, the way another process of the same store would make them. */
  mcp(name: string, args: Record<string, unknown>, times = 1) {
    const rpc = [
      {
        jsonrpc: "2.0",
        id: 1,
        method: "initialize",
        params: {
          protocolVersion: "2025-06-18",
          capabilities: {},
          clientInfo: { name: "e2e", version: "0" },
        },
      },
      { jsonrpc: "2.0", method: "notifications/initialized" },
      ...Array.from({ length: times }, (_, i) => ({
        jsonrpc: "2.0",
        id: 2 + i,
        method: "tools/call",
        params: { name, arguments: args },
      })),
    ];
    return this.run(["mcp"], rpc.map((m) => JSON.stringify(m)).join("\n"));
  }

  /**
   * Makes the throwaway home a git checkout with a `[tasks]` section and tasks, through
   * `rtok task`: `rtok web` runs there, so its board reads them. The last task is finished and
   * the first is the parent of the second.
   */
  addTasks(titles: string[]) {
    const git = spawnSync("git", ["init", "-q", "-b", "main"], { cwd: this.home, env: this.env() });
    if (git.status !== 0) throw new Error(`git init: ${git.stderr}`);
    this.run(["config", "set", "tasks.prefix", "E"], "");
    const ids = titles.map((title, i) =>
      this.run(["task", "create", title, ...(i === 1 ? ["--parent", "E1"] : [])], "").trim(),
    );
    this.run(["task", "status", ids[ids.length - 1]!, "done"], "");
    return ids;
  }

  /** Registers the throwaway home as a graph project, so the graph page has a picture to light. */
  addProject() {
    return this.run(["graph", "projects", "add", this.home], "");
  }

  /** A registered git project holding `file` as committed; `editProject` then gives Compare a change. Returns its registry id. */
  addGitProject(file: string, before: string): number {
    const dir = join(this.home, "proj");
    mkdirSync(dir);
    const git = (...args: string[]) => {
      const out = spawnSync("git", args, {
        cwd: dir,
        env: {
          ...this.env(),
          GIT_AUTHOR_NAME: "e2e",
          GIT_AUTHOR_EMAIL: "e2e@example.com",
          GIT_COMMITTER_NAME: "e2e",
          GIT_COMMITTER_EMAIL: "e2e@example.com",
        },
        encoding: "utf8",
      });
      if (out.status !== 0) throw new Error(`git ${args.join(" ")}: ${out.stderr}`);
    };
    git("init", "-q", "-b", "main");
    writeFileSync(join(dir, file), before);
    git("add", "-A");
    git("commit", "-q", "-m", "before");
    this.run(["graph", "projects", "add", dir], "");
    const rows = JSON.parse(this.run(["graph", "projects", "--json"], "")) as {
      id: number;
      root: string;
    }[];
    const row = rows.find((r) => r.root.endsWith("/proj"));
    if (!row) throw new Error("the git project was not registered");
    return row.id;
  }

  /** Every registered project as `graph projects --json` lists it, the CLI's view of the registry. */
  projects() {
    return JSON.parse(this.run(["graph", "projects", "--json"], "")) as {
      id: number;
      root: string;
      state: string;
    }[];
  }

  /** The page draws the index, so a project must be indexed after each edit. */
  indexProject(id: number) {
    this.run(["graph", "index", "--project", String(id)], "");
  }

  editProject(file: string, text: string) {
    writeFileSync(join(this.home, "proj", file), text);
  }

  /** What the page's file picker would be given: a symbols-level export of the project as it is now. */
  exportSymbols(id: number): string {
    return this.run(["graph", "export", "--level", "symbols", "--project", String(id)], "");
  }

  async start() {
    if (this.child) throw new Error("rtok web is already running");
    // Port 0 on the first start: the kernel picks a free port as `rtok web` binds, and the
    // startup line names it. A port probed here and handed over could be taken by a parallel
    // worker in between. A restart asks for the same port, so the open page reconnects.
    const child = spawnGroup(
      BIN,
      ["web", "--host", "127.0.0.1", "--port", String(this.bound ?? 0)],
      {
        cwd: this.home,
        env: this.env(),
        stdio: ["ignore", "ignore", "pipe"],
      },
    );
    this.child = child;
    const tail: string[] = [];
    const listening = new Promise<number>((done) => {
      // Reading every line keeps the pipe drained, so the server never blocks on stderr.
      createInterface({ input: child.stderr! }).on("line", (line) => {
        tail.push(line);
        if (tail.length > STDERR_TAIL) tail.shift();
        const port = LISTENING.exec(line)?.[1];
        if (port) done(Number(port));
      });
    });
    const exited = new Promise<never>((_, fail) =>
      child.once("exit", (code) =>
        fail(new Error(`rtok web exited early (${code}): ${tail.join("\n")}`)),
      ),
    );
    let timer: NodeJS.Timeout | undefined;
    const silent = new Promise<never>((_, fail) => {
      timer = setTimeout(
        () => fail(new Error("rtok web never printed its address")),
        HEALTH_TIMEOUT_MS,
      );
    });
    const deadline = Date.now() + HEALTH_TIMEOUT_MS;
    const healthy = async () => {
      while (Date.now() < deadline) {
        try {
          if ((await fetch(`${this.url}/health`)).ok) return;
        } catch {
          // Not listening yet.
        }
        await new Promise((r) => setTimeout(r, 50));
      }
      throw new Error("rtok web never answered /health");
    };
    try {
      this.bound = await Promise.race([listening, exited, silent]);
      await Promise.race([healthy(), exited]);
    } catch (e) {
      await this.stop();
      throw e;
    } finally {
      clearTimeout(timer);
    }
  }

  /** Stops the process this object spawned and everything it started, and waits for all of them. */
  async stop() {
    const child = this.child;
    this.child = undefined;
    if (child) await stopGroup(child);
  }

  async dispose() {
    await this.stop();
    rmSync(this.home, { recursive: true, force: true });
  }
}
