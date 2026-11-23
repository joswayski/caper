import obsidianPreset from "../../../../shared/themes/obsidian.json";
import paperPreset from "../../../../shared/themes/paper.json";

export interface CaperThemeDocument {
  schema: "caper.theme.v1";
  id: string;
  name: string;
  appearance: "dark" | "light";
  surfaces: {
    frame: string;
    base: string;
    sidebar: string;
    sidebarRaised: string;
    conversation: string;
    raised: string;
    composer: string;
    replyDraft: string;
    hover: string;
    hoverAlpha: number;
  };
  borders: {
    default: string;
    translucent: string;
    subtle: string;
    rim: string;
  };
  text: {
    primary: string;
    message: string;
    muted: string;
    link: string;
    linkHover: string;
  };
  action: {
    primary: string;
    primaryHover: string;
    primaryPress: string;
    onPrimary: string;
  };
  accent: {
    primary: string;
    bright: string;
    softText: string;
    surface: string;
    border: string;
  };
  brand: {
    caper: string;
    caperBright: string;
    resizeHighlight: string;
  };
  status: {
    voice: string;
    pin: string;
    pinWash: string;
    idle: string;
    errorText: string;
    danger: string;
    dangerHover: string;
    dangerWash: string;
    onDanger: string;
    voiceOff: string;
    voiceOffIcon: string;
    voiceOffWash: string;
  };
  radii: {
    canvas: number;
    card: number;
    composer: number;
    control: number;
    pill: number;
    avatar: string;
  };
}

export const THEME_STORAGE_KEY = "caper.theme.v1";
export const OBSIDIAN_THEME = obsidianPreset as CaperThemeDocument;
export const PAPER_THEME = paperPreset as CaperThemeDocument;
export const BUILTIN_THEMES: readonly CaperThemeDocument[] = [OBSIDIAN_THEME, PAPER_THEME];

const HEX_RE = /^#[0-9a-fA-F]{6}$/;

