//! Caper's shared design tokens (`shared/design.css`) and the conversation
//! screen's geometry, in DIPs.

pub const BLACKOUT: u32 = 0x0C0D0F;
pub const SURFACE_RAISED: u32 = 0x1C1F21;
pub const SURFACE_SIDEBAR: u32 = 0x151C1E;
pub const SURFACE_CONVERSATION: u32 = 0x192123;
pub const SURFACE_COMPOSER: u32 = 0x283133;
pub const BORDER: u32 = 0x34383B;
pub const TEXT: u32 = 0xF3F4F5;
pub const TEXT_MUTED: u32 = 0xB9BCBE;
pub const LINK: u32 = 0x69CFFF;
pub const TERRACOTTA: u32 = 0xB64D32;
/// Hovered fields and controls.
pub const TERRACOTTA_BRIGHT: u32 = 0xDB6849;
/// Error text, readable on blackout.
pub const ERROR: u32 = 0xFF9B82;
/// Primary actions (`.primary`): eggshell at rest, lighter on hover, darker pressed.
pub const ACTION: u32 = 0xF1EDE4;
pub const ACTION_HOVER: u32 = 0xFBF9F4;
pub const ACTION_PRESS: u32 = 0xDDD7CB;
pub const CAPER: u32 = 0x637A43;
/// Voice under way: egui's session green.
pub const VOICE_GREEN: u32 = 0x4AA86B;
/// `--surface-hover`: white at 6%.
pub const HOVER: (u32, f32) = (0xFFFFFF, 0.06);
pub const SELECTION: (u32, f32) = (0x3A6EA5, 0.55);
pub const RADIUS: f32 = 8.0;

pub const RAIL_W: f32 = 60.0;
pub const SIDEBAR_W: f32 = 280.0;
pub const MEMBERS_W: f32 = 220.0;
pub const HEADER_H: f32 = 54.0;
/// The composer's lip over the bottom of a message history.
pub const LIP_H: f32 = 8.0;
pub const SECTION_TOP: f32 = 66.0;
pub const CHANNEL_TOP: f32 = 96.0;
pub const CHANNEL_H: f32 = 32.0;
pub const MEMBER_TOP: f32 = 96.0;
pub const MEMBER_H: f32 = 41.0;
pub const RAIL_ITEM: f32 = 40.0;
pub const RAIL_PITCH: f32 = 47.0;
pub const RAIL_TOP: f32 = 12.0;
