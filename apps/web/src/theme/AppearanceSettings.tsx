import { useEffect, useRef, useState } from "react";
import { Download, RotateCcw, Save, Trash2, Upload } from "lucide-react";
import {
  BUILTIN_THEMES,
  OBSIDIAN_THEME,
  PAPER_THEME,
  deleteCustomTheme,
  isBuiltinTheme,
  loadSavedCustomThemes,
  loadSavedTheme,
  parseThemeJson,
  readableForeground,
  saveCustomTheme,
  selectCustomTheme,
  selectPresetTheme,
  subscribeTheme,
  validateThemeDocument,
  type CaperThemeDocument,
} from "./themes";
import "./appearance.css";

interface SwatchSpec {
  id: string;
  label: string;
  read: (theme: CaperThemeDocument) => string;
  write: (theme: CaperThemeDocument, hex: string) => CaperThemeDocument;
}

const COLOR_SWATCHES: SwatchSpec[] = [
  {
    id: "frame",
    label: "Background",
    read: (theme) => theme.surfaces.frame,
    write: (theme, hex) => ({
      ...theme,
      surfaces: { ...theme.surfaces, frame: hex, base: hex },
    }),
  },
  {
    id: "sidebar",
    label: "Sidebar",
    read: (theme) => theme.surfaces.sidebar,
    write: (theme, hex) => ({
      ...theme,
      surfaces: { ...theme.surfaces, sidebar: hex, sidebarRaised: hex },
    }),
  },
  {
    id: "conversation",
    label: "Conversation",
    read: (theme) => theme.surfaces.conversation,
    write: (theme, hex) => ({
      ...theme,
      surfaces: { ...theme.surfaces, conversation: hex },
    }),
  },
  {
    id: "composer",
    label: "Cards & Input",
    read: (theme) => theme.surfaces.composer,
    write: (theme, hex) => ({
      ...theme,
      surfaces: { ...theme.surfaces, composer: hex, raised: hex },
    }),
  },
  {
    id: "accent",
    label: "Accent",
    read: (theme) => theme.accent.primary,
    write: (theme, hex) => ({
      ...theme,
      accent: {
        ...theme.accent,
        primary: hex,
        bright: hex,
        softText: hex,
        border: hex,
      },
    }),
  },
  {
    id: "action",
    label: "Primary button",
    read: (theme) => theme.action.primary,
    write: (theme, hex) => ({
      ...theme,
      action: {
        primary: hex,
        primaryHover: hex,
        primaryPress: hex,
        onPrimary: readableForeground(hex),
      },
    }),
  },
  {
    id: "text",
    label: "Text",
    read: (theme) => theme.text.primary,
    write: (theme, hex) => ({
      ...theme,
      text: { ...theme.text, primary: hex, message: hex },
    }),
  },
  {
    id: "border",
    label: "Borders",
    read: (theme) => theme.borders.default,
    write: (theme, hex) => ({
      ...theme,
      borders: { ...theme.borders, default: hex, translucent: hex },
    }),
  },
];

