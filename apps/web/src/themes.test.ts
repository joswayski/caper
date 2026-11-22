import { execFileSync } from "node:child_process";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(import.meta.dirname, "../../..");

describe("shared theme tokens", () => {
  it("keeps all client theme blocks synced with shared/themes/obsidian.json", () => {
    const output = execFileSync(process.execPath, [resolve(root, "scripts/generate-themes.mjs"), "--check"], {
      cwd: root,
      encoding: "utf8",
    });
    expect(output).toContain("Verified");
  });
});
