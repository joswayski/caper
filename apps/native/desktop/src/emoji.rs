use eframe::egui;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Deserialize)]
pub struct Entry {
    pub id: String,
    pub emoji: String,
    pub name: String,
    pub keywords: String,
    pub category: String,
    pub selectable: bool,
    pub sheet: usize,
    pub x: u32,
    pub y: u32,
}

pub fn catalog() -> &'static [Entry] {
    static CATALOG: OnceLock<Vec<Entry>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_slice(include_bytes!("../../../../shared/emoji/catalog.json"))
            .expect("embedded emoji catalog is valid")
    })
}

pub fn find(value: &str) -> Option<&'static Entry> {
    let id = unicode_id(value);
    catalog().iter().find(|entry| entry.id == id)
}

/// The catalog's dash-separated name (`thumbs-up`) for a reaction emoji.
/// Reactions are stored fully qualified while the catalog keys some emoji
/// without U+FE0F, so the fallback compares with every U+FE0F removed.
/// Unoffered variants (such as skin tones) are named only by their code
/// points, which is not a name.
pub fn name(value: &str) -> Option<&'static str> {
    let bare = |emoji: &str| emoji.replace('\u{fe0f}', "");
    find(value)
        .or_else(|| {
            let wanted = bare(value);
            catalog().iter().find(|entry| bare(&entry.emoji) == wanted)
        })
        .filter(|entry| entry.name != entry.id)
        .map(|entry| entry.name.as_str())
}

pub fn unicode_id(value: &str) -> String {
    let joined = value
        .chars()
        .map(|character| format!("{:x}", character as u32))
        .collect::<Vec<_>>();
    if joined.iter().any(|part| part == "200d") {
        joined.join("-")
    } else {
        joined
            .into_iter()
            .filter(|part| part != "fe0f")
            .collect::<Vec<_>>()
            .join("-")
    }
}

#[derive(Default)]
pub struct Textures(BTreeMap<usize, egui::TextureHandle>);

impl Textures {
    pub fn image(&mut self, ui: &mut egui::Ui, entry: &Entry, size: f32) -> egui::Image<'static> {
        let texture = self.0.entry(entry.sheet).or_insert_with(|| {
            let decoded = image::load_from_memory(sheet(entry.sheet))
                .expect("embedded emoji sheet is valid")
                .to_rgba8();
            let dimensions = [decoded.width() as usize, decoded.height() as usize];
            ui.ctx().load_texture(
                format!("emoji-sheet-{}", entry.sheet),
                egui::ColorImage::from_rgba_unmultiplied(dimensions, decoded.as_raw()),
                egui::TextureOptions::LINEAR,
            )
        });
        let uv = egui::Rect::from_min_max(
            egui::pos2(entry.x as f32 / 1024.0, entry.y as f32 / 1024.0),
            egui::pos2(
                (entry.x + 64) as f32 / 1024.0,
                (entry.y + 64) as f32 / 1024.0,
            ),
        );
        egui::Image::new((texture.id(), egui::vec2(size, size))).uv(uv)
    }
}

fn sheet(page: usize) -> &'static [u8] {
    match page {
        0 => include_bytes!("../../../../shared/emoji/sheet-0.png"),
        1 => include_bytes!("../../../../shared/emoji/sheet-1.png"),
        2 => include_bytes!("../../../../shared/emoji/sheet-2.png"),
        3 => include_bytes!("../../../../shared/emoji/sheet-3.png"),
        4 => include_bytes!("../../../../shared/emoji/sheet-4.png"),
        5 => include_bytes!("../../../../shared/emoji/sheet-5.png"),
        6 => include_bytes!("../../../../shared/emoji/sheet-6.png"),
        7 => include_bytes!("../../../../shared/emoji/sheet-7.png"),
        8 => include_bytes!("../../../../shared/emoji/sheet-8.png"),
        9 => include_bytes!("../../../../shared/emoji/sheet-9.png"),
        10 => include_bytes!("../../../../shared/emoji/sheet-10.png"),
        11 => include_bytes!("../../../../shared/emoji/sheet-11.png"),
        12 => include_bytes!("../../../../shared/emoji/sheet-12.png"),
        13 => include_bytes!("../../../../shared/emoji/sheet-13.png"),
        14 => include_bytes!("../../../../shared/emoji/sheet-14.png"),
        _ => unreachable!("catalog references an unknown emoji sheet"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn twemoji_ids_follow_shared_rules() {
        assert_eq!(unicode_id("❤️"), "2764");
        assert_eq!(unicode_id("👩‍💻"), "1f469-200d-1f4bb");
        assert!(catalog().iter().filter(|entry| entry.selectable).count() >= 1870);
        assert!(find("👍").is_some());
    }

    #[test]
    fn reaction_names_ignore_variation_selectors() {
        for (emoji, expected) in [
            ("👍", "thumbs-up"),
            ("😂", "face-with-tears-of-joy"),
            ("🎉", "party-popper"),
            ("👀", "looking"),
            ("❤️", "red-heart"),
            ("❤", "red-heart"),
        ] {
            assert_eq!(name(emoji), Some(expected), "{emoji}");
        }
        // The catalog keeps U+FE0F in these ZWJ sequences; reactions written
        // without it still find the name.
        for emoji in [
            "\u{1f3c3}\u{200d}\u{2640}\u{fe0f}",
            "\u{1f3c3}\u{200d}\u{2640}",
        ] {
            assert_eq!(name(emoji), Some("woman-running"));
        }
        assert_eq!(name("\u{1f3f3}\u{200d}\u{1f308}"), Some("rainbow-flag"));
        assert_eq!(name("👍🏽"), None, "skin tones have no catalog name");
        assert_eq!(name("not an emoji"), None);
    }
}