export default function AppearanceSettings({ compact = false }: { compact?: boolean }) {
  const [active, setActive] = useState<CaperThemeDocument>(() => loadSavedTheme());
  const [savedThemes, setSavedThemes] = useState<CaperThemeDocument[]>(() => loadSavedCustomThemes());
  const [themeName, setThemeName] = useState<string>(() => {
    const current = loadSavedTheme();
    return isBuiltinTheme(current) ? "My Custom Theme" : current.name;
  });
  const [pasteOpen, setPasteOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string>();
  const [status, setStatus] = useState<string>();
  const fileInput = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const sync = () => {
      const current = loadSavedTheme();
      setActive(current);
      setSavedThemes(loadSavedCustomThemes());
      if (!isBuiltinTheme(current)) {
        setThemeName(current.name);
      }
    };
    sync();
    return subscribeTheme(sync);
  }, []);

  const applyImportedJson = (raw: string) => {
    const parsed = parseThemeJson(raw);
    if (!parsed.ok) {
      setError(parsed.error);
      setStatus(undefined);
      return;
    }
    setError(undefined);
    const saved = saveCustomTheme(parsed.theme);
    setThemeName(saved.name);
    setStatus(`Saved and applied "${saved.name}".`);
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

  const handleSwatchChange = (spec: SwatchSpec, hex: string) => {
    const base = isBuiltinTheme(active)
      ? {
          ...active,
          id: "custom-draft",
          name: themeName.trim() || "My Custom Theme",
        }
      : active;
    const updated = spec.write(base, hex);
    setError(undefined);
    setStatus(undefined);
    selectCustomTheme(updated);
  };

  const handleAppearanceToggle = (appearance: "dark" | "light") => {
    if (active.appearance === appearance) return;
    const preset = appearance === "dark" ? OBSIDIAN_THEME : PAPER_THEME;
    const updated: CaperThemeDocument = {
      ...preset,
      id: isBuiltinTheme(active) ? "custom-draft" : active.id,
      name: isBuiltinTheme(active) ? themeName.trim() || "My Custom Theme" : active.name,
      accent: active.accent,
      brand: active.brand,
    };
    setError(undefined);
    setStatus(undefined);
    selectCustomTheme(updated);
  };

  const handleSaveCustom = () => {
    const candidate: CaperThemeDocument = {
      ...active,
      name: themeName.trim() || "My Custom Theme",
      id: isBuiltinTheme(active) ? "custom-draft" : active.id,
    };
    const checked = validateThemeDocument(candidate);
    if (!checked.ok) {
      setError(checked.error);
      setStatus(undefined);
      return;
    }
    const saved = saveCustomTheme(checked.theme);
    setThemeName(saved.name);
    setError(undefined);
    setStatus(`Saved "${saved.name}".`);
  };

  const allThemes: CaperThemeDocument[] = [...BUILTIN_THEMES, ...savedThemes];
  const customActive = !isBuiltinTheme(active);

  return (
    <section className="appearance-settings" data-compact={compact ? "" : undefined} aria-label="Appearance">
      {!compact && <p className="appearance-eyebrow">Appearance</p>}
      <h2 className="appearance-heading">Theme</h2>
      <p className="appearance-copy">
        Switch between Obsidian Dark and Paper Light, customize colors live, or save your own themes.
      </p>

      <div className="appearance-presets" role="group" aria-label="Themes">
        {allThemes.map((preset) => {
          const selected = active.id === preset.id;
          const builtin = isBuiltinTheme(preset);
          return (
            <div key={preset.id} className="appearance-card-wrap">
              <button
                type="button"
                className="appearance-card"
                aria-pressed={selected}
                onClick={() => {
                  setError(undefined);
                  setStatus(undefined);
                  if (builtin) {
                    selectPresetTheme(preset.id as "obsidian" | "paper");
                  } else {
                    selectCustomTheme(preset);
                    setThemeName(preset.name);
                  }
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
              {!builtin && (
                <button
                  type="button"
                  className="appearance-card-delete"
                  aria-label={`Delete ${preset.name}`}
                  title={`Delete ${preset.name}`}
                  onClick={() => {
                    deleteCustomTheme(preset.id);
                    setStatus(`Deleted "${preset.name}".`);
                    setError(undefined);
                  }}
                >
                  <Trash2 size={13} aria-hidden="true" />
                </button>
              )}
            </div>
          );
        })}
      </div>

      <div className="appearance-editor" aria-label="Customize theme colors">
        <div className="appearance-editor-head">
          <div>
            <h3 className="appearance-editor-title">Customize colors</h3>
            <p className="appearance-editor-sub">Changes preview live. Save to keep your custom theme.</p>
          </div>
          <div className="appearance-mode-toggle" role="group" aria-label="Base appearance">
            <button
              type="button"
              className="appearance-mode-btn"
              aria-pressed={active.appearance === "dark"}
              onClick={() => handleAppearanceToggle("dark")}
            >
              Dark
            </button>
            <button
              type="button"
              className="appearance-mode-btn"
              aria-pressed={active.appearance === "light"}
              onClick={() => handleAppearanceToggle("light")}
            >
              Light
            </button>
          </div>
        </div>

        <div className="appearance-name-row">
          <label className="appearance-name-label" htmlFor="caper-theme-name">
            Theme name
          </label>
          <input
            id="caper-theme-name"
            type="text"
            className="appearance-name-input"
            maxLength={48}
            placeholder="My Custom Theme"
            value={themeName}
            onChange={(event) => setThemeName(event.target.value)}
          />
        </div>

        <div className="appearance-color-grid">
          {COLOR_SWATCHES.map((spec) => {
            const value = spec.read(active);
            const inputId = `caper-swatch-${spec.id}`;
            return (
              <label key={spec.id} className="appearance-color-row" htmlFor={inputId}>
                <span className="appearance-color-meta">
                  <span className="appearance-color-label">{spec.label}</span>
                  <code className="appearance-color-hex">{value.toUpperCase()}</code>
                </span>
                <input
                  id={inputId}
                  type="color"
                  className="appearance-color-input"
                  value={value}
                  onChange={(event) => handleSwatchChange(spec, event.target.value)}
                />
              </label>
            );
          })}
        </div>

        <div className="appearance-actions">
          <button type="button" className="appearance-btn appearance-btn-primary" onClick={handleSaveCustom}>
            <Save size={14} aria-hidden="true" />
            Save theme
          </button>
          {customActive && (
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
          )}
        </div>
      </div>

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
