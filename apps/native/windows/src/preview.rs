//! A composer's card for its draft's first link, as the egui client draws
//! it (web `DraftEmbed`): inside the box under the text, a thumbnail, the
//! site, the title and the first line of text, with Remove to send the
//! message without previews. The main composer and the open thread's reply
//! box each have one. The shared `caper_client::embeds::ComposerPreview`
//! decides which link to ask about (400 ms after it last changed) and keeps
//! the removal while that link stays in the draft.
use crate::files::{alpha, cover, line};
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::theme::*;
use crate::{App, EV_SCREEN};
use caper_client::embeds::{heading, key};
use caper_client::media::ImageState;
use caper_client::model::{Embed, EmbedBlock};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};
use windows::core::Result;

/// Asks about the draft's link once it has stayed put for the debounce.
pub const TIMER_PREVIEW: usize = 16;
const CARD_H: f32 = 72.0;
const THUMB: f32 = 56.0;
/// The card and the margin under it, below the composer's text.
pub const PREVIEW_H: f32 = CARD_H + 8.0;

/// The picture the card shows: a thumbnail image, the first image, a
/// player's or video's poster, or the author's icon (egui's order).
pub fn image(embed: &Embed) -> Option<&str> {
    let images = || {
        embed.blocks.iter().filter_map(|block| match block {
            EmbedBlock::Images { items, thumbnail } => Some((items, *thumbnail)),
            _ => None,
        })
    };
    let poster = |player: bool| {
        embed.blocks.iter().find_map(|block| match block {
            EmbedBlock::Player { poster, .. } if player => poster.as_deref(),
            EmbedBlock::Video { poster, .. } if !player => poster.as_deref(),
            _ => None,
        })
    };
    images()
        .find(|(_, thumbnail)| *thumbnail)
        .or_else(|| images().next())
        .and_then(|(items, _)| items.first())
        .map(|image| image.url.as_str())
        .or_else(|| poster(true))
        .or_else(|| poster(false))
        .or_else(|| {
            embed.blocks.iter().find_map(|block| match block {
                EmbedBlock::Author { icon, .. } => icon.as_deref(),
                _ => None,
            })
        })
}

/// The card in a box `bw` by `bh` (box-local).
pub fn card_rect(bw: f32, bh: f32) -> D2D_RECT_F {
    rect(8.0, bh - PREVIEW_H, bw - 8.0, bh - 8.0)
}

/// Remove, over the card's top-right corner.
pub fn remove_rect(card: D2D_RECT_F) -> D2D_RECT_F {
    rect(
        card.right - 28.0,
        card.top + 4.0,
        card.right - 4.0,
        card.top + 28.0,
    )
}

/// Draws the card `card` with its loaded `thumb`.
pub fn draw(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    embed: &Embed,
    thumb: Option<&ID2D1Bitmap1>,
    card: D2D_RECT_F,
    remove_hover: bool,
) -> Result<()> {
    fill_round(dc, card, 8.0, &g.solid(SURFACE_RAISED));
    stroke_round(dc, card, 8.0, &g.brushes.border);
    let mut x = card.left + 8.0;
    if image(embed).is_some() {
        let t = rect(x, card.top + 8.0, x + THUMB, card.top + 8.0 + THUMB);
        fill_round(dc, t, 6.0, &alpha(dc, SURFACE_COMPOSER, 1.0)?);
        if let Some(b) = thumb {
            cover(dc, b, t, 6.0)?;
        }
        x = t.right + 10.0;
    }
    let width = (card.right - 32.0 - x).max(1.0);
    let site = if embed.site.name != embed.site.domain && !embed.site.domain.is_empty() {
        format!("{}  {}", embed.site.name, embed.site.domain)
    } else {
        embed.site.name.clone()
    };
    let text = embed.blocks.iter().find_map(|block| match block {
        EmbedBlock::Text { text } => Some(text.split_whitespace().collect::<Vec<_>>().join(" ")),
        _ => None,
    });
    let mut y = card.top + 8.0;
    for (s, format, brush) in [
        (Some(site), &g.formats.embed_meta, &g.brushes.muted),
        (
            Some(heading(embed).to_owned()),
            &g.formats.embed_title,
            &g.brushes.text,
        ),
        (text, &g.formats.embed_text, &g.brushes.muted),
    ] {
        let Some(s) = s.filter(|s| !s.is_empty()) else {
            continue;
        };
        let l = line(g, &s, format, width)?;
        unsafe {
            l.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let sign = DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                delimiter: 0,
                delimiterCount: 0,
            };
            l.SetTrimming(&sign, &g.dwrite.CreateEllipsisTrimmingSign(format)?)?;
        }
        crate::text::draw(dc, &l, x, y, brush);
        y += gfx::metrics(&l).height + 2.0;
    }
    let r = remove_rect(card);
    fill_round(dc, r, 8.0, &g.solid(SURFACE_RAISED));
    stroke_round(dc, r, 8.0, &g.brushes.border);
    icons::draw(
        dc,
        Icon::Close,
        rect(r.left + 6.0, r.top + 6.0, r.right - 6.0, r.bottom - 6.0),
        if remove_hover { TEXT } else { TEXT_MUTED },
        1.0,
    );
    Ok(())
}

