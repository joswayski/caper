import { useEffect, useState } from "react";
import { BUILTIN_THEMES, loadSavedTheme, selectPresetTheme, subscribeTheme, type CaperThemeDocument } from "./themes";
import "./appearance.css";

export default function AppearanceSettings({ compact = false }: { compact?: boolean }) {
  const [active, setActive] = useState<CaperThemeDocument>(() => loadSavedTheme());

  useEffect(() => {
    const sync = () => setActive(loadSavedTheme());
    sync();
    return subscribeTheme(sync);
  }, []);

  const activePresetId = active.id === "paper" ? "paper" : "obsidian";

  return (
    <section className="appearance-settings" data-compact={compact ? "" : undefined} aria-label="Appearance">
      {!compact && <p className="appearance-eyebrow">Appearance</p>}
      <h2 className="appearance-heading">Theme</h2>
      <p className="appearance-copy">Choose between Obsidian Dark and Paper Light.</p>

      <div className="appearance-presets" role="group" aria-label="Themes">
        {BUILTIN_THEMES.map((preset) => {
          const selected = activePresetId === preset.id;
          return (
            <button
              key={preset.id}
              type="button"
              className="appearance-card"
              aria-pressed={selected}
              onClick={() => selectPresetTheme(preset.id as "obsidian" | "paper")}
            >
              <div className="appearance-swatch" style={{ background: preset.surfaces.frame }} aria-hidden="true">
                <div className="appearance-swatch-sidebar" style={{ background: preset.surfaces.sidebar }}>
                  <span className="appearance-swatch-bar" style={{ background: preset.brand.caper, width: "75%" }} />
                  <span className="appearance-swatch-bar" style={{ background: preset.text.muted, width: "55%" }} />
                  <span className="appearance-swatch-bar" style={{ background: preset.text.muted, width: "65%" }} />
                </div>
                <div className="appearance-swatch-canvas" style={{ background: preset.surfaces.conversation }}>
                  <div className="appearance-swatch-lines">
                    <span className="appearance-swatch-bar" style={{ background: preset.text.primary, width: "68%" }} />
                    <span className="appearance-swatch-bar" style={{ background: preset.text.muted, width: "48%" }} />
                  </div>
                  <span className="appearance-swatch-pill" style={{ background: preset.action.primary }} />
                </div>
              </div>
              <div className="appearance-card-meta">
                <strong>{preset.name}</strong>
                <span>{preset.appearance === "dark" ? "Dark" : "Light"}</span>
              </div>
            </button>
          );
        })}
      </div>
    </section>
  );
}
