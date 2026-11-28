//! Caper's shared design tokens (`shared/design.css`) and the conversation
//! screen's geometry, in DIPs.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

// BEGIN GENERATED THEME TOKENS
pub const BLACKOUT: u32 = 0x090A0C;
pub const SURFACE: u32 = 0x111216;
pub const SURFACE_RAISED: u32 = 0x191B22;
pub const SURFACE_SIDEBAR: u32 = 0x0D0E11;
pub const SURFACE_CONVERSATION: u32 = 0x121317;
pub const SURFACE_COMPOSER: u32 = 0x1B1D24;
pub const BORDER: u32 = 0x24262F;
pub const TEXT: u32 = 0xF4F4F6;
pub const TEXT_MUTED: u32 = 0x8F919C;
pub const LINK: u32 = 0x6EB8FF;
pub const TERRACOTTA: u32 = 0x4E7835;
/// Hovered fields and controls.
pub const TERRACOTTA_BRIGHT: u32 = 0x77A659;
pub const ACCENT_SURFACE: u32 = 0x172314;
pub const ACCENT_SOFT_TEXT: u32 = 0xB9D9A3;
/// Error text, readable on blackout.
pub const ERROR: u32 = 0xFF9685;
/// Primary actions (`.primary`): warm chalk white at rest, brighter on hover, darker pressed.
pub const ACTION: u32 = 0xF4F4F6;
pub const ACTION_HOVER: u32 = 0xFFFFFF;
pub const ACTION_PRESS: u32 = 0xE2E3E8;
pub const CAPER: u32 = 0x638E46;
/// Voice under way: live session emerald.
pub const VOICE_GREEN: u32 = 0x4CC970;
/// `--surface-hover`: white at 6%.
pub const HOVER: (u32, f32) = (0xFFFFFF, 0.05);
// END GENERATED THEME TOKENS
pub const SELECTION: (u32, f32) = (0x3A6EA5, 0.55);

/// Active color tokens for the current appearance (`Obsidian` dark or `Paper` light).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub blackout: u32,
    pub surface: u32,
    pub surface_raised: u32,
    pub surface_sidebar: u32,
    pub surface_conversation: u32,
    pub surface_composer: u32,
    pub border: u32,
    pub text: u32,
    pub text_muted: u32,
    pub link: u32,
    pub terracotta: u32,
    pub terracotta_bright: u32,
    pub accent_surface: u32,
    pub accent_soft_text: u32,
    pub error: u32,
    pub action: u32,
    pub action_hover: u32,
    pub action_press: u32,
    pub on_action: u32,
    pub caper: u32,
    pub voice_green: u32,
    /// A muted microphone or deafened output: its icon, and at 16% its wash.
    pub voice_off: u32,
    pub hover: (u32, f32),
    pub scrim: (u32, f32),
}

impl Palette {
    pub const OBSIDIAN_DEFAULTS: Self = Self {
        blackout: BLACKOUT,
        surface: SURFACE,
        surface_raised: SURFACE_RAISED,
        surface_sidebar: SURFACE_SIDEBAR,
        surface_conversation: SURFACE_CONVERSATION,
        surface_composer: SURFACE_COMPOSER,
        border: BORDER,
        text: TEXT,
        text_muted: TEXT_MUTED,
        link: LINK,
        terracotta: TERRACOTTA,
        terracotta_bright: TERRACOTTA_BRIGHT,
        accent_surface: ACCENT_SURFACE,
        accent_soft_text: ACCENT_SOFT_TEXT,
        error: ERROR,
        action: ACTION,
        action_hover: ACTION_HOVER,
        action_press: ACTION_PRESS,
        on_action: BLACKOUT,
        caper: CAPER,
        voice_green: VOICE_GREEN,
        voice_off: 0xFF6E7F,
        hover: HOVER,
        scrim: (0x090A0C, 0.6),
    };

    pub fn from_doc(doc: &caper_client::themes::ThemeDocument) -> Self {
        let hex = |s: &str, fallback: u32| caper_client::themes::parse_hex(s).unwrap_or(fallback);
        let d = Self::OBSIDIAN_DEFAULTS;
        let light = doc.is_light();
        Self {
            blackout: hex(&doc.surfaces.frame, d.blackout),
            surface: hex(&doc.surfaces.base, d.surface),
            surface_raised: hex(&doc.surfaces.raised, d.surface_raised),
            surface_sidebar: hex(&doc.surfaces.sidebar, d.surface_sidebar),
            surface_conversation: hex(&doc.surfaces.conversation, d.surface_conversation),
            surface_composer: hex(&doc.surfaces.composer, d.surface_composer),
            border: hex(&doc.borders.default, d.border),
            text: hex(&doc.text.primary, d.text),
            text_muted: hex(&doc.text.muted, d.text_muted),
            link: hex(&doc.text.link, d.link),
            terracotta: hex(&doc.accent.primary, d.terracotta),
            terracotta_bright: hex(&doc.accent.bright, d.terracotta_bright),
            accent_surface: hex(&doc.accent.surface, d.accent_surface),
            accent_soft_text: hex(&doc.accent.soft_text, d.accent_soft_text),
            error: hex(&doc.status.error_text, d.error),
            action: hex(&doc.action.primary, d.action),
            action_hover: hex(&doc.action.primary_hover, d.action_hover),
            action_press: hex(&doc.action.primary_press, d.action_press),
            on_action: hex(&doc.action.on_primary, d.on_action),
            caper: hex(&doc.brand.caper, d.caper),
            voice_green: hex(&doc.status.voice, d.voice_green),
            voice_off: hex(&doc.status.voice_off, d.voice_off),
            hover: (
                if light { 0x0F172A } else { 0xFFFFFF },
                doc.surfaces.hover_alpha,
            ),
            scrim: if light {
                (0x090A0C, 0.36)
            } else {
                (0x090A0C, 0.6)
            },
        }
    }
}

