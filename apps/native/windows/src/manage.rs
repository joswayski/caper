//! Space and channel management, as in the egui client and web: "Create a
//! space" (a name, checked with the shared `caper_client::names` rules on
//! submit) and "Create a channel" (a name normalized as it is typed, with
//! its # or lock, web's guidance and the Private channel switch).
use crate::composer::Composer;
use crate::gfx::{self, Gfx, rect};
use crate::icons::{self, Icon};
use crate::overlay::{DIRECT_FIELD_H, Dialog, FIELD_PAD_X, Kind, Part, Rect};
use crate::theme::*;
use caper_client::names;
use windows::Win32::Graphics::Direct2D::*;
use windows::core::Result;

/// Room for Create a channel's # or lock before its text.
pub const ICON_W: f32 = 22.0;
const TOGGLE_H: f32 = 26.0;
/// Web's guidance under a new channel's name.
pub const GUIDE: &str = "Channels are where conversations happen around a topic. Use a name that is easy to find and understand.";

/// A name as typed, kept as web keeps it: a channel's normalized
/// (`normalize_channel`) with the caret after the same typed text, a
/// space's cut at 80 characters. None when it already is.
pub(crate) fn normalized(text: &[u16], caret: usize, channel: bool) -> Option<(Vec<u16>, usize)> {
    let caret = caret.min(text.len());
    let typed = String::from_utf16_lossy(text);
    let (next, at) = if channel {
        let before = String::from_utf16_lossy(&text[..caret]);
        (
            names::normalize_channel(&typed),
            names::normalize_channel(&before).encode_utf16().count(),
        )
    } else {
        (typed.chars().take(80).collect::<String>(), caret)
    };
    if next == typed {
        return None;
    }
    let next = gfx::wide(&next);
    let at = at.min(next.len());
    Some((next, at))
}

impl Dialog {
    pub fn create_space() -> Self {
        let field = Composer {
            focused: true,
            caret_on: true,
            ..Default::default()
        };
        Self::new(Kind::CreateSpace, None, field)
    }

    /// Create a channel in the space named `space_name`.
    pub fn create_channel(space_name: String) -> Self {
        let mut d = Self::create_space();
        d.kind = Kind::CreateChannel;
        d.space_name = space_name;
        d
    }

