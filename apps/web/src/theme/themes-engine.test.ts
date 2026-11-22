import { afterEach, describe, expect, it } from "vitest";
import {
  OBSIDIAN_THEME,
  PAPER_THEME,
  SAVED_THEMES_STORAGE_KEY,
  THEME_STORAGE_KEY,
  deleteCustomTheme,
  loadSavedCustomThemes,
  parseThemeJson,
  readableForeground,
  saveCustomTheme,
  themeToCssVars,
  validateThemeDocument,
} from "./themes";

describe("web theme engine (caper.theme.v1)", () => {
  afterEach(() => {
    if (typeof window !== "undefined") {
      window.localStorage.removeItem(THEME_STORAGE_KEY);
      window.localStorage.removeItem(SAVED_THEMES_STORAGE_KEY);
    }
  });

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

  it("picks readable foregrounds and persists/deletes saved custom themes", () => {
    expect(readableForeground("#ffffff")).toBe("#0b0b0c");
    expect(readableForeground("#111114")).toBe("#ffffff");

    if (typeof window !== "undefined") {
      const saved = saveCustomTheme({
        ...OBSIDIAN_THEME,
        id: "custom-draft",
        name: "Midnight Forest",
        accent: { ...OBSIDIAN_THEME.accent, primary: "#4f8a42", bright: "#66ab57" },
      });
      expect(saved.id).toBe("midnight-forest");
      expect(loadSavedCustomThemes().map((t) => t.id)).toContain("midnight-forest");

      deleteCustomTheme("midnight-forest");
      expect(loadSavedCustomThemes().map((t) => t.id)).not.toContain("midnight-forest");
    }
  });
});
