import { describe, expect, it } from "vitest";
import { OBSIDIAN_THEME, PAPER_THEME, parseThemeJson, themeToCssVars, validateThemeDocument } from "./themes";

describe("web theme engine (caper.theme.v1)", () => {
  it("validates built-in Obsidian and Paper presets and maps CSS variables", () => {
    expect(validateThemeDocument(OBSIDIAN_THEME).ok).toBe(true);
    expect(validateThemeDocument(PAPER_THEME).ok).toBe(true);

    const paperVars = themeToCssVars(PAPER_THEME);
    expect(paperVars["--blackout"]).toBe("#efeff2");
    expect(paperVars["--surface-conversation"]).toBe("#ffffff");
    expect(paperVars["--action"]).toBe("#111114");
    expect(paperVars["--on-action"]).toBe("#ffffff");
  });

  it("accepts a custom caper.theme.v1 document with partial overrides", () => {
    const custom = parseThemeJson(
      JSON.stringify({
        schema: "caper.theme.v1",
        id: "forest-night",
        name: "Forest Night",
        appearance: "dark",
        surfaces: { frame: "#070b08", conversation: "#101712" },
        accent: { primary: "#5a8f49" },
      }),
    );
    expect(custom.ok).toBe(true);
    if (custom.ok) {
      expect(custom.theme.name).toBe("Forest Night");
      expect(custom.theme.surfaces.conversation).toBe("#101712");
      expect(custom.theme.text.primary).toBe(OBSIDIAN_THEME.text.primary);
    }
  });

  it("rejects invalid schemas, malformed hexes, and low-contrast themes", () => {
    expect(parseThemeJson("{bad json").ok).toBe(false);
    expect(validateThemeDocument({ schema: "other" }).ok).toBe(false);
    expect(
      validateThemeDocument({
        schema: "caper.theme.v1",
        id: "broken",
        name: "Broken",
        appearance: "dark",
        surfaces: { conversation: "#121214" },
        text: { primary: "#1a1a1d" },
      }).ok,
    ).toBe(false);
  });
});
