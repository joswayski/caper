//! Link preview cards: the text every client draws from a card's blocks.

use crate::attachments::EMBED_KEY;
use crate::links;
use crate::model::{Attachment, AttachmentKind, AttachmentStatus, Embed, EmbedBlock, EmbedImage};
use std::time::{Duration, Instant};

/// The card's name for its link: its title, else its author, else the site.
pub fn heading(embed: &Embed) -> &str {
    embed
        .blocks
        .iter()
        .find_map(|block| match block {
            EmbedBlock::Title { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .or_else(|| {
            embed.blocks.iter().find_map(|block| match block {
                EmbedBlock::Author { name, .. } => Some(name.as_str()),
                _ => None,
            })
        })
        .unwrap_or(&embed.site.name)
}

/// Web's `dateStyle: "medium"`, in local time.
fn footer_date(timestamp: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%b %-d, %Y")
                .to_string()
        })
}

pub fn footer_line(text: Option<&str>, timestamp: Option<&str>) -> String {
    [
        text.filter(|text| !text.is_empty()).map(str::to_owned),
        timestamp.and_then(footer_date),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// The media cache key for a card's unsigned image.
pub fn key(url: &str) -> String {
    format!("{EMBED_KEY}{url}")
}

/// An image block's items as viewer files: unsigned, ready, sized when known.
pub fn image_files(items: &[EmbedImage], name: &str) -> Vec<Attachment> {
    items
        .iter()
        .map(|image| Attachment {
            id: key(&image.url),
            spoiler: false,
            voice: false,
            waveform: None,
            transcript: None,
            kind: AttachmentKind::Image,
            // The decoders sniff the real format; this only picks the path.
            content_type: "image/jpeg".into(),
            name: image
                .alt
                .clone()
                .filter(|alt| !alt.is_empty())
                .unwrap_or_else(|| name.into()),
            size: 0,
            width: image.width,
            height: image.height,
            duration_ms: None,
            preview: true,
            url: Some(image.url.clone()),
            preview_url: Some(image.url.clone()),
            unavailable: false,
            status: AttachmentStatus::Ready,
            animated: false,
        })
        .collect()
}

/// A direct video as a viewer file; its poster shares the inline tile's
/// texture (both use the video's id).
pub fn video_file(block: &EmbedBlock, name: &str) -> Option<Attachment> {
    let EmbedBlock::Video {
        url,
        poster,
        width,
        height,
        duration_ms,
        gif,
    } = block
    else {
        return None;
    };
    Some(Attachment {
        id: key(url),
        spoiler: false,
        voice: false,
        waveform: None,
        transcript: None,
        kind: AttachmentKind::Video,
        content_type: if url.to_ascii_lowercase().contains(".webm") {
            "video/webm"
        } else {
            "video/mp4"
        }
        .into(),
        name: name.into(),
        size: 0,
        width: *width,
        height: *height,
        duration_ms: *duration_ms,
        preview: poster.is_some(),
        url: Some(url.clone()),
        preview_url: poster.clone(),
        unavailable: false,
        status: AttachmentStatus::Ready,
        animated: *gif,
    })
}

/// Media in a card, as web's `fit()`.
pub const MAX_WIDTH: f32 = 400.0;
pub const MAX_HEIGHT: f32 = 300.0;

/// Display size (width, height) that reserves layout space before media
/// loads; media of unknown size gets the full width at 16:9.
pub fn fit(width: Option<u32>, height: Option<u32>) -> [f32; 2] {
    match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            let (width, height) = (width as f32, height as f32);
            let scale = (MAX_WIDTH / width).min(MAX_HEIGHT / height).min(1.0);
            [
                (width * scale).round().max(1.0),
                (height * scale).round().max(1.0),
            ]
        }
        _ => [MAX_WIDTH, (MAX_WIDTH * 9.0 / 16.0).round()],
    }
}

/// [`fit`] narrowed to `available`, keeping its aspect ratio.
pub fn fit_within(size: [f32; 2], available: f32) -> [f32; 2] {
    if size[0] <= available || available <= 0.0 {
        size
    } else {
        [available, (size[1] * available / size[0]).round().max(1.0)]
    }
}

/// An image block's tiles in a card `width` wide, as (x, y, width, height):
/// one image fitted, two side by side, three with the first spanning both
/// rows, four in a grid.
pub fn gallery(items: &[EmbedImage], width: f32) -> Vec<[f32; 4]> {
    const GAP: f32 = 4.0;
    let half = ((width - GAP) / 2.0).max(1.0);
    let cell = |column: f32, row: f32, height: f32, rows: f32| {
        [
            column * (half + GAP),
            row * (height + GAP),
            half,
            height * rows + GAP * (rows - 1.0),
        ]
    };
    match items {
        [] => Vec::new(),
        [image] => {
            let [w, h] = fit_within(fit(image.width, image.height), width);
            vec![[0.0, 0.0, w, h]]
        }
        [_, _] => vec![cell(0.0, 0.0, 220.0, 1.0), cell(1.0, 0.0, 220.0, 1.0)],
        [_, _, _] => vec![
            cell(0.0, 0.0, 148.0, 2.0),
            cell(1.0, 0.0, 148.0, 1.0),
            cell(1.0, 1.0, 148.0, 1.0),
        ],
        _ => (0..items.len().min(4))
            .map(|index| cell((index % 2) as f32, (index / 2) as f32, 148.0, 1.0))
            .collect(),
    }
}

/// The composer waits this long after the link last changed before asking.
pub const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(400);

/// The first link in a draft that will get a card, for the composer preview
/// (web `previewLink`): links written `<like this>` get none.
pub fn preview_link(draft: &str) -> Option<String> {
    let segments = links::segments(draft);
    segments.iter().enumerate().find_map(|(index, segment)| {
        let href = segment.href.as_ref()?;
        let bracketed = index
            .checked_sub(1)
            .and_then(|previous| segments.get(previous))
            .is_some_and(|previous| previous.text.ends_with('<'))
            && segments
                .get(index + 1)
                .is_some_and(|next| next.text.starts_with('>'));
        (!bracketed).then(|| href.clone())
    })
}

/// The main composer's card for its first link (web `useLinkPreview`).
/// Removing it means the message is sent without previews; the choice lasts
/// while that link stays in the draft, and a sent or cleared draft starts
/// over.
#[derive(Default)]
pub struct ComposerPreview {
    /// The draft's preview link and when it became that link.
    link: Option<(String, Instant)>,
    /// The link last asked about.
    asked: Option<String>,
    /// The newest answer: a link and its card, if it has one.
    shown: Option<(String, Option<Embed>)>,
    removed: Option<String>,
}

impl ComposerPreview {
    /// Track the draft; returns a link to ask about now, or how long until
    /// one is due. Previews need a signed-in account.
    pub fn due(
        &mut self,
        draft: &str,
        signed_in: bool,
        now: Instant,
    ) -> (Option<String>, Option<Duration>) {
        if draft.is_empty() {
            self.removed = None;
        }
        let link = signed_in.then(|| preview_link(draft)).flatten();
        if self.link.as_ref().map(|(link, _)| link) != link.as_ref() {
            self.link = link.map(|link| (link, now));
        }
        let Some((link, since)) = &self.link else {
            return (None, None);
        };
        if self.removed.as_ref() == Some(link) || self.asked.as_ref() == Some(link) {
            return (None, None);
        }
        let waited = now.saturating_duration_since(*since);
        if waited < PREVIEW_DEBOUNCE {
            return (None, Some(PREVIEW_DEBOUNCE - waited));
        }
        self.asked = Some(link.clone());
        (Some(link.clone()), None)
    }

    /// An answer for `url` arrived.
    pub fn answer(&mut self, url: String, embed: Option<Embed>) {
        self.shown = Some((url, embed));
    }

    /// The card to show for the draft's current link.
    pub fn card(&self) -> Option<&Embed> {
        let (link, _) = self.link.as_ref()?;
        if self.removed.as_ref() == Some(link) {
            return None;
        }
        self.shown
            .as_ref()
            .filter(|(url, _)| url == link)
            .and_then(|(_, embed)| embed.as_ref())
    }

    pub fn remove(&mut self) {
        self.removed = self.link.as_ref().map(|(link, _)| link.clone());
    }

    /// Whether `text` goes without previews: its preview link was removed.
    pub fn removed(&self, text: &str) -> bool {
        self.removed
            .as_ref()
            .is_some_and(|removed| preview_link(text).as_ref() == Some(removed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preview_link_is_the_first_link_not_written_in_angle_brackets() {
        assert_eq!(preview_link("no links here"), None);
        assert_eq!(
            preview_link("see https://x.com/a and https://b.example/c"),
            Some("https://x.com/a".into())
        );
        assert_eq!(
            preview_link("skip <https://x.com/a> but https://b.example/c"),
            Some("https://b.example/c".into())
        );
        assert_eq!(preview_link("<https://x.com/a>"), None);
        assert_eq!(
            preview_link("<https://x.com/a and more"),
            Some("https://x.com/a".into()),
            "only a link closed by `>` is skipped"
        );
        assert_eq!(
            preview_link("www.example.com"),
            Some("https://www.example.com".into())
        );
    }

    #[test]
    fn composer_preview_waits_for_the_link_to_settle_and_remembers_removal() {
        let start = Instant::now();
        let mut preview = ComposerPreview::default();
        assert_eq!(
            preview.due("hi https://x.com/a", false, start),
            (None, None),
            "signed out"
        );
        let (url, wait) = preview.due("hi https://x.com/a", true, start);
        assert_eq!((url, wait), (None, Some(PREVIEW_DEBOUNCE)));
        // Typing after the link does not restart the wait; changing it does.
        let later = start + Duration::from_millis(300);
        assert_eq!(preview.due("hi https://x.com/a ok", true, later).0, None);
        let (url, _) = preview.due("hi https://x.com/a ok", true, start + PREVIEW_DEBOUNCE);
        assert_eq!(url.as_deref(), Some("https://x.com/a"));
        assert_eq!(
            preview.due("hi https://x.com/a ok", true, start + PREVIEW_DEBOUNCE * 2),
            (None, None),
            "asked once"
        );
        assert!(preview.card().is_none());
        let embed = Embed::parse(&json!({"version":1,"url":"https://x.com/a",
            "site":{"name":"X","domain":"x.com"},"blocks":[{"type":"text","text":"a"}]}));
        preview.answer("https://x.com/a".into(), embed.clone());
        assert_eq!(preview.card(), embed.as_ref());
        // An answer for an older link is not shown for the new one.
        let moved = start + PREVIEW_DEBOUNCE * 3;
        assert_eq!(
            preview.due("https://b.example/c", true, moved).1,
            Some(PREVIEW_DEBOUNCE)
        );
        assert!(preview.card().is_none());
        preview.remove();
        assert!(preview.removed("look https://b.example/c"));
        assert!(!preview.removed("look https://x.com/a"));
        assert_eq!(
            preview.due("https://b.example/c", true, moved + PREVIEW_DEBOUNCE),
            (None, None),
            "a removed link is not asked about"
        );
        // A sent or cleared draft starts over.
        preview.due("", true, moved + PREVIEW_DEBOUNCE);
        assert!(!preview.removed("https://b.example/c"));
    }

    #[test]
    fn media_fits_like_web_and_galleries_keep_to_the_card() {
        assert_eq!(fit(None, Some(10)), [400.0, 225.0]);
        assert_eq!(fit(Some(1280), Some(720)), [400.0, 225.0]);
        assert_eq!(fit(Some(300), Some(900)), [100.0, 300.0]);
        assert_eq!(fit(Some(200), Some(100)), [200.0, 100.0]);
        assert_eq!(fit_within([400.0, 225.0], 200.0), [200.0, 113.0]);
        let image = |n: usize| {
            (0..n)
                .map(|index| EmbedImage {
                    url: format!("https://e/{index}"),
                    width: None,
                    height: None,
                    alt: None,
                })
                .collect::<Vec<_>>()
        };
        let three = gallery(&image(3), 404.0);
        assert_eq!(three[0][3], 300.0, "the first of three spans both rows");
        assert_eq!(three[2][1], 152.0);
        assert!(
            gallery(&image(4), 404.0)
                .iter()
                .all(|r| r[0] + r[2] <= 404.0)
        );
        assert_eq!(gallery(&image(2), 404.0)[1][3], 220.0);
    }
}
