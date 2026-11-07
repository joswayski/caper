//! Browse channels, as web's dialog (egui lists the same channels in its
//! sidebar): every channel in the open space under a search, each row
//! opening it, as a read-only preview while you haven't joined it. A row
//! shows its lock or hash, "Joined" or "Read-only preview", and the unread
//! dot of a channel with something new.
use crate::App;
use crate::composer::LINE;
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::overlay::{CARD_PAD, Dialog, Kind, Part};
use crate::theme::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::Result;

const WIDTH: f32 = 520.0;
const ROW_H: f32 = 48.0;
const FIELD_H: f32 = 42.0;
const FIELD_PAD_X: f32 = 10.0;
const CLOSE: f32 = 32.0;

/// One of the space's channels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub private: bool,
    pub joined: bool,
    pub unread: bool,
}

/// What the dialog browses: the open space and its channels.
pub struct Browse {
    pub space: String,
    pub space_name: String,
    pub rows: Vec<Row>,
}

impl Browse {
    /// The rows the search shows, as web filters them: its text without a
    /// leading `#` (people type the name as it is shown), in the name.
    pub fn shown(&self, query: &str) -> Vec<&Row> {
        let query = query.trim().trim_start_matches('#').trim().to_lowercase();
        self.rows
            .iter()
            .filter(|r| r.name.to_lowercase().contains(&query))
            .collect()
    }
}

fn layout(g: &Gfx, text: &str, format: &IDWriteTextFormat, w: f32) -> Result<IDWriteTextLayout> {
    g.layout(&gfx::wide(text), format, w.max(1.0))
}

fn height(l: &IDWriteTextLayout) -> f32 {
    gfx::metrics(l).height
}

impl Dialog {
    pub fn browse(browse: Browse) -> Self {
        let field = crate::composer::Composer {
            focused: true,
            caret_on: true,
            ..Default::default()
        };
        let mut d = Self::new(Kind::Browse, None, field);
        d.browse = Some(Box::new(browse));
        d
    }

    fn browse_shown(&self) -> Vec<&Row> {
        let query = self.draft();
        self.browse
            .as_ref()
            .map_or_else(Vec::new, |b| b.shown(&query))
    }

    /// Shown row `k`'s accessible name, as web's button.
    pub fn browse_row_name(&self, k: usize) -> String {
        self.browse_shown().get(k).map_or_else(String::new, |r| {
            format!(
                "Preview #{}{}{}",
                r.name,
                if r.private { ", private" } else { "" },
                if r.unread { ", unread activity" } else { "" }
            )
        })
    }

