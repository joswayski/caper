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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub query: String,
}

fn query_character(value: char) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, '_' | '+' | '-')
}

// egui cursors count Unicode scalars, not UTF-8 bytes or grapheme clusters.
pub fn token(text: &str, caret: usize) -> Option<Token> {
    let chars: Vec<_> = text.chars().collect();
    if caret > chars.len()
        || chars
            .get(caret)
            .is_some_and(|c| *c == ':' || query_character(*c))
    {
        return None;
    }
    let mut start = caret;
    while start > 0 && query_character(chars[start - 1]) {
        start -= 1;
    }
    let colon = start.checked_sub(1)?;
    if chars[colon] != ':'
        || (colon > 0 && !chars[colon - 1].is_whitespace() && !"([{".contains(chars[colon - 1]))
    {
        return None;
    }
    Some(Token {
        start: colon,
        end: caret,
        query: chars[start..caret].iter().collect(),
    })
}

fn normalized(value: &str) -> String {
    value.to_lowercase().replace(['_', '-'], " ")
}

pub fn suggestions(query: &str) -> Vec<&'static Entry> {
    if query.is_empty() {
        return ["1f44d", "1f600", "2764", "1f389", "1f680", "1f440"]
            .iter()
            .filter_map(|id| {
                catalog()
                    .iter()
                    .find(|entry| entry.selectable && entry.id == *id)
            })
            .collect();
    }
    let needle = normalized(query);
    let mut matches: Vec<_> = catalog()
        .iter()
        .filter(|entry| entry.selectable)
        .filter_map(|entry| {
            let name = normalized(&entry.name);
            let keywords = normalized(&entry.keywords);
            let rank = if name == needle {
                0
            } else if name.starts_with(&needle) {
                1
            } else if keywords.starts_with(&needle) || keywords.contains(&format!(" {needle}")) {
                2
            } else if name.contains(&needle) || keywords.contains(&needle) {
                3
            } else {
                return None;
            };
            Some((rank, entry))
        })
        .collect();
    // Within a rank the shorter name is the closer match: ":fi" offers fire before film-frames.
    matches.sort_by_key(|(rank, entry)| (*rank, entry.name.chars().count()));
    matches
        .into_iter()
        .take(6)
        .map(|(_, entry)| entry)
        .collect()
}

pub fn insert(text: &str, token: &Token, emoji: &str) -> Option<(String, usize)> {
    let value: String = text
        .chars()
        .take(token.start)
        .chain(emoji.chars())
        .chain(text.chars().skip(token.end))
        .collect();
    (value.chars().count() <= 4_000).then(|| (value, token.start + emoji.chars().count()))
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
            ("👀", "eyes"),
            ("🔥", "fire"),
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

    #[test]
    fn autocomplete_boundaries_search_and_unicode_insertion() {
        for value in ["word:tom", "12:30", "https://tom", ":tom:"] {
            assert!(token(value, value.chars().count()).is_none(), "{value}");
        }
        assert!(token(":tomato", 4).is_none());
        assert!(token(":tom:", 4).is_none());
        assert!(token("hi (:tom", 8).is_some());
        assert!(suggestions("tom").iter().any(|entry| entry.emoji == "🍅"));
        assert_eq!(suggestions("tomato")[0].emoji, "🍅");
        assert_eq!(suggestions("thumbs_up")[0].emoji, "👍");
        assert_eq!(suggestions("+1")[0].emoji, "👍");
        assert_eq!(suggestions("WOMAN-TECHNOLOGIST")[0].emoji, "👩‍💻");
        assert_eq!(suggestions("red_heart")[0].emoji, "❤️");
        assert_eq!(suggestions("fire")[0].emoji, "🔥");
        assert_eq!(suggestions("dog")[0].emoji, "🐕");
        assert!(suggestions("fi").iter().any(|entry| entry.emoji == "🔥"));
        assert_eq!(
            suggestions("")
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["1f44d", "1f600", "2764", "1f389", "1f680", "1f440"]
        );
        let text = "👩‍💻 hi :roc suffix 🚀";
        let active = token(text, "👩‍💻 hi :roc".chars().count()).unwrap();
        assert_eq!(
            insert(text, &active, "🚀"),
            Some(("👩‍💻 hi 🚀 suffix 🚀".into(), "👩‍💻 hi 🚀".chars().count()))
        );
        let text = format!("{} :x", "😀".repeat(3998));
        let active = token(&text, text.chars().count()).unwrap();
        assert!(insert(&text, &active, "🚀").is_some());
        assert!(insert(&text, &active, "👩‍💻").is_none());
    }
}
