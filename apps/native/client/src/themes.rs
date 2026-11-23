//! Shared Phase 2 theme engine (`caper.theme.v1`) for Rust desktop clients
//! (Linux GTK4, Windows Win32, and Desktop egui).
//!
//! Embeds the canonical `shared/themes/obsidian.json` and `shared/themes/paper.json`
//! presets, validates custom `.capertheme.json` files, and formats GTK CSS
//! overrides and RGB palettes.

use serde::{Deserialize, Serialize};

pub const OBSIDIAN_JSON: &str = include_str!("../../../../shared/themes/obsidian.json");
pub const PAPER_JSON: &str = include_str!("../../../../shared/themes/paper.json");
pub const THEME_PREF_KEY: &str = "theme-v1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeDocument {
    pub schema: String,
    pub id: String,
    pub name: String,
    pub appearance: String,
    pub surfaces: ThemeSurfaces,
    pub borders: ThemeBorders,
    pub text: ThemeText,
    pub action: ThemeAction,
    pub accent: ThemeAccent,
    pub brand: ThemeBrand,
    pub status: ThemeStatus,
    pub radii: ThemeRadii,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeSurfaces {
    pub frame: String,
    pub base: String,
    pub sidebar: String,
    pub sidebar_raised: String,
    pub conversation: String,
    pub raised: String,
    pub composer: String,
    pub reply_draft: String,
    pub hover: String,
    pub hover_alpha: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeBorders {
    pub default: String,
    pub translucent: String,
    pub subtle: String,
    pub rim: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeText {
    pub primary: String,
    pub message: String,
    pub muted: String,
    pub link: String,
    pub link_hover: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeAction {
    pub primary: String,
    pub primary_hover: String,
    pub primary_press: String,
    pub on_primary: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeAccent {
    pub primary: String,
    pub bright: String,
    pub soft_text: String,
    pub surface: String,
    pub border: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeBrand {
    pub caper: String,
    pub caper_bright: String,
    pub resize_highlight: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeStatus {
    pub voice: String,
    pub pin: String,
    pub pin_wash: String,
    pub idle: String,
    pub error_text: String,
    pub danger: String,
    pub danger_hover: String,
    pub danger_wash: String,
    pub on_danger: String,
    pub voice_off: String,
    pub voice_off_icon: String,
    pub voice_off_wash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeRadii {
    pub canvas: u16,
    pub card: u16,
    pub composer: u16,
    pub control: u16,
    pub pill: u16,
    pub avatar: String,
}

pub fn obsidian() -> ThemeDocument {
    serde_json::from_str(OBSIDIAN_JSON).expect("bundled obsidian.json must be valid")
}

pub fn paper() -> ThemeDocument {
    serde_json::from_str(PAPER_JSON).expect("bundled paper.json must be valid")
}

pub fn parse_hex(hex: &str) -> Option<u32> {
    let body = hex.strip_prefix('#')?;
    if body.len() != 6 || !body.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(body, 16).ok()
}

/// Loads either a built-in preset name (`"obsidian"`, `"paper"`) or a raw
/// `caper.theme.v1` JSON string.
pub fn parse_theme_preference(raw: &str) -> Result<ThemeDocument, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("obsidian") {
        return Ok(obsidian());
    }
    if trimmed.eq_ignore_ascii_case("paper") {
        return Ok(paper());
    }
    parse_theme_json(trimmed)
}

pub fn parse_theme_json(raw: &str) -> Result<ThemeDocument, String> {
    let doc: ThemeDocument =
        serde_json::from_str(raw).map_err(|e| format!("Invalid theme JSON: {e}"))?;
    validate_theme(&doc)?;
    Ok(doc)
}

pub fn validate_theme(doc: &ThemeDocument) -> Result<(), String> {
    if doc.schema != "caper.theme.v1" {
        return Err(r#"Theme schema must be "caper.theme.v1"."#.into());
    }
    if doc.id.is_empty() || doc.id.len() > 48 {
        return Err("Theme id must be 1-48 characters.".into());
    }
    if doc.name.trim().is_empty() || doc.name.len() > 48 {
        return Err("Theme name must be 1-48 characters.".into());
    }
    if doc.appearance != "dark" && doc.appearance != "light" {
        return Err(r#"Theme appearance must be "dark" or "light"."#.into());
    }
    for (label, value) in [
        ("surfaces.frame", &doc.surfaces.frame),
        ("surfaces.base", &doc.surfaces.base),
        ("surfaces.sidebar", &doc.surfaces.sidebar),
        ("surfaces.sidebarRaised", &doc.surfaces.sidebar_raised),
        ("surfaces.conversation", &doc.surfaces.conversation),
        ("surfaces.raised", &doc.surfaces.raised),
        ("surfaces.composer", &doc.surfaces.composer),
        ("surfaces.replyDraft", &doc.surfaces.reply_draft),
        ("borders.default", &doc.borders.default),
        ("text.primary", &doc.text.primary),
        ("text.message", &doc.text.message),
        ("text.muted", &doc.text.muted),
        ("text.link", &doc.text.link),
        ("text.linkHover", &doc.text.link_hover),
        ("action.primary", &doc.action.primary),
        ("action.primaryHover", &doc.action.primary_hover),
        ("action.primaryPress", &doc.action.primary_press),
        ("action.onPrimary", &doc.action.on_primary),
        ("accent.primary", &doc.accent.primary),
        ("accent.bright", &doc.accent.bright),
        ("accent.softText", &doc.accent.soft_text),
        ("accent.surface", &doc.accent.surface),
        ("accent.border", &doc.accent.border),
        ("brand.caper", &doc.brand.caper),
        ("brand.caperBright", &doc.brand.caper_bright),
        ("status.voice", &doc.status.voice),
        ("status.pin", &doc.status.pin),
        ("status.errorText", &doc.status.error_text),
        ("status.danger", &doc.status.danger),
        ("status.dangerHover", &doc.status.danger_hover),
        ("status.voiceOff", &doc.status.voice_off),
    ] {
        if parse_hex(value).is_none() {
            return Err(format!("{label} must be a 6-digit hex color like #131316."));
        }
    }
    Ok(())
}

impl ThemeDocument {
    pub fn is_light(&self) -> bool {
        self.appearance == "light"
    }

    pub fn to_pretty_json(&self) -> String {
        serde_json::to_string_pretty(self)
            .map(|s| format!("{s}\n"))
            .unwrap_or_default()
    }

    /// Generates a GTK4 CSS override sheet redefining every `@define-color caper_*`
    /// token (plus light-mode surface/button polish when `appearance == "light"`).
    pub fn to_gtk_css(&self) -> String {
        let hover_rgba = if self.is_light() {
            format!("rgba(11, 11, 12, {:.2})", self.surfaces.hover_alpha)
        } else {
            format!("rgba(255, 255, 255, {:.2})", self.surfaces.hover_alpha)
        };
        let border_subtle = if self.is_light() {
            "rgba(11, 11, 12, 0.08)"
        } else {
            "rgba(255, 255, 255, 0.06)"
        };
        let surface_rim = if self.is_light() {
            "rgba(255, 255, 255, 0.85)"
        } else {
            "rgba(255, 255, 255, 0.08)"
        };
        let mut css = format!(
            "@define-color caper_blackout {};\n\
             @define-color caper_surface {};\n\
             @define-color caper_surface_raised {};\n\
             @define-color caper_surface_sidebar {};\n\
             @define-color caper_surface_conversation {};\n\
             @define-color caper_surface_composer {};\n\
             @define-color caper_surface_hover {};\n\
             @define-color caper_border {};\n\
             @define-color caper_border_subtle {};\n\
             @define-color caper_surface_rim {};\n\
             @define-color caper_text {};\n\
             @define-color caper_text_message {};\n\
             @define-color caper_text_muted {};\n\
             @define-color caper_link {};\n\
             @define-color caper_link_hover {};\n\
             @define-color caper_terracotta {};\n\
             @define-color caper_terracotta_bright {};\n\
             @define-color caper_accent_soft_text {};\n\
             @define-color caper_accent_surface {};\n\
             @define-color caper_accent_border {};\n\
             @define-color caper_action {};\n\
             @define-color caper_action_hover {};\n\
             @define-color caper_action_press {};\n\
             @define-color caper_on_action {};\n\
             @define-color caper_green {};\n\
             @define-color caper_green_bright {};\n\
             @define-color caper_voice_green {};\n\
             @define-color caper_pin_gold {};\n\
             @define-color caper_error {};\n\
             @define-color caper_danger {};\n\
             @define-color caper_danger_hover {};\n\
             @define-color caper_voice_off {};\n",
            self.surfaces.frame,
            self.surfaces.base,
            self.surfaces.raised,
            self.surfaces.sidebar,
            self.surfaces.conversation,
            self.surfaces.composer,
            hover_rgba,
            self.borders.default,
            border_subtle,
            surface_rim,
            self.text.primary,
            self.text.message,
            self.text.muted,
            self.text.link,
            self.text.link_hover,
            self.accent.primary,
            self.accent.bright,
            self.accent.soft_text,
            self.accent.surface,
            self.accent.border,
            self.action.primary,
            self.action.primary_hover,
            self.action.primary_press,
            self.action.on_primary,
            self.brand.caper,
            self.brand.caper_bright,
            self.status.voice,
            self.status.pin,
            self.status.error_text,
            self.status.danger,
            self.status.danger_hover,
            self.status.voice_off,
        );
        if self.is_light() {
            css.push_str(
                "button.primary {\n\
                   background-image: linear-gradient(180deg, @caper_action_hover, @caper_action);\n\
                   border-color: rgba(0, 0, 0, 0.78);\n\
                   box-shadow: inset 0 1.5px 0 rgba(255, 255, 255, 0.24), inset 0 -2px 0 rgba(0, 0, 0, 0.35), 0 3px 10px rgba(22, 22, 26, 0.2);\n\
                 }\n\
                 button.primary:hover {\n\
                   background-image: linear-gradient(180deg, @caper_action_hover, @caper_action);\n\
                 }\n\
                 .composer {\n\
                   border-color: rgba(11, 11, 12, 0.14);\n\
                   box-shadow: 0 0 0 4px rgba(17, 17, 20, 0.025), 0 0 0 5px rgba(17, 17, 20, 0.09);\n\
                 }\n\
                 .composer:focus-within {\n\
                   border-color: alpha(@caper_terracotta, 0.62);\n\
                   box-shadow: 0 0 0 4px rgba(17, 17, 20, 0.025), 0 0 0 5px rgba(17, 17, 20, 0.09);\n\
                 }\n",
            );
        }
        css
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_presets_parse_and_validate() {
        let dark = obsidian();
        let light = paper();
        assert_eq!(dark.id, "obsidian");
        assert!(!dark.is_light());
        assert_eq!(light.id, "paper");
        assert!(light.is_light());
        assert!(validate_theme(&dark).is_ok());
        assert!(validate_theme(&light).is_ok());
        assert!(
            light
                .to_gtk_css()
                .contains("@define-color caper_blackout #efeff2;")
        );
    }

    #[test]
    fn rejects_invalid_custom_theme_json() {
        assert!(parse_theme_json("{invalid").is_err());
        let mut broken = obsidian();
        broken.surfaces.frame = "not-a-hex".into();
        assert!(validate_theme(&broken).is_err());
    }
}
