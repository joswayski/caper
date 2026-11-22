// Sync Caper's shared theme tokens (shared/themes/obsidian.json) into every
// client's native token block so Web, Apple, Android, Windows, Linux, and
// desktop stay in lockstep. Use --check to fail on drift without writing.
import assert from "node:assert/strict";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const themesDir = `${root}shared/themes/`;
const checkOnly = process.argv.includes("--check");

function parseHex(hex, label) {
  assert.match(hex, /^#[0-9a-f]{6}$/i, `Invalid hex color for ${label}: ${hex}`);
  return {
    r: Number.parseInt(hex.slice(1, 3), 16),
    g: Number.parseInt(hex.slice(3, 5), 16),
    b: Number.parseInt(hex.slice(5, 7), 16),
    upper: hex.slice(1).toUpperCase(),
    lower: hex.toLowerCase(),
  };
}

function relativeLuminance({ r, g, b }) {
  const chan = (c) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * chan(r) + 0.7152 * chan(g) + 0.0722 * chan(b);
}

function contrastRatio(hexA, hexB) {
  const l1 = relativeLuminance(parseHex(hexA, "contrastA"));
  const l2 = relativeLuminance(parseHex(hexB, "contrastB"));
  return (Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05);
}

function validateTheme(theme, filename) {
  assert.equal(theme.schema, "caper.theme.v1", `${filename}: schema must be caper.theme.v1`);
  assert.match(theme.id, /^[a-z0-9-]+$/, `${filename}: invalid id`);
  assert.ok(theme.appearance === "dark" || theme.appearance === "light", `${filename}: invalid appearance`);

  const hexChecks = [
    ["surfaces.frame", theme.surfaces.frame],
    ["surfaces.base", theme.surfaces.base],
    ["surfaces.sidebar", theme.surfaces.sidebar],
    ["surfaces.sidebarRaised", theme.surfaces.sidebarRaised],
    ["surfaces.conversation", theme.surfaces.conversation],
    ["surfaces.raised", theme.surfaces.raised],
    ["surfaces.composer", theme.surfaces.composer],
    ["surfaces.replyDraft", theme.surfaces.replyDraft],
    ["borders.default", theme.borders.default],
    ["text.primary", theme.text.primary],
    ["text.message", theme.text.message],
    ["text.muted", theme.text.muted],
    ["text.link", theme.text.link],
    ["text.linkHover", theme.text.linkHover],
    ["action.primary", theme.action.primary],
    ["action.primaryHover", theme.action.primaryHover],
    ["action.primaryPress", theme.action.primaryPress],
    ["action.onPrimary", theme.action.onPrimary],
    ["accent.primary", theme.accent.primary],
    ["accent.bright", theme.accent.bright],
    ["accent.softText", theme.accent.softText],
    ["accent.surface", theme.accent.surface],
    ["accent.border", theme.accent.border],
    ["brand.caper", theme.brand.caper],
    ["brand.caperBright", theme.brand.caperBright],
    ["brand.resizeHighlight", theme.brand.resizeHighlight],
    ["status.voice", theme.status.voice],
    ["status.pin", theme.status.pin],
    ["status.idle", theme.status.idle],
    ["status.errorText", theme.status.errorText],
    ["status.danger", theme.status.danger],
    ["status.dangerHover", theme.status.dangerHover],
    ["status.onDanger", theme.status.onDanger],
    ["status.voiceOff", theme.status.voiceOff],
    ["status.voiceOffIcon", theme.status.voiceOffIcon],
  ];
  for (const [label, value] of hexChecks) {
    parseHex(value, `${filename}:${label}`);
  }

  // Enforce readable contrast on primary text, muted text, primary action, and mention badge.
  assert.ok(
    contrastRatio(theme.text.primary, theme.surfaces.conversation) >= 10,
    `${filename}: text.primary contrast too low on surfaces.conversation`,
  );
  assert.ok(
    contrastRatio(theme.text.muted, theme.surfaces.conversation) >= 4.5,
    `${filename}: text.muted contrast too low on surfaces.conversation`,
  );
  assert.ok(
    contrastRatio(theme.action.onPrimary, theme.action.primary) >= 10,
    `${filename}: action.onPrimary contrast too low on action.primary`,
  );
  assert.ok(
    contrastRatio("#ffffff", theme.accent.primary) >= 4.5,
    `${filename}: white label contrast too low on accent.primary`,
  );
}

// Validate every preset in shared/themes/*.json.
const presets = new Map();
for (const entry of readdirSync(themesDir).sort()) {
  if (!entry.endsWith(".json") || entry === "schema.json") continue;
  const theme = JSON.parse(readFileSync(`${themesDir}${entry}`, "utf8"));
  validateTheme(theme, entry);
  presets.set(theme.id, theme);
}

const t = presets.get("obsidian");
assert.ok(t, "Missing default theme shared/themes/obsidian.json");

const rgbTriplet = (hex) => {
  const { r, g, b } = parseHex(hex, hex);
  return `${r} ${g} ${b}`;
};
const rustHex = (hex) => `0x${parseHex(hex, hex).upper}`;
const rustRgb = (hex) => {
  const { r, g, b } = parseHex(hex, hex);
  return `Color32::from_rgb(${r}, ${g}, ${b})`;
};
const kotlinColor = (hex) => `Color(0xFF${parseHex(hex, hex).upper})`;
const kotlinAlphaColor = (hex, alphaHex) => `Color(0x${alphaHex}${parseHex(hex, hex).upper})`;
const swiftColor = (hex) => {
  const { r, g, b } = parseHex(hex, hex);
  return `Color(red: ${r}/255, green: ${g}/255, blue: ${b}/255)`;
};
const gtkRgba = (cssRgb) => {
  const m = cssRgb.match(/^rgb\((\d+)\s+(\d+)\s+(\d+)\s*\/\s*(\d+)%\)$/);
  assert.ok(m, `Expected rgb(r g b / p%) format, got: ${cssRgb}`);
  return `rgba(${m[1]}, ${m[2]}, ${m[3]}, ${Number(m[4]) / 100})`;
};

function replaceGeneratedBlock(relPath, startMarker, endMarker, replacement) {
  const fullPath = `${root}${relPath}`;
  const current = readFileSync(fullPath, "utf8");
  const startIdx = current.indexOf(startMarker);
  const endIdx = current.indexOf(endMarker);
  assert.ok(startIdx !== -1 && endIdx !== -1 && endIdx > startIdx, `Missing generated markers in ${relPath}`);
  const next = `${current.slice(0, startIdx + startMarker.length)}\n${replacement}\n${current.slice(endIdx)}`;
  if (checkOnly) {
    assert.equal(current, next, `Theme token drift in ${relPath}. Run: node scripts/generate-themes.mjs`);
  } else if (current !== next) {
    writeFileSync(fullPath, next);
  }
}

// 1. shared/design.css
replaceGeneratedBlock(
  "shared/design.css",
  "  /* BEGIN GENERATED THEME TOKENS */",
  "  /* END GENERATED THEME TOKENS */",
  [
    `  --blackout: ${t.surfaces.frame};`,
    `  --surface: ${t.surfaces.base};`,
    `  --surface-raised: ${t.surfaces.raised};`,
    `  --surface-sidebar: ${t.surfaces.sidebar};`,
    `  --surface-conversation: ${t.surfaces.conversation};`,
    `  --surface-composer: ${t.surfaces.composer};`,
    `  --surface-hover: ${t.surfaces.hover};`,
    `  --border: ${t.borders.translucent};`,
    `  --border-solid: ${t.borders.default};`,
    `  --border-subtle: ${t.borders.subtle};`,
    `  --surface-rim: ${t.borders.rim};`,
    `  --text: ${t.text.primary};`,
    `  --text-message: ${t.text.message};`,
    `  --text-muted: ${t.text.muted};`,
    `  --link: ${t.text.link};`,
    `  --link-hover: ${t.text.linkHover};`,
    `  --accent: ${t.accent.primary};`,
    `  --accent-bright: ${t.accent.bright};`,
    `  --accent-soft-text: ${t.accent.softText};`,
    `  --accent-surface: ${t.accent.surface};`,
    `  --accent-border: ${t.accent.border};`,
    `  --accent-rgb: ${rgbTriplet(t.accent.primary)};`,
    `  --accent-bright-rgb: ${rgbTriplet(t.accent.bright)};`,
    `  /* Legacy aliases kept so existing rules resolve through the active theme. */`,
    `  --terracotta: var(--accent);`,
    `  --terracotta-bright: var(--accent-bright);`,
    `  --action: ${t.action.primary};`,
    `  --action-hover: ${t.action.primaryHover};`,
    `  --action-press: ${t.action.primaryPress};`,
    `  --on-action: ${t.action.onPrimary};`,
    `  --caper: ${t.brand.caper};`,
    `  --caper-bright: ${t.brand.caperBright};`,
    `  --voice-session-green: ${t.status.voice};`,
    `  --pin-gold: ${t.status.pin};`,
    `  --pin-wash: ${t.status.pinWash};`,
    `  --error-text: ${t.status.errorText};`,
    `  --danger: ${t.status.danger};`,
    `  --danger-hover: ${t.status.dangerHover};`,
    `  --danger-wash: ${t.status.dangerWash};`,
    `  --on-danger: ${t.status.onDanger};`,
    `  --voice-off: ${t.status.voiceOff};`,
    `  --voice-off-wash: ${t.status.voiceOffWash};`,
    `  --radius: ${t.radii.control}px;`,
    `  --radius-canvas: ${t.radii.canvas}px;`,
    `  --radius-card: ${t.radii.card}px;`,
    `  --radius-composer: ${t.radii.composer}px;`,
    `  --radius-pill: ${t.radii.pill}px;`,
    `  --radius-avatar: ${t.radii.avatar};`,
  ].join("\n"),
);

// 2. apps/native/linux/src/style.css
replaceGeneratedBlock(
  "apps/native/linux/src/style.css",
  "/* BEGIN GENERATED THEME TOKENS */",
  "/* END GENERATED THEME TOKENS */",
  [
    `@define-color caper_blackout ${t.surfaces.frame};`,
    `@define-color caper_surface ${t.surfaces.base};`,
    `@define-color caper_surface_raised ${t.surfaces.raised};`,
    `@define-color caper_surface_sidebar ${t.surfaces.sidebar};`,
    `@define-color caper_surface_conversation ${t.surfaces.conversation};`,
    `@define-color caper_surface_composer ${t.surfaces.composer};`,
    `@define-color caper_surface_hover ${gtkRgba(t.surfaces.hover)};`,
    `@define-color caper_border ${gtkRgba(t.borders.translucent)};`,
    `@define-color caper_border_solid ${t.borders.default};`,
    `@define-color caper_border_subtle ${gtkRgba(t.borders.subtle)};`,
    `@define-color caper_surface_rim ${gtkRgba(t.borders.rim)};`,
    `@define-color caper_text ${t.text.primary};`,
    `@define-color caper_text_muted ${t.text.muted};`,
    `@define-color caper_link ${t.text.link};`,
    `@define-color caper_link_hover ${t.text.linkHover};`,
    `@define-color caper_terracotta ${t.accent.primary};`,
    `@define-color caper_terracotta_bright ${t.accent.bright};`,
    `@define-color caper_accent_soft_text ${t.accent.softText};`,
    `@define-color caper_accent_surface ${t.accent.surface};`,
    `@define-color caper_accent_border ${t.accent.border};`,
    `@define-color caper_action ${t.action.primary};`,
    `@define-color caper_action_hover ${t.action.primaryHover};`,
    `@define-color caper_action_press ${t.action.primaryPress};`,
    `@define-color caper_on_action ${t.action.onPrimary};`,
    `@define-color caper_green ${t.brand.caper};`,
    `@define-color caper_voice_green ${t.status.voice};`,
    `@define-color caper_error_text ${t.status.errorText};`,
    `@define-color caper_danger ${t.status.danger};`,
    `@define-color caper_danger_hover ${t.status.dangerHover};`,
  ].join("\n"),
);

// 3. apps/native/windows/src/theme.rs
replaceGeneratedBlock(
  "apps/native/windows/src/theme.rs",
  "// BEGIN GENERATED THEME TOKENS",
  "// END GENERATED THEME TOKENS",
  [
    `pub const BLACKOUT: u32 = ${rustHex(t.surfaces.frame)};`,
    `pub const SURFACE: u32 = ${rustHex(t.surfaces.base)};`,
    `pub const SURFACE_RAISED: u32 = ${rustHex(t.surfaces.raised)};`,
    `pub const SURFACE_SIDEBAR: u32 = ${rustHex(t.surfaces.sidebar)};`,
    `pub const SURFACE_CONVERSATION: u32 = ${rustHex(t.surfaces.conversation)};`,
    `pub const SURFACE_COMPOSER: u32 = ${rustHex(t.surfaces.composer)};`,
    `pub const BORDER: u32 = ${rustHex(t.borders.default)};`,
    `pub const TEXT: u32 = ${rustHex(t.text.primary)};`,
    `pub const TEXT_MUTED: u32 = ${rustHex(t.text.muted)};`,
    `pub const LINK: u32 = ${rustHex(t.text.link)};`,
    `pub const TERRACOTTA: u32 = ${rustHex(t.accent.primary)};`,
    `/// Hovered fields and controls.`,
    `pub const TERRACOTTA_BRIGHT: u32 = ${rustHex(t.accent.bright)};`,
    `pub const ACCENT_SURFACE: u32 = ${rustHex(t.accent.surface)};`,
    `pub const ACCENT_SOFT_TEXT: u32 = ${rustHex(t.accent.softText)};`,
    `/// Error text, readable on blackout.`,
    `pub const ERROR: u32 = ${rustHex(t.status.errorText)};`,
    `/// Primary actions (\`.primary\`): warm chalk white at rest, brighter on hover, darker pressed.`,
    `pub const ACTION: u32 = ${rustHex(t.action.primary)};`,
    `pub const ACTION_HOVER: u32 = ${rustHex(t.action.primaryHover)};`,
    `pub const ACTION_PRESS: u32 = ${rustHex(t.action.primaryPress)};`,
    `pub const CAPER: u32 = ${rustHex(t.brand.caper)};`,
    `/// Voice under way: live session emerald.`,
    `pub const VOICE_GREEN: u32 = ${rustHex(t.status.voice)};`,
    `/// \`--surface-hover\`: white at 6%.`,
    `pub const HOVER: (u32, f32) = (0xFFFFFF, ${t.surfaces.hoverAlpha});`,
  ].join("\n"),
);

// 4. apps/native/android/app/src/main/java/chat/caper/android/ui/Theme.kt
replaceGeneratedBlock(
  "apps/native/android/app/src/main/java/chat/caper/android/ui/Theme.kt",
  "// BEGIN GENERATED THEME TOKENS",
  "// END GENERATED THEME TOKENS",
  [
    `val Blackout = ${kotlinColor(t.surfaces.frame)}`,
    `val Surface = ${kotlinColor(t.surfaces.base)}`,
    `val Border = ${kotlinColor(t.borders.default)}`,
    `val BorderSubtle = Color(0x17FFFFFF)`,
    `val SurfaceRim = Color(0x14FFFFFF)`,
    `val Text = ${kotlinColor(t.text.primary)}`,
    `val Terracotta = ${kotlinColor(t.accent.primary)}`,
    `/** Primary actions are warm chalk white with obsidian labels (shared/themes/obsidian.json). */`,
    `val Action = ${kotlinColor(t.action.primary)}`,
    `val OnAction = ${kotlinColor(t.action.onPrimary)}`,
    `val CaperGreen = ${kotlinColor(t.brand.caper)}`,
    `// Caper green mixed into Border: replaces a panel divider while its resize handle is in use.`,
    `val ResizeHighlight = ${kotlinColor(t.brand.resizeHighlight)}`,
    `val VoiceSessionGreen = ${kotlinColor(t.status.voice)}`,
    `val SurfaceSidebar = ${kotlinColor(t.surfaces.sidebar)}`,
    `/** Web's --surface-hover (6% white) over the sidebar, opaque: a private channel's own members in the member list. */`,
    `val SurfaceSidebarRaised = ${kotlinColor(t.surfaces.sidebarRaised)}`,
    `val SurfaceConversation = ${kotlinColor(t.surfaces.conversation)}`,
    `val SurfaceComposer = ${kotlinColor(t.surfaces.composer)}`,
    `val SurfaceRaised = ${kotlinColor(t.surfaces.raised)}`,
    `val TextMuted = ${kotlinColor(t.text.muted)}`,
    `val MessageText = ${kotlinColor(t.text.message)}`,
    `val LinkBlue = ${kotlinColor(t.text.link)}`,
    `val LinkBlueActive = ${kotlinColor(t.text.linkHover)}`,
    `val TerracottaBright = ${kotlinColor(t.accent.bright)}`,
    `val TerracottaDark = ${kotlinColor(t.accent.surface)}`,
    `val TerracottaBorder = ${kotlinColor(t.accent.border)}`,
    `val TerracottaWash = ${kotlinAlphaColor(t.accent.primary, "29")}`,
    `/** Text on an accent wash, such as your own reaction's count. */`,
    `val TerracottaLight = ${kotlinColor(t.accent.softText)}`,
    `val PinGold = ${kotlinColor(t.status.pin)}`,
    `val PinGoldWash = ${kotlinAlphaColor(t.status.pin, "0F")}`,
    `val Idle = ${kotlinColor(t.status.idle)}`,
    `val Danger = ${kotlinColor(t.status.danger)}`,
    `val ErrorText = ${kotlinColor(t.status.errorText)}`,
  ].join("\n"),
);

// 5. apps/native/android/app/src/main/res/values/colors.xml
replaceGeneratedBlock(
  "apps/native/android/app/src/main/res/values/colors.xml",
  "    <!-- BEGIN GENERATED THEME TOKENS -->",
  "    <!-- END GENERATED THEME TOKENS -->",
  `    <color name="blackout">#${parseHex(t.surfaces.frame, "frame").upper}</color>`,
);

// 6. apps/native/apple/Sources/CaperCore/CaperView.swift
replaceGeneratedBlock(
  "apps/native/apple/Sources/CaperCore/CaperView.swift",
  "    // BEGIN GENERATED THEME TOKENS",
  "    // END GENERATED THEME TOKENS",
  [
    `    public static let blackout = ${swiftColor(t.surfaces.frame)}`,
    `    public static let surface = ${swiftColor(t.surfaces.base)}`,
    `    public static let raised = ${swiftColor(t.surfaces.raised)}`,
    `    public static let sidebar = ${swiftColor(t.surfaces.sidebar)}`,
    `    /// Web's --surface-hover (6% white) over the sidebar, opaque: a private`,
    `    /// channel's own members in the member list.`,
    `    public static let sidebarRaised = ${swiftColor(t.surfaces.sidebarRaised)}`,
    `    public static let conversation = ${swiftColor(t.surfaces.conversation)}`,
    `    public static let composer = ${swiftColor(t.surfaces.composer)}`,
    `    /// 3% white over the conversation: the reply draft card above the composer.`,
    `    public static let replyDraft = ${swiftColor(t.surfaces.replyDraft)}`,
    `    public static let border = ${swiftColor(t.borders.default)}`,
    `    public static let borderSubtle = Color.white.opacity(0.08)`,
    `    public static let surfaceRim = Color.white.opacity(0.08)`,
    `    public static let text = ${swiftColor(t.text.primary)}`,
    `    public static let muted = ${swiftColor(t.text.muted)}`,
    `    public static let link = ${swiftColor(t.text.link)}`,
    `    public static let terracotta = ${swiftColor(t.accent.primary)}`,
    `    public static let terracottaBright = ${swiftColor(t.accent.bright)}`,
    `    /// Primary actions are warm chalk white with obsidian labels (shared/themes/obsidian.json).`,
    `    public static let action = ${swiftColor(t.action.primary)}`,
    `    public static let actionHover = ${swiftColor(t.action.primaryHover)}`,
    `    public static let actionPress = ${swiftColor(t.action.primaryPress)}`,
    `    public static let onAction = blackout`,
    `    public static let green = ${swiftColor(t.brand.caper)}`,
    `    public static let voiceSessionGreen = ${swiftColor(t.status.voice)}`,
    `    public static let pinGold = ${swiftColor(t.status.pin)}`,
    `    /// Web's error text, in place of the system red.`,
    `    public static let error = ${swiftColor(t.status.errorText)}`,
    `    /// Web's muted/deafened account controls: the icon and its options chevron.`,
    `    public static let voiceOff = ${swiftColor(t.status.voiceOffIcon)}`,
  ].join("\n"),
);

// 7. apps/native/desktop/src/main.rs
replaceGeneratedBlock(
  "apps/native/desktop/src/main.rs",
  "// BEGIN GENERATED THEME TOKENS",
  "// END GENERATED THEME TOKENS",
  [
    `const BLACKOUT: Color32 = ${rustRgb(t.surfaces.frame)};`,
    `const SURFACE: Color32 = ${rustRgb(t.surfaces.base)};`,
    `const RAISED: Color32 = ${rustRgb(t.surfaces.raised)};`,
    `const SIDEBAR: Color32 = ${rustRgb(t.surfaces.sidebar)};`,
    `/// Web's --surface-hover (6% white) over the sidebar, opaque: a private`,
    `/// channel's own members in the member list.`,
    `const SIDEBAR_RAISED: Color32 = ${rustRgb(t.surfaces.sidebarRaised)};`,
    `const CONVERSATION: Color32 = ${rustRgb(t.surfaces.conversation)};`,
    `const COMPOSER: Color32 = ${rustRgb(t.surfaces.composer)};`,
    `/// Web's \`--surface-hover\`: a neutral wash that reads on every dark surface.`,
    `const SURFACE_HOVER: Color32 = Color32::from_rgba_premultiplied(15, 15, 15, 15);`,
    `const BORDER: Color32 = ${rustRgb(t.borders.default)};`,
    `/// The reply-draft card: 3% white over the conversation surface.`,
    `const REPLY_CARD: Color32 = ${rustRgb(t.surfaces.replyDraft)};`,
    `const TEXT: Color32 = ${rustRgb(t.text.primary)};`,
    `const MUTED: Color32 = ${rustRgb(t.text.muted)};`,
    `const TERRACOTTA: Color32 = ${rustRgb(t.accent.primary)};`,
    `const TERRACOTTA_BRIGHT: Color32 = ${rustRgb(t.accent.bright)};`,
    `const LINK: Color32 = ${rustRgb(t.text.link)};`,
    `const LINK_ACTIVE: Color32 = ${rustRgb(t.text.linkHover)};`,
    `/// Primary actions are warm chalk white with obsidian labels (shared/themes/obsidian.json).`,
    `const ACTION: Color32 = ${rustRgb(t.action.primary)};`,
    `const ACTION_HOVER: Color32 = ${rustRgb(t.action.primaryHover)};`,
    `const ACTION_PRESS: Color32 = ${rustRgb(t.action.primaryPress)};`,
    `const ON_ACTION: Color32 = BLACKOUT;`,
    `const CAPER: Color32 = ${rustRgb(t.brand.caper)};`,
    `const VOICE_SESSION_GREEN: Color32 = ${rustRgb(t.status.voice)};`,
    `const PIN_GOLD: Color32 = ${rustRgb(t.status.pin)};`,
    `const ERROR: Color32 = ${rustRgb(t.status.errorText)};`,
    `const MESSAGE_TEXT: Color32 = ${rustRgb(t.text.message)};`,
  ].join("\n"),
);

console.log(
  checkOnly
    ? `Verified ${presets.size} theme presets and all 7 client token blocks.`
    : `Synced ${presets.size} theme presets into all 7 client token blocks.`,
);
