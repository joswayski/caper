import { useEffect, useRef, useState } from "react";
import { Download, RotateCcw, Upload } from "lucide-react";
import {
  BUILTIN_THEMES,
  OBSIDIAN_THEME,
  isBuiltinTheme,
  loadSavedTheme,
  parseThemeJson,
  selectCustomTheme,
  selectPresetTheme,
  subscribeTheme,
  type CaperThemeDocument,
} from "./themes";
import "./appearance.css";

export default function AppearanceSettings({ compact = false }: { compact?: boolean }) {
  const [active, setActive] = useState<CaperThemeDocument>(() => loadSavedTheme());
  const [pasteOpen, setPasteOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string>();
  const [status, setStatus] = useState<string>();
  const fileInput = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setActive(loadSavedTheme());
    return subscribeTheme(() => setActive(loadSavedTheme()));
  }, []);

  const applyImportedJson = (raw: string) => {
    const parsed = parseThemeJson(raw);
    if (!parsed.ok) {
      setError(parsed.error);
      setStatus(undefined);
      return;
    }
    setError(undefined);
    selectCustomTheme(parsed.theme);
    setStatus(`Applied "${parsed.theme.name}".`);
    setPasteOpen(false);
    setDraft("");
  };

  const handleFile = (file: File | undefined) => {
    if (!file) return;
    void file.text().then(applyImportedJson, () => setError("Could not read that file."));
  };

  const handleExport = () => {
    const json = `${JSON.stringify(active, null, 2)}\n`;
    const blob = new Blob([json], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `${active.id}.capertheme.json`;
    link.click();
    URL.revokeObjectURL(url);
    setStatus(`Exported ${active.id}.capertheme.json.`);
    setError(undefined);
  };

  const customActive = !isBuiltinTheme(active);

  return (
    <section className="appearance-settings" data-compact={compact ? "" : undefined} aria-label="Appearance">
      {!compact && <p className="appearance-eyebrow">Appearance</p>}
      <h2 className="appearance-heading">Theme</h2>
      <p className="appearance-copy">
        Switch between Obsidian Dark and Paper Light, or import a custom <code>.capertheme.json</code> file.
      </p>

      <div className="appearance-presets" role="group" aria-label="Built-in themes">
        {BUILTIN_THEMES.map((preset) => {
          const selected = active.id === preset.id;
          return (
            <button
              key={preset.id}
              type="button"
              className="appearance-card"
              aria-pressed={selected}
              onClick={() => {
                setError(undefined);
                setStatus(undefined);
                selectPresetTheme(preset.id as "obsidian" | "paper");
              }}
            >
              <div className="appearance-swatch" style={{ background: preset.surfaces.frame }} aria-hidden="true">
                <div className="appearance-swatch-sidebar" style={{ background: preset.surfaces.sidebar }}>
                  <span className="appearance-swatch-bar" style={{ background: preset.brand.caper, width: "75%" }} />
                  <span className="appearance-swatch-bar" style={{ background: preset.text.muted, width: "55%" }} />
                  <span className="appearance-swatch-bar" style={{ background: preset.text.muted, width: "65%" }} />
                </div>
                <div className="appearance-swatch-canvas" style={{ background: preset.surfaces.conversation }}>
                  <span className="appearance-swatch-bar" style={{ background: preset.text.primary, width: "68%" }} />
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

      {customActive && (
        <div className="appearance-custom-badge" role="status">
          <span>
            Custom theme: <strong>{active.name}</strong> ({active.appearance})
          </span>
          <button
            type="button"
            className="appearance-btn"
            onClick={() => {
              setError(undefined);
              setStatus("Reset to Obsidian.");
              selectPresetTheme(OBSIDIAN_THEME.id as "obsidian");
            }}
          >
            <RotateCcw size={13} aria-hidden="true" />
            Reset
          </button>
        </div>
      )}

      <div className="appearance-actions">
        <input
          ref={fileInput}
          type="file"
          accept=".json,.capertheme.json,application/json"
          hidden
          onChange={(event) => {
            handleFile(event.target.files?.[0]);
            event.target.value = "";
          }}
        />
        <button type="button" className="appearance-btn" onClick={() => fileInput.current?.click()}>
          <Upload size={14} aria-hidden="true" />
          Import .capertheme.json
        </button>
        <button
          type="button"
          className="appearance-btn"
          aria-expanded={pasteOpen}
          onClick={() => {
            setPasteOpen((open) => !open);
            setError(undefined);
          }}
        >
          Paste JSON
        </button>
        <button type="button" className="appearance-btn" onClick={handleExport}>
          <Download size={14} aria-hidden="true" />
          Export theme
        </button>
      </div>

      {pasteOpen && (
        <div className="appearance-paste">
          <textarea
            aria-label="Custom theme JSON"
            placeholder='Paste a "caper.theme.v1" JSON object…'
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
          />
          <div className="appearance-actions">
            <button
              type="button"
              className="appearance-btn appearance-btn-primary"
              onClick={() => applyImportedJson(draft)}
            >
              Apply custom theme
            </button>
            <button
              type="button"
              className="appearance-btn"
              onClick={() => setDraft(`${JSON.stringify(active, null, 2)}\n`)}
            >
              Load current JSON
            </button>
          </div>
        </div>
      )}

      {error && (
        <p className="appearance-error" role="alert">
          {error}
        </p>
      )}
      {status && (
        <p className="appearance-status" role="status">
          {status}
        </p>
      )}
    </section>
  );
}