static LIGHT: AtomicBool = AtomicBool::new(false);
static OBSIDIAN: LazyLock<Palette> =
    LazyLock::new(|| Palette::from_doc(&caper_client::themes::obsidian()));
static PAPER: LazyLock<Palette> =
    LazyLock::new(|| Palette::from_doc(&caper_client::themes::paper()));

/// Normalizes a persisted `theme-v1` preference to `true` for Paper (light)
/// and `false` for Obsidian (dark).
pub fn is_light_preference(raw: &str) -> bool {
    matches!(
        caper_client::themes::parse_theme_preference(raw).ok(),
        Some(doc) if doc.id == "paper" || doc.is_light()
    )
}

pub fn set_light(light: bool) {
    LIGHT.store(light, Ordering::Relaxed);
}

pub fn is_light() -> bool {
    LIGHT.load(Ordering::Relaxed)
}

pub fn palette() -> Palette {
    if is_light() { *PAPER } else { *OBSIDIAN }
}

pub fn blackout() -> u32 {
    palette().blackout
}

pub fn surface() -> u32 {
    palette().surface
}

pub fn surface_raised() -> u32 {
    palette().surface_raised
}

pub fn surface_sidebar() -> u32 {
    palette().surface_sidebar
}

pub fn surface_conversation() -> u32 {
    palette().surface_conversation
}

pub fn surface_composer() -> u32 {
    palette().surface_composer
}

pub fn border() -> u32 {
    palette().border
}

pub fn text() -> u32 {
    palette().text
}

pub fn text_muted() -> u32 {
    palette().text_muted
}

pub fn terracotta() -> u32 {
    palette().terracotta
}

pub fn terracotta_bright() -> u32 {
    palette().terracotta_bright
}

pub fn accent_surface() -> u32 {
    palette().accent_surface
}

pub fn error() -> u32 {
    palette().error
}

pub fn action() -> u32 {
    palette().action
}

pub fn action_hover() -> u32 {
    palette().action_hover
}

pub fn on_action() -> u32 {
    palette().on_action
}

pub fn caper() -> u32 {
    palette().caper
}

pub fn voice_green() -> u32 {
    palette().voice_green
}

pub fn voice_off() -> u32 {
    palette().voice_off
}

pub fn scrim() -> (u32, f32) {
    palette().scrim
}

pub const RADIUS: f32 = 10.0;
#[allow(dead_code)]
pub const RADIUS_CARD: f32 = 13.0;
#[allow(dead_code)]
pub const RADIUS_COMPOSER: f32 = 15.0;
#[allow(dead_code)]
pub const RADIUS_CANVAS: f32 = 16.0;
#[allow(dead_code)]
pub const RADIUS_PILL: f32 = 999.0;

pub const RAIL_W: f32 = 60.0;
pub const SIDEBAR_W: f32 = 280.0;
pub const MEMBERS_W: f32 = 220.0;
pub const HEADER_H: f32 = 54.0;
/// How tall the composer backdrop's eased band is, over the message history
/// that scrolls on under it.
pub const FADE_H: f32 = 32.0;
/// How far below the composer's top edge its backdrop turns solid: about the
/// field's midline, so the history (and a hovered row) shows on beside the
/// field's upper half and the field floats over it rather than on a slab.
pub const COMPOSER_SINK: f32 = 22.0;
/// How tall the band above the typing row is where, while someone types,
/// the conversation colour eases in under it (web's `.chat-typing::before`).
pub const TYPING_FADE: f32 = 12.0;
pub const SECTION_TOP: f32 = 66.0;
pub const CHANNEL_TOP: f32 = 96.0;
/// A channel row's pitch: 6 DIPs around its 20 DIP line inside the
/// 32 DIP highlight, and 2 between rows.
pub const CHANNEL_H: f32 = 34.0;
/// The first member row: 16 DIPs under the section heading's line, as
/// the heading is under the header's border.
pub const MEMBER_TOP: f32 = 95.0;
/// A member row: 8 DIPs around its 30 DIP avatar inside the highlight.
pub const MEMBER_H: f32 = 46.0;
/// The Offline heading's band above the first offline row: its line with
/// the same 16 DIPs above and below as the first heading's (8 here, plus
/// the 8 a row keeps around its avatar).
pub const OFFLINE_H: f32 = 33.0;
pub const RAIL_ITEM: f32 = 40.0;
pub const RAIL_PITCH: f32 = 47.0;
pub const RAIL_TOP: f32 = 12.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsidian_doc_matches_generated_defaults() {
        assert_eq!(
            Palette::from_doc(&caper_client::themes::obsidian()),
            Palette::OBSIDIAN_DEFAULTS
        );
    }

    #[test]
    fn paper_doc_produces_light_palette() {
        let p = Palette::from_doc(&caper_client::themes::paper());
        assert_eq!(p.blackout, 0xECEEF2);
        assert_eq!(p.surface_conversation, 0xFFFFFF);
        assert_eq!(p.surface_sidebar, 0xF1F2F6);
        assert_eq!(p.text, 0x111217);
        assert_eq!(p.action, 0x111217);
        assert_eq!(p.on_action, 0xFFFFFF);
        assert_eq!(p.hover.0, 0x0F172A);
    }

    #[test]
    fn preference_normalization_recognizes_paper_and_obsidian() {
        assert!(is_light_preference("paper"));
        assert!(!is_light_preference("obsidian"));
        assert!(!is_light_preference(""));
        assert!(!is_light_preference("invalid-theme"));
    }
}