    /// The note when no row is shown.
    pub fn browse_empty(&self) -> &'static str {
        if self.draft().trim().is_empty() {
            "This space has no channels yet."
        } else {
            "No channels match your search."
        }
    }

    /// Shown row `k`'s channel, for opening it.
    pub fn browse_row(&self, k: usize) -> Option<(String, String)> {
        let b = self.browse.as_ref()?;
        let r = self.browse_shown().get(k).map(|r| r.id.clone())?;
        Some((b.space.clone(), r))
    }

    /// Close, the heading and note, the search, then the rows in a body
    /// that scrolls once it fills the room. The body keeps the height the
    /// whole list needs, so the search stays put as it narrows the list.
    pub(crate) fn relayout_browse(&mut self, g: &Gfx, w: f32, h: f32) -> Result<()> {
        let card_w = WIDTH.min(w - 48.0).max(240.0);
        let inner = card_w - 2.0 * CARD_PAD;
        let mut parts = Vec::new();
        parts.push((Part::Close, (inner - CLOSE, 0.0, CLOSE, CLOSE)));
        let title = layout(
            g,
            &self.title(),
            &g.formats.dialog_title,
            inner - CLOSE - 8.0,
        )?;
        parts.push((Part::Title, (0.0, 0.0, inner - CLOSE - 8.0, height(&title))));
        let mut y = height(&title).max(CLOSE) + 2.0;
        let note = layout(g, &self.note(), &g.formats.dialog_note, inner)?;
        parts.push((Part::Note, (0.0, y, inner, height(&note))));
        y += height(&note) + 12.0;
        self.field
            .relayout_with(g, &g.formats.composer, 15.0, inner - 2.0 * FIELD_PAD_X)?;
        parts.push((Part::Field, (0.0, y, inner, FIELD_H)));
        y += FIELD_H + 12.0;
        let body_top = y;
        let shown = self.browse_shown().len();
        if shown == 0 {
            let l = layout(g, self.browse_empty(), &g.formats.dialog_note, inner)?;
            parts.push((Part::Hint, (0.0, y, inner, height(&l))));
        }
        for k in 0..shown {
            parts.push((Part::Channel(k), (0.0, y + k as f32 * ROW_H, inner, ROW_H)));
        }
        let all = self.browse.as_ref().map_or(0, |b| b.rows.len()).max(1);
        let room = (h - 48.0 - 2.0 * CARD_PAD - body_top).clamp(ROW_H, 6.5 * ROW_H);
        let view = (all as f32 * ROW_H).min(room);
        let content = shown as f32 * ROW_H;
        self.scroll = self.scroll.clamp(0.0, (content - view).max(0.0));
        let card_h = body_top + view + 2.0 * CARD_PAD;
        let (cx, cy) = (
            ((w - card_w) / 2.0).round(),
            ((h - card_h) / 2.0).max(24.0).round(),
        );
        self.card = (cx, cy, card_w, card_h);
        for (p, r) in &mut parts {
            r.0 += cx + CARD_PAD;
            r.1 += cy + CARD_PAD;
            if matches!(p, Part::Channel(_)) {
                r.1 -= self.scroll;
            }
        }
        self.body = (cx + CARD_PAD, cy + CARD_PAD + body_top, inner, view);
        self.parts = parts;
        if let Some((fx, fy, _, fh)) = self.part_rect(Part::Field) {
            self.field_origin = (fx + FIELD_PAD_X, fy + (fh - LINE) / 2.0);
        }
        Ok(())
    }

    pub(crate) fn draw_browse(&self, g: &Gfx, dc: &ID2D1DeviceContext) -> Result<()> {
        for (part, r) in &self.parts {
            let (x, y, w, h) = *r;
            match part {
                Part::Title => {
                    let l = layout(g, &self.title(), &g.formats.dialog_title, w)?;
                    crate::text::draw(dc, &l, x, y, &g.brushes.text);
                }
                Part::Note | Part::Hint => {
                    let text = if *part == Part::Note {
                        self.note()
                    } else {
                        self.browse_empty().into()
                    };
                    let l = layout(g, &text, &g.formats.dialog_note, w)?;
                    crate::text::draw(dc, &l, x, y, &g.brushes.muted);
                }
                Part::Close => {
                    let lit = self.hover == Some(Part::Close);
                    if lit {
                        fill_round(dc, rect(x, y, x + w, y + h), 6.0, &g.brushes.hover);
                    }
                    icons::draw(
                        dc,
                        Icon::Close,
                        rect(x + 8.0, y + 8.0, x + w - 8.0, y + h - 8.0),
                        if lit { TEXT } else { TEXT_MUTED },
                        1.0,
                    );
                }
                Part::Field => {
                    let body = rect(x, y, x + w, y + h);
                    let stroke = if self.field.focused {
                        g.solid(TERRACOTTA)
                    } else if self.hover == Some(Part::Field) {
                        g.solid(TERRACOTTA_BRIGHT)
                    } else {
                        g.brushes.border.clone()
                    };
                    fill_round(dc, body, 7.0, &g.brushes.composer);
                    stroke_round(dc, body, 7.0, &stroke);
                    unsafe {
                        dc.PushAxisAlignedClip(
                            &rect(x + 2.0, y + 2.0, x + w - 2.0, y + h - 2.0),
                            D2D1_ANTIALIAS_MODE_ALIASED,
                        );
                    }
                    let (ox, oy) = self.text_origin();
                    let hint = layout(g, "Search by name", &g.formats.composer, 1e6)?;
                    self.field
                        .draw_text(g, dc, ox, oy, Some((&hint, &g.brushes.muted)));
                    unsafe { dc.PopAxisAlignedClip() };
                }
                _ => {}
            }
        }
        let (bx, by, bw, bh) = self.body;
        unsafe {
            dc.PushAxisAlignedClip(&rect(bx, by, bx + bw, by + bh), D2D1_ANTIALIAS_MODE_ALIASED);
        }
        let drawn = self.draw_rows(g, dc);
        unsafe { dc.PopAxisAlignedClip() };
        drawn
    }

    fn draw_rows(&self, g: &Gfx, dc: &ID2D1DeviceContext) -> Result<()> {
        let (_, by, _, bh) = self.body;
        let shown = self.browse_shown();
        for (part, r) in &self.parts {
            let (x, y, w, h) = *r;
            let Part::Channel(k) = *part else { continue };
            let Some(row) = shown.get(k) else { continue };
            if y > by + bh || y + h < by {
                continue;
            }
            if self.hover == Some(*part) {
                fill_round(dc, rect(x, y, x + w, y + h), 6.0, &g.brushes.hover);
            }
            let mark = if row.private { "\u{1F512}" } else { "#" };
            let l = layout(g, mark, &g.formats.channel, 40.0)?;
            crate::emoji::apply(&l, mark, 15.0, 0.9)?;
            crate::text::draw(
                dc,
                &l,
                x + 12.0,
                y + (h - height(&l)) / 2.0,
                &g.brushes.muted,
            );
            let right = if row.unread {
                x + w - 24.0
            } else {
                x + w - 12.0
            };
            let tw = (right - (x + 36.0)).max(1.0);
            let name = layout(g, &row.name, &g.formats.direct_name, tw)?;
            let status = if row.joined {
                "Joined"
            } else {
                "Read-only preview"
            };
            let note = layout(g, status, &g.formats.direct_note, tw)?;
            let top = y + (h - height(&name) - height(&note)) / 2.0;
            crate::text::draw(dc, &name, x + 36.0, top, &g.brushes.text);
            crate::text::draw(dc, &note, x + 36.0, top + height(&name), &g.brushes.muted);
            // egui's small text-coloured dot for a channel with anything new.
            if row.unread {
                let cy = y + h / 2.0;
                fill_round(
                    dc,
                    rect(x + w - 18.0, cy - 3.0, x + w - 12.0, cy + 3.0),
                    3.0,
                    &g.brushes.text,
                );
            }
        }
        Ok(())
    }
}