function hexToRgbTuple(hex: string): [number, number, number] {
  const n = Number.parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function relativeLuminance(hex: string): number {
  const [r, g, b] = hexToRgbTuple(hex).map((v) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrastRatio(a: string, b: string): number {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  const [hi, lo] = la > lb ? [la, lb] : [lb, la];
  return (hi + 0.05) / (lo + 0.05);
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function validateThemeDocument(
  raw: unknown,
): { ok: true; theme: CaperThemeDocument } | { ok: false; error: string } {
  if (!isObject(raw)) return { ok: false, error: "Theme must be a JSON object." };
  if (raw.schema !== "caper.theme.v1") return { ok: false, error: 'Theme schema must be "caper.theme.v1".' };
  if (typeof raw.id !== "string" || !/^[a-z0-9][a-z0-9-]{0,47}$/.test(raw.id)) {
    return { ok: false, error: "Theme id must be 1-48 lowercase letters, digits, or hyphens." };
  }
  if (typeof raw.name !== "string" || !raw.name.trim() || raw.name.length > 48) {
    return { ok: false, error: "Theme name must be 1-48 characters." };
  }
  if (raw.appearance !== "dark" && raw.appearance !== "light") {
    return { ok: false, error: 'Theme appearance must be "dark" or "light".' };
  }

  const base = raw.appearance === "light" ? PAPER_THEME : OBSIDIAN_THEME;
  const pickHex = (section: Record<string, unknown> | undefined, key: string, fallback: string, label: string) => {
    const val = section?.[key];
    if (val === undefined) return fallback;
    if (typeof val !== "string" || !HEX_RE.test(val)) {
      throw new Error(`${label} must be a 6-digit hex color like #131316.`);
    }
    return val.toLowerCase();
  };
  const pickCss = (section: Record<string, unknown> | undefined, key: string, fallback: string, label: string) => {
    const val = section?.[key];
    if (val === undefined) return fallback;
    if (typeof val !== "string" || !val.trim() || /[;{}]/.test(val)) {
      throw new Error(`${label} must be a valid CSS color value.`);
    }
    return val.trim();
  };
  const pickNumber = (
    section: Record<string, unknown> | undefined,
    key: string,
    fallback: number,
    min: number,
    max: number,
    label: string,
  ) => {
    const val = section?.[key];
    if (val === undefined) return fallback;
    if (typeof val !== "number" || !Number.isFinite(val) || val < min || val > max) {
      throw new Error(`${label} must be a number between ${min} and ${max}.`);
    }
    return val;
  };

  try {
    const surfaces = isObject(raw.surfaces) ? raw.surfaces : undefined;
    const borders = isObject(raw.borders) ? raw.borders : undefined;
    const text = isObject(raw.text) ? raw.text : undefined;
    const action = isObject(raw.action) ? raw.action : undefined;
    const accent = isObject(raw.accent) ? raw.accent : undefined;
    const brand = isObject(raw.brand) ? raw.brand : undefined;
    const status = isObject(raw.status) ? raw.status : undefined;
    const radii = isObject(raw.radii) ? raw.radii : undefined;

    const theme: CaperThemeDocument = {
      schema: "caper.theme.v1",
      id: raw.id,
      name: raw.name.trim(),
      appearance: raw.appearance,
      surfaces: {
        frame: pickHex(surfaces, "frame", base.surfaces.frame, "surfaces.frame"),
        base: pickHex(surfaces, "base", base.surfaces.base, "surfaces.base"),
        sidebar: pickHex(surfaces, "sidebar", base.surfaces.sidebar, "surfaces.sidebar"),
        sidebarRaised: pickHex(surfaces, "sidebarRaised", base.surfaces.sidebarRaised, "surfaces.sidebarRaised"),
        conversation: pickHex(surfaces, "conversation", base.surfaces.conversation, "surfaces.conversation"),
        raised: pickHex(surfaces, "raised", base.surfaces.raised, "surfaces.raised"),
        composer: pickHex(surfaces, "composer", base.surfaces.composer, "surfaces.composer"),
        replyDraft: pickHex(surfaces, "replyDraft", base.surfaces.replyDraft, "surfaces.replyDraft"),
        hover: pickCss(surfaces, "hover", base.surfaces.hover, "surfaces.hover"),
        hoverAlpha: pickNumber(surfaces, "hoverAlpha", base.surfaces.hoverAlpha, 0, 1, "surfaces.hoverAlpha"),
      },
      borders: {
        default: pickHex(borders, "default", base.borders.default, "borders.default"),
        translucent: pickCss(borders, "translucent", base.borders.translucent, "borders.translucent"),
        subtle: pickCss(borders, "subtle", base.borders.subtle, "borders.subtle"),
        rim: pickCss(borders, "rim", base.borders.rim, "borders.rim"),
      },
      text: {
        primary: pickHex(text, "primary", base.text.primary, "text.primary"),
        message: pickHex(text, "message", base.text.message, "text.message"),
        muted: pickHex(text, "muted", base.text.muted, "text.muted"),
        link: pickHex(text, "link", base.text.link, "text.link"),
        linkHover: pickHex(text, "linkHover", base.text.linkHover, "text.linkHover"),
      },
      action: {
        primary: pickHex(action, "primary", base.action.primary, "action.primary"),
        primaryHover: pickHex(action, "primaryHover", base.action.primaryHover, "action.primaryHover"),
        primaryPress: pickHex(action, "primaryPress", base.action.primaryPress, "action.primaryPress"),
        onPrimary: pickHex(action, "onPrimary", base.action.onPrimary, "action.onPrimary"),
      },
      accent: {
        primary: pickHex(accent, "primary", base.accent.primary, "accent.primary"),
        bright: pickHex(accent, "bright", base.accent.bright, "accent.bright"),
        softText: pickHex(accent, "softText", base.accent.softText, "accent.softText"),
        surface: pickHex(accent, "surface", base.accent.surface, "accent.surface"),
        border: pickHex(accent, "border", base.accent.border, "accent.border"),
      },
      brand: {
        caper: pickHex(brand, "caper", base.brand.caper, "brand.caper"),
        caperBright: pickHex(brand, "caperBright", base.brand.caperBright, "brand.caperBright"),
        resizeHighlight: pickHex(brand, "resizeHighlight", base.brand.resizeHighlight, "brand.resizeHighlight"),
      },
      status: {
        voice: pickHex(status, "voice", base.status.voice, "status.voice"),
        pin: pickHex(status, "pin", base.status.pin, "status.pin"),
        pinWash: pickCss(status, "pinWash", base.status.pinWash, "status.pinWash"),
        idle: pickHex(status, "idle", base.status.idle, "status.idle"),
        errorText: pickHex(status, "errorText", base.status.errorText, "status.errorText"),
        danger: pickHex(status, "danger", base.status.danger, "status.danger"),
        dangerHover: pickHex(status, "dangerHover", base.status.dangerHover, "status.dangerHover"),
        dangerWash: pickCss(status, "dangerWash", base.status.dangerWash, "status.dangerWash"),
        onDanger: pickHex(status, "onDanger", base.status.onDanger, "status.onDanger"),
        voiceOff: pickHex(status, "voiceOff", base.status.voiceOff, "status.voiceOff"),
        voiceOffIcon: pickHex(status, "voiceOffIcon", base.status.voiceOffIcon, "status.voiceOffIcon"),
        voiceOffWash: pickCss(status, "voiceOffWash", base.status.voiceOffWash, "status.voiceOffWash"),
      },
      radii: {
        canvas: pickNumber(radii, "canvas", base.radii.canvas, 0, 32, "radii.canvas"),
        card: pickNumber(radii, "card", base.radii.card, 0, 32, "radii.card"),
        composer: pickNumber(radii, "composer", base.radii.composer, 0, 32, "radii.composer"),
        control: pickNumber(radii, "control", base.radii.control, 0, 24, "radii.control"),
        pill: pickNumber(radii, "pill", base.radii.pill, 0, 999, "radii.pill"),
        avatar: pickCss(radii, "avatar", base.radii.avatar, "radii.avatar"),
      },
    };

    const bodyRatio = contrastRatio(theme.text.primary, theme.surfaces.conversation);
    if (bodyRatio < 4.5) {
      return {
        ok: false,
        error: `text.primary on surfaces.conversation has low contrast (${bodyRatio.toFixed(2)}:1; needs at least 4.5:1).`,
      };
    }
    const actionRatio = contrastRatio(theme.action.onPrimary, theme.action.primary);
    if (actionRatio < 4.5) {
      return {
        ok: false,
        error: `action.onPrimary on action.primary has low contrast (${actionRatio.toFixed(2)}:1; needs at least 4.5:1).`,
      };
    }

    return { ok: true, theme };
  } catch (error) {
    return { ok: false, error: error instanceof Error ? error.message : "Invalid theme JSON." };
  }
}

export function parseThemeJson(text: string): { ok: true; theme: CaperThemeDocument } | { ok: false; error: string } {
  try {
    const parsed: unknown = JSON.parse(text);
    return validateThemeDocument(parsed);
  } catch {
    return { ok: false, error: "Could not parse JSON. Check syntax and try again." };
  }
}

export function themeToCssVars(theme: CaperThemeDocument): Record<string, string> {
  const accentRgb = hexToRgbTuple(theme.accent.primary).join(" ");
  const accentBrightRgb = hexToRgbTuple(theme.accent.bright).join(" ");
  return {
    "--blackout": theme.surfaces.frame,
    "--surface": theme.surfaces.base,
    "--surface-raised": theme.surfaces.raised,
    "--surface-sidebar": theme.surfaces.sidebar,
    "--surface-conversation": theme.surfaces.conversation,
    "--surface-composer": theme.surfaces.composer,
    "--surface-hover": theme.surfaces.hover,
    "--border": theme.borders.translucent,
    "--border-solid": theme.borders.default,
    "--border-subtle": theme.borders.subtle,
    "--surface-rim": theme.borders.rim,
    "--text": theme.text.primary,
    "--text-message": theme.text.message,
    "--text-muted": theme.text.muted,
    "--link": theme.text.link,
    "--link-hover": theme.text.linkHover,
    "--accent": theme.accent.primary,
    "--accent-bright": theme.accent.bright,
    "--accent-soft-text": theme.accent.softText,
    "--accent-surface": theme.accent.surface,
    "--accent-border": theme.accent.border,
    "--accent-rgb": accentRgb,
    "--accent-bright-rgb": accentBrightRgb,
    "--action": theme.action.primary,
    "--action-hover": theme.action.primaryHover,
    "--action-press": theme.action.primaryPress,
    "--on-action": theme.action.onPrimary,
    "--caper": theme.brand.caper,
    "--caper-bright": theme.brand.caperBright,
    "--voice-session-green": theme.status.voice,
    "--pin-gold": theme.status.pin,
    "--pin-wash": theme.status.pinWash,
    "--error-text": theme.status.errorText,
    "--danger": theme.status.danger,
    "--danger-hover": theme.status.dangerHover,
    "--danger-wash": theme.status.dangerWash,
    "--on-danger": theme.status.onDanger,
    "--voice-off": theme.status.voiceOff,
    "--voice-off-wash": theme.status.voiceOffWash,
    "--radius": `${theme.radii.control}px`,
    "--radius-canvas": `${theme.radii.canvas}px`,
    "--radius-card": `${theme.radii.card}px`,
    "--radius-composer": `${theme.radii.composer}px`,
    "--radius-pill": `${theme.radii.pill}px`,
    "--radius-avatar": theme.radii.avatar,
  };
}

interface StoredTheme {
  preset?: "obsidian" | "paper";
  custom?: CaperThemeDocument;
}

const listeners = new Set<() => void>();

export function subscribeTheme(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function notifyThemeChanged() {
  for (const listener of listeners) listener();
}

export function loadSavedTheme(): CaperThemeDocument {
  if (typeof window === "undefined") return OBSIDIAN_THEME;
  try {
    const raw = window.localStorage.getItem(THEME_STORAGE_KEY);
    if (!raw) return OBSIDIAN_THEME;
    const stored = JSON.parse(raw) as StoredTheme;
    if (stored.preset === "paper") return PAPER_THEME;
    if (stored.preset === "obsidian") return OBSIDIAN_THEME;
    if (stored.custom?.appearance === "light") return PAPER_THEME;
  } catch {
    // Ignore corrupt local storage and fall back to Obsidian.
  }
  return OBSIDIAN_THEME;
}

export function applyTheme(theme: CaperThemeDocument) {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  root.dataset.appearance = theme.appearance;
  root.dataset.themeId = theme.id;
  const vars = themeToCssVars(theme);
  for (const [key, value] of Object.entries(vars)) {
    root.style.setProperty(key, value);
  }
  const metaTheme = document.querySelector<HTMLMetaElement>('meta[name="theme-color"]');
  if (metaTheme) metaTheme.content = theme.surfaces.frame;
  const metaScheme = document.querySelector<HTMLMetaElement>('meta[name="color-scheme"]');
  if (metaScheme) metaScheme.content = theme.appearance;
}

export function selectPresetTheme(preset: "obsidian" | "paper") {
  const theme = preset === "paper" ? PAPER_THEME : OBSIDIAN_THEME;
  if (typeof window !== "undefined") {
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify({ preset } satisfies StoredTheme));
    } catch {
      // Ignore storage quota errors.
    }
  }
  applyTheme(theme);
  notifyThemeChanged();
}

export function selectCustomTheme(theme: CaperThemeDocument) {
  if (typeof window !== "undefined") {
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, JSON.stringify({ custom: theme } satisfies StoredTheme));
    } catch {
      // Ignore storage quota errors.
    }
  }
  applyTheme(theme);
  notifyThemeChanged();
}

export function isBuiltinTheme(theme: CaperThemeDocument): boolean {
  return theme.id === "obsidian" || theme.id === "paper";
}

export const SAVED_THEMES_STORAGE_KEY = "caper.customThemes.v1";

export function readableForeground(hex: string): "#0b0b0c" | "#ffffff" {
  const [r, g, b] = hexToRgbTuple(hex);
  const luma = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
  return luma > 0.56 ? "#0b0b0c" : "#ffffff";
}

export function loadSavedCustomThemes(): CaperThemeDocument[] {
  if (typeof window === "undefined") return [];
  try {
    const raw = window.localStorage.getItem(SAVED_THEMES_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const out: CaperThemeDocument[] = [];
    for (const item of parsed) {
      const checked = validateThemeDocument(item);
      if (checked.ok && !isBuiltinTheme(checked.theme)) out.push(checked.theme);
    }
    return out;
  } catch {
    return [];
  }
}

export function saveCustomTheme(draft: CaperThemeDocument): CaperThemeDocument {
  const cleanName = draft.name.trim().slice(0, 48) || "Custom Theme";
  const slug = cleanName
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  const id =
    !draft.id || draft.id === "obsidian" || draft.id === "paper" || draft.id === "custom-draft"
      ? !slug || slug === "obsidian" || slug === "paper"
        ? `custom-${Date.now()}`
        : slug
      : draft.id;
  const saved: CaperThemeDocument = { ...draft, id, name: cleanName };
  const existing = loadSavedCustomThemes().filter((item) => item.id !== id);
  const next = [...existing, saved];
  if (typeof window !== "undefined") {
    try {
      window.localStorage.setItem(SAVED_THEMES_STORAGE_KEY, JSON.stringify(next));
    } catch {
      // Ignore storage quota errors.
    }
  }
  selectCustomTheme(saved);
  return saved;
}

export function deleteCustomTheme(id: string) {
  const next = loadSavedCustomThemes().filter((item) => item.id !== id);
  if (typeof window !== "undefined") {
    try {
      window.localStorage.setItem(SAVED_THEMES_STORAGE_KEY, JSON.stringify(next));
    } catch {
      // Ignore storage quota errors.
    }
  }
  if (loadSavedTheme().id === id) {
    selectPresetTheme("obsidian");
  } else {
    notifyThemeChanged();
  }
}

/**
 * Tiny inline bootstrap script for __root.tsx so saved Paper Light paints
 * before first frame with zero dark flash.
 */
export const themeInitScript = `try{var r=localStorage.getItem(${JSON.stringify(THEME_STORAGE_KEY)});if(r){var d=JSON.parse(r),p=${JSON.stringify(themeToCssVars(PAPER_THEME))},e=document.documentElement;if(d.preset==="paper"||(d.custom&&d.custom.appearance==="light")){e.dataset.appearance="light";e.dataset.themeId="paper";for(var k in p)e.style.setProperty(k,p[k]);}}}catch{}`;
