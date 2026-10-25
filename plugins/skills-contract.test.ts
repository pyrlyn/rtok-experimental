// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

// `rtok skills --json` as a TS plugin consumes it: the shapes of openapi/skills.yaml.
// Needs a built binary in RTOK_BIN (e.g. target/debug/rtok); skipped otherwise.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const bin = process.env.RTOK_BIN;
const agents: Record<string, string> = {
    claude: ".claude/skills",
    cursor: ".cursor/skills",
    copilot: ".copilot/skills",
};

test("the contract names every field the plugins read", () => {
    const spec = readFileSync(join(__dirname, "..", "openapi", "skills.yaml"), "utf8");
    for (const field of ["size_bytes", "enabled", "scope", "changed", "dry_run"]) {
        expect(spec).toContain(field);
    }
});

for (const [agent, dir] of Object.entries(agents)) {
    test.skipIf(!bin)(`${agent}: list, disable, enable round trip`, () => {
        const home = mkdtempSync(join(tmpdir(), `rtok-skills-${agent}-`));
        try {
            const skill = join(home, dir, "demo");
            mkdirSync(skill, { recursive: true });
            mkdirSync(join(home, "proj"));
            writeFileSync(join(skill, "SKILL.md"), "---\nname: demo\ndescription: d\n---\n");
            const env = { ...process.env, HOME: home, RTOK_HOME: join(home, ".rtok"), RTOK_AGENT: agent };
            const run = (...args: string[]) =>
                JSON.parse(
                    execFileSync(bin!, ["skills", ...args, "--json", "--project", join(home, "proj")], { env }).toString(),
                );
            const list = run("list");
            expect(list.agent).toBe(agent);
            expect(list.skills[0]).toMatchObject({ name: "demo", enabled: true, scope: "user", size_bytes: 34 });
            expect(run("disable", "demo").changed).toBe(true);
            expect(existsSync(join(home, `${dir}-disabled`, "demo", "SKILL.md"))).toBe(true);
            expect(run("enable", "demo").skill.enabled).toBe(true);
            expect(run("enable", "demo").changed).toBe(false);
        } finally {
            rmSync(home, { recursive: true, force: true });
        }
    });
}