impl App {
    /// The card for the main composer's draft, when one is shown.
    pub fn preview_card(&self) -> Option<&Embed> {
        if self.dm_bar().is_some() || self.rec().is_some() || !self.c.is_live() {
            return None;
        }
        self.live.as_ref()?.preview.card()
    }

    /// The card for the open thread's reply draft, when one is shown.
    pub fn thread_preview_card(&self) -> Option<&Embed> {
        self.thread.as_ref()?;
        self.live.as_ref()?.thread_preview.card()
    }

    /// The main card's thumbnail (the reply box's with `thread`) once loaded
    /// (asking the media cache for it).
    pub fn preview_thumb(&mut self, thread: bool) -> Option<ID2D1Bitmap1> {
        let live = self.live.as_mut()?;
        let card = if thread {
            live.thread_preview.card()
        } else {
            live.preview.card()
        };
        let url = image(card?)?.to_owned();
        let k = key(&url);
        if matches!(live.media.image(&k, Some(&url)), ImageState::Ready(_)) {
            live.media.texture(&k).cloned()
        } else {
            None
        }
    }

    /// Tracks the main draft and the open thread's reply for their cards:
    /// asks once the debounce has passed, and lays a composer out again
    /// when its card changed. With no thread open its card starts over.
    pub fn preview_tick(&mut self) {
        let draft = String::from_utf16_lossy(&self.composer.text);
        let reply = self
            .thread
            .as_ref()
            .map(|t| String::from_utf16_lossy(&t.composer.text))
            .unwrap_or_default();
        let Some(live) = self.live.as_mut() else {
            return;
        };
        let (changed, wait) = live.preview_tick(false, &draft);
        let (thread_changed, thread_wait) = live.preview_tick(true, &reply);
        // One timer serves both: it fires for whichever is due first.
        let wait = [wait, thread_wait].into_iter().flatten().min();
        unsafe {
            match wait {
                Some(wait) => {
                    SetTimer(
                        Some(self.hwnd),
                        TIMER_PREVIEW,
                        (wait.as_millis() + 1).min(u32::MAX as u128) as u32,
                        None,
                    );
                }
                None => {
                    let _ = KillTimer(Some(self.hwnd), TIMER_PREVIEW);
                }
            }
        }
        if changed {
            self.preview_changed();
        }
        if thread_changed {
            self.thread_box_changed();
        }
    }

    /// The card appeared, went or changed.
    pub fn preview_changed(&mut self) {
        self.dirty.layout = true;
        self.dirty.composer = true;
        self.events |= EV_SCREEN;
        self.need = true;
    }

    /// Remove: the message goes without previews while this link stays.
    pub fn remove_preview(&mut self) {
        if let Some(live) = &mut self.live {
            live.preview.remove();
        }
        self.preview_changed();
    }

    /// The reply box's Remove.
    pub fn remove_thread_preview(&mut self) {
        if let Some(live) = &mut self.live {
            live.thread_preview.remove();
        }
        self.thread_box_changed();
    }
}
