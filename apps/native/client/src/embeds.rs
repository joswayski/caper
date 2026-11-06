//! Link preview cards: the text every client draws from a card's blocks.

use crate::attachments::EMBED_KEY;
use crate::model::{Attachment, AttachmentKind, AttachmentStatus, Embed, EmbedBlock, EmbedImage};

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

#[cfg(test)]
mod tests {
    use super::*;

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
