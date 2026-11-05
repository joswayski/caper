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