impl App {
    /// The space menu's Browse channels.
    pub fn open_browse(&mut self) {
        let Some(browse) = self.live.as_ref().and_then(|l| l.browse()) else {
            return;
        };
        self.open_dialog(Dialog::browse(browse));
    }

    /// A row opens its channel (a preview until you join it) and closes
    /// the dialog.
    pub fn browse_open(&mut self, k: usize) {
        let Some((space, channel)) = self.dialog.as_ref().and_then(|d| d.browse_row(k)) else {
            return;
        };
        self.close_dialog();
        if let Some(live) = &mut self.live {
            live.navigate(Some(space), Some(channel));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_ignores_leading_hashes_as_web_does() {
        let row = |name: &str| Row {
            id: name.into(),
            name: name.into(),
            private: false,
            joined: true,
            unread: false,
        };
        let browse = Browse {
            space: "s".into(),
            space_name: "Studio".into(),
            rows: vec![row("design"), row("dev"), row("c#-tips")],
        };
        let names = |q: &str| {
            browse
                .shown(q)
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(""), ["design", "dev", "c#-tips"]);
        assert_eq!(names(" ##DES "), ["design"]);
        assert_eq!(names("#de"), ["design", "dev"]);
        // Only a leading `#` goes; one inside the name still has to match.
        assert_eq!(names("c#"), ["c#-tips"]);
        assert!(names("#c-").is_empty());
    }
}
