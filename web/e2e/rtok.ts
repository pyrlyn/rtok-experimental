// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

import { type ChildProcess, spawn, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

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
    const reply = this.mcp("mem_save", { title: "fixture", body, kind: "note" });
    if (!reply.includes('"isError":false')) throw new Error("fixture mem_save failed");
  }

  /** One tool call through `rtok mcp` on stdio, the way another process of the same store would make it. */
  mcp(name: string, args: Record<string, unknown>) {
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
      {
        jsonrpc: "2.0",
        id: 2,
        method: "tools/call",
        params: {
          name,
          arguments: args,
        },
      },
    ];
    return this.run(["mcp"], rpc.map((m) => JSON.stringify(m)).join("\n"));
  }

  /** Registers the throwaway home as a graph project, so the graph page has a picture to light. */
  addProject() {
    return this.run(["graph", "projects", "add", this.home], "");
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
