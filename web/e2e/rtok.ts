// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { type ChildProcess, spawn, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

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

const freePort = () =>
  new Promise<number>((done, fail) => {
    const probe = createServer();
    probe.once("error", fail);
    probe.listen(0, "127.0.0.1", () => {
      const address = probe.address();
      probe.close(() => {
        if (address && typeof address === "object") done(address.port);
        else fail(new Error("no port"));
      });
    });
  });

/**
 * One real `rtok web` process on a throwaway store. `HOME` and `RTOK_HOME` both point at a
 * fresh temp dir and nothing else of this machine's environment is passed on, so the snapshot
 * never reads the real `~/.claude`, no real agent runs, and a stray `RTOK_WEB_DIST` cannot
 * replace the SPA the binary embeds.
 */
export class Rtok {
  private child: ChildProcess | undefined;

  private constructor(
    readonly home: string,
    readonly port: number,
  ) {}

  static async create(): Promise<Rtok> {
    const home = mkdtempSync(join(tmpdir(), "rtok-spa-e2e-"));
    const rtok = new Rtok(home, await freePort());
    try {
      rtok.seed();
    } catch (e) {
      rmSync(home, { recursive: true, force: true });
      throw e;
    }
    return rtok;
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
    return { PATH: SAFE_PATH, HOME: this.home, RTOK_HOME: this.home };
  }

  private run(args: string[], input: string) {
    const out = spawnSync(BIN, args, {
      cwd: this.home,
      env: this.env(),
      input,
      timeout: SEED_TIMEOUT_MS,
      encoding: "utf8",
    });
    if (out.status !== 0) {
      throw new Error(`rtok ${args.join(" ")} exited ${out.status}: ${out.stderr}`);
    }
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
    const child = spawn(BIN, ["web", "--host", "127.0.0.1", "--port", String(this.port)], {
      cwd: this.home,
      env: this.env(),
      stdio: "ignore",
    });
    this.child = child;
    const exited = new Promise<never>((_, fail) =>
      child.once("exit", (code) => fail(new Error(`rtok web exited early (${code})`))),
    );
    const deadline = Date.now() + HEALTH_TIMEOUT_MS;
    const healthy = (async () => {
      while (Date.now() < deadline) {
        try {
          if ((await fetch(`${this.url}/health`)).ok) return;
        } catch {
          // Not listening yet.
        }
        await new Promise((r) => setTimeout(r, 50));
      }
      throw new Error("rtok web never answered /health");
    })();
    try {
      await Promise.race([healthy, exited]);
    } catch (e) {
      await this.stop();
      throw e;
    }
  }

  /** Stops only the process this object spawned. */
  async stop() {
    const child = this.child;
    this.child = undefined;
    if (!child || child.exitCode !== null || child.signalCode !== null) return;
    const gone = new Promise((r) => child.once("exit", r));
    child.kill("SIGTERM");
    const timer = setTimeout(() => child.kill("SIGKILL"), 5000);
    await gone;
    clearTimeout(timer);
  }

  async dispose() {
    await this.stop();
    rmSync(this.home, { recursive: true, force: true });
  }
}