    /// The name field's label.
    pub fn create_label(&self) -> &'static str {
        if self.kind == Kind::CreateChannel {
            "Channel name"
        } else {
            "Space name"
        }
    }

    /// The name sent, as web sends it: a space's trimmed, a channel's
    /// without the dash left while typing.
    pub fn create_name(&self) -> String {
        let draft = self.draft();
        if self.kind == Kind::CreateChannel {
            draft.trim_end_matches('-').to_owned()
        } else {
            names::space_name(&draft).to_owned()
        }
    }

    /// Under the Private channel switch, as on web.
    pub fn privacy_note(&self) -> String {
        if self.private {
            "Only you and the people you add can view or join.".into()
        } else {
            let space = if self.space_name.is_empty() {
                "this space"
            } else {
                self.space_name.as_str()
            };
            format!("Anyone in {space} can view or join this channel.")
        }
    }

    /// The name as typed, kept as web keeps it: a channel's normalized
    /// (`normalize_channel`) with the caret after the same typed text, a
    /// space's cut at 80 characters.
    pub(crate) fn normalize_name(&mut self) {
        let channel = self.kind == Kind::CreateChannel;
        if let Some((text, caret)) = normalized(&self.field.text, self.field.caret, channel) {
            self.field.text = text;
            self.field.caret = caret;
            self.field.anchor = caret;
        }
    }

    /// The label, the name field and, for a channel, its guidance, the
    /// Private channel switch and what it means, from `y`.
    pub(crate) fn relayout_create(
        &mut self,
        g: &Gfx,
        inner: f32,
        mut y: f32,
        parts: &mut Vec<(Part, Rect)>,
    ) -> Result<f32> {
        let text_h = |text: &str, format| -> Result<f32> {
            Ok(gfx::metrics(&g.layout(&gfx::wide(text), format, inner)?).height)
        };
        y += 8.0;
        let h = text_h(self.create_label(), &g.formats.dialog_note)?;
        parts.push((Part::Label, (0.0, y, inner, h)));
        y += h + 6.0;
        let channel = self.kind == Kind::CreateChannel;
        let icon = if channel { ICON_W } else { 0.0 };
        self.field.relayout_with(
            g,
            &g.formats.composer,
            15.0,
            inner - 2.0 * FIELD_PAD_X - icon,
        )?;
        parts.push((Part::Field, (0.0, y, inner, DIRECT_FIELD_H)));
        y += DIRECT_FIELD_H;
        if !channel {
            return Ok(y);
        }
        y += 8.0;
        let h = text_h(GUIDE, &g.formats.direct_note)?;
        parts.push((Part::Guide, (0.0, y, inner, h)));
        y += h + 14.0;
        parts.push((Part::Toggle, (0.0, y, inner, TOGGLE_H)));
        y += TOGGLE_H + 4.0;
        let h = text_h(&self.privacy_note(), &g.formats.direct_note)?;
        parts.push((Part::Hint, (0.0, y, inner, h)));
        Ok(y + h)
    }

    /// Create a channel's # or lock, inside its field.
    pub(crate) fn draw_channel_icon(&self, dc: &ID2D1DeviceContext, (x, y, _, h): Rect) {
        let (l, t) = (x + FIELD_PAD_X, y + (h - 16.0) / 2.0);
        icons::draw(
            dc,
            if self.private { Icon::Lock } else { Icon::Hash },
            rect(l, t, l + 16.0, t + 16.0),
            TEXT_MUTED,
            1.0,
        );
    }

    pub(crate) fn draw_create_part(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        part: Part,
        (x, y, w, h): Rect,
    ) -> Result<()> {
        let draw = |text: &str, format, brush, tx: f32, ty: f32| -> Result<()> {
            let l = g.layout(&gfx::wide(text), format, (w - (tx - x)).max(1.0))?;
            crate::text::draw(dc, &l, tx, ty, brush);
            Ok(())
        };
        match part {
            Part::Label => draw(
                self.create_label(),
                &g.formats.dialog_note,
                &g.brushes.text,
                x,
                y,
            )?,
            Part::Guide => draw(GUIDE, &g.formats.direct_note, &g.brushes.muted, x, y)?,
            Part::Toggle => {
                let cy = y + h / 2.0;
                icons::draw(
                    dc,
                    Icon::Lock,
                    rect(x, cy - 8.0, x + 16.0, cy + 8.0),
                    TEXT_MUTED,
                    1.0,
                );
                let l = g.layout(&gfx::wide("Private channel"), &g.formats.dialog_note, w)?;
                let lh = gfx::metrics(&l).height;
                crate::text::draw(dc, &l, x + 24.0, cy - lh / 2.0, &g.brushes.text);
                crate::settings::switch(
                    g,
                    dc,
                    x + w - 40.0,
                    cy - 11.0,
                    self.enabled(Part::Toggle),
                    self.private,
                    self.hover == Some(Part::Toggle),
                );
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(d: &mut Dialog, text: &str, caret: usize) {
        d.field.text = gfx::wide(text);
        d.field.caret = caret;
        d.normalize_name();
    }

    #[test]
    fn a_channel_name_is_normalized_as_typed_and_the_caret_stays_with_it() {
        let mut d = Dialog::create_channel("Studio".into());
        // A space typed mid-name becomes a dash; the caret follows what was typed.
        typed(&mut d, "Ab cD", 3);
        assert_eq!((d.draft().as_str(), d.field.caret), ("ab-cd", 3));
        // Dropped characters before the caret move it back.
        typed(&mut d, "a!!b", 3);
        assert_eq!((d.draft().as_str(), d.field.caret), ("ab", 1));
        // The trailing dash stays while typing but isn't sent.
        typed(&mut d, "release notes ", 14);
        assert_eq!(d.draft(), "release-notes-");
        assert_eq!(d.create_name(), "release-notes");
        assert_eq!(
            d.privacy_note(),
            "Anyone in Studio can view or join this channel."
        );
    }

    #[test]
    fn a_space_name_keeps_80_characters_and_is_sent_trimmed() {
        let mut d = Dialog::create_space();
        let long = "\u{1F600}".repeat(81);
        typed(&mut d, &long, 162);
        assert_eq!(d.draft().chars().count(), 80);
        assert_eq!(d.field.caret, d.field.text.len());
        typed(&mut d, "  Gaming nights  ", 3);
        assert_eq!(d.create_name(), "Gaming nights");
    }
}
