//! Caper's shared design tokens (`shared/design.css`) and the conversation
//! screen's geometry, in DIPs.

// BEGIN GENERATED THEME TOKENS
pub const BLACKOUT: u32 = 0x0B0B0C;
pub const SURFACE: u32 = 0x121214;
pub const SURFACE_RAISED: u32 = 0x1B1B1F;
pub const SURFACE_SIDEBAR: u32 = 0x0F0F11;
pub const SURFACE_CONVERSATION: u32 = 0x131316;
pub const SURFACE_COMPOSER: u32 = 0x1D1D22;
pub const BORDER: u32 = 0x29292F;
pub const TEXT: u32 = 0xF4F3EF;
pub const TEXT_MUTED: u32 = 0x9A99A2;
pub const LINK: u32 = 0x7CC7FF;
pub const TERRACOTTA: u32 = 0x557A38;
/// Hovered fields and controls.
pub const TERRACOTTA_BRIGHT: u32 = 0x87BC5C;
pub const ACCENT_SURFACE: u32 = 0x1C2816;
pub const ACCENT_SOFT_TEXT: u32 = 0xC3E4A4;
/// Error text, readable on blackout.
pub const ERROR: u32 = 0xFF9B82;
/// Primary actions (`.primary`): warm chalk white at rest, brighter on hover, darker pressed.
pub const ACTION: u32 = 0xF4F3EF;
pub const ACTION_HOVER: u32 = 0xFFFFFF;
pub const ACTION_PRESS: u32 = 0xDFDDD6;
pub const CAPER: u32 = 0x6F9948;
/// Voice under way: live session emerald.
pub const VOICE_GREEN: u32 = 0x4ADE80;
/// `--surface-hover`: white at 6%.
pub const HOVER: (u32, f32) = (0xFFFFFF, 0.06);
// END GENERATED THEME TOKENS
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
