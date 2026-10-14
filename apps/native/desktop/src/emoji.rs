use eframe::egui;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use unicode_segmentation::UnicodeSegmentation;

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

pub const CATEGORIES: [(&str, &str, &str); 8] = [
    ("smileys_people", "Smileys & people", "😀"),
    ("animals_nature", "Animals & nature", "🐱"),
    ("food_drink", "Food & drink", "🍔"),
    ("travel_places", "Travel & places", "🚌"),
    ("activities", "Activities", "⚽"),
    ("objects", "Objects", "👕"),
    ("symbols", "Symbols", "🎵"),
    ("flags", "Flags", "🏁"),
];

/// An entry's place in the emoji picker, approximating web's order: smileys
/// (the emoticon block, then other faces by name or keyword) before people,
/// then the other categories with flags last. The catalog itself is in code
/// point order.
pub fn picker_order(entry: &Entry) -> (usize, usize) {
    let emoticon = u32::from_str_radix(entry.id.split('-').next().unwrap_or_default(), 16)
        .is_ok_and(|point| (0x1f600..=0x1f64f).contains(&point));
    let face = if entry.category != "smileys_people" || emoticon {
        0
    } else if entry.name.contains("face") || entry.keywords.split(' ').any(|word| word == "face") {
        1
    } else {
        2
    };
    (picker_rank(&entry.category), face)
}

/// A category's place in the emoji picker, as on web: smileys and people
/// first, flags last, uncategorized entries after them.
pub fn picker_rank(category: &str) -> usize {
    const ORDER: [&str; 8] = [
        "smileys_people",
        "animals_nature",
        "food_drink",
        "travel_places",
        "activities",
        "objects",
        "symbols",
        "flags",
    ];
    ORDER
        .iter()
        .position(|item| *item == category)
        .unwrap_or(ORDER.len())
}

pub fn find(value: &str) -> Option<&'static Entry> {
    let id = unicode_id(value);
    catalog().iter().find(|entry| entry.id == id)
}

/// The catalog by Twemoji id, and by its emoji without U+FE0F, for text.
fn index() -> &'static HashMap<String, usize> {
    static INDEX: OnceLock<HashMap<String, usize>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut index = HashMap::new();
        for (position, entry) in catalog().iter().enumerate() {
            index.entry(entry.id.clone()).or_insert(position);
            index
                .entry(entry.emoji.replace('\u{fe0f}', ""))
                .or_insert(position);
        }
        index
    })
}

/// Byte ranges of the emoji graphemes in message text, with their artwork,
/// so text can draw them in color like the other clients. Plain ASCII
/// (digits, `#`, `*`) is never an emoji without its keycap or variation mark,
/// and neither is a text-default character (© ™ ♥ ➡), whose catalog emoji
/// carries U+FE0F, without its U+FE0F: web shows those as text.
pub fn inline(text: &str) -> Vec<(std::ops::Range<usize>, &'static Entry)> {
    if text.is_ascii() {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (start, grapheme) in text.grapheme_indices(true) {
        if grapheme.is_ascii() {
            continue;
        }
        let single = grapheme.chars().nth(1).is_none();
        let entry = index()
            .get(&unicode_id(grapheme))
            .or_else(|| index().get(&grapheme.replace('\u{fe0f}', "")))
            .map(|&position| &catalog()[position])
            .filter(|entry| !(single && entry.emoji.contains('\u{fe0f}')));
        if let Some(entry) = entry {
            found.push((start..start + grapheme.len(), entry));
        }
    }
    found
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

/// Observe the editor's rendered glyphs without adding interactive widgets or changing selection.
pub fn editor_tooltip(output: &egui::text_edit::TextEditOutput, text: &str) {
    let response = &output.response;
    let hold_id = response.id.with("emoji-name-hold");
    let held = response
        .ctx
        .data_mut(|data| data.remove_temp::<(egui::Pos2, String)>(hold_id));
    let Some(pointer) = response.ctx.input(|input| input.pointer.hover_pos()) else {
        return;
    };
    if !response.rect.contains(pointer) || !output.text_clip_rect.contains(pointer) {
        return;
    }
    let local = pointer - output.galley_pos;
    let mut scalar = 0;
    let mut hit = None;
    for row in &output.galley.rows {
        for glyph in &row.glyphs {
            if glyph
                .logical_rect()
                .translate(row.pos.to_vec2())
                .contains(local.to_pos2())
            {
                hit = Some(scalar);
                break;
            }
            scalar += 1;
        }
        if hit.is_some() {
            break;
        }
        scalar += usize::from(row.ends_with_newline);
    }
    let Some(hit) = hit else {
        return;
    };
    let mut start = 0;
    for grapheme in text.graphemes(true) {
        let end = start + grapheme.chars().count();
        if (start..end).contains(&hit) {
            if let Some(name) = name(grapheme) {
                let label = format!(":{name}:");
                let holding = response
                    .ctx
                    .input(|input| input.pointer.any_down() && !input.pointer.any_pressed())
                    && held.as_ref().is_some_and(|(origin, previous)| {
                        previous == &label && origin.distance(pointer) <= 10.0
                    });
                // long_touched is a one-frame event; keep displaying during the hold.
                if response.long_touched() || holding {
                    response.ctx.data_mut(|data| {
                        data.insert_temp(hold_id, held.unwrap_or((pointer, label.clone())));
                    });
                    response.show_tooltip_text(label);
                } else {
                    response.clone().on_hover_text_at_pointer(label);
                }
            }
            break;
        }
        start = end;
    }
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
    // One character is an emoticon (":D", ":P", ":3"), not a search; Enter sends it as typed.
    if caret - start == 1 {
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

/// The bundled sheets, uploaded once and shared: reactions, the picker and
/// message text (through `shared`) draw from the same textures.
#[derive(Default, Clone)]
pub struct Textures(Arc<Mutex<BTreeMap<usize, egui::TextureHandle>>>);

impl Textures {
    /// The context's shared sheets, so free functions drawing text reuse them.
    pub fn shared(context: &egui::Context) -> Self {
        let id = egui::Id::new("emoji-textures");
        context.data_mut(|data| data.get_temp_mut_or_default::<Self>(id).clone())
    }

    fn texture(&self, context: &egui::Context, page: usize) -> egui::TextureHandle {
        let mut sheets = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sheets
            .entry(page)
            .or_insert_with(|| {
                let decoded = image::load_from_memory(sheet(page))
                    .expect("embedded emoji sheet is valid")
                    .to_rgba8();
                let dimensions = [decoded.width() as usize, decoded.height() as usize];
                context.load_texture(
                    format!("emoji-sheet-{page}"),
                    egui::ColorImage::from_rgba_unmultiplied(dimensions, decoded.as_raw()),
                    egui::TextureOptions::LINEAR,
                )
            })
            .clone()
    }

    /// Decode/upload only the opening window's bundled sheets. Repeated load,
    /// hover and focus intent reuses the same textures, without network work.
    pub fn preload(&self, context: &egui::Context, category: &str) {
        let mut entries: Vec<_> = catalog()
            .iter()
            .filter(|entry| entry.selectable && entry.category == category)
            .collect();
        entries.sort_by_key(|entry| picker_order(entry));
        for entry in entries.into_iter().take(128) {
            let _ = self.texture(context, entry.sheet);
        }
    }

    pub fn image(&self, ui: &egui::Ui, entry: &Entry, size: f32) -> egui::Image<'static> {
        let texture = self.texture(ui.ctx(), entry.sheet);
        let uv = Self::uv(entry);
        egui::Image::new((texture.id(), egui::vec2(size, size))).uv(uv)
    }

    /// Paints `entry` in color over `rect`, as message text does.
    pub fn paint(&self, painter: &egui::Painter, entry: &Entry, rect: egui::Rect) {
        let texture = self.texture(painter.ctx(), entry.sheet);
        painter.image(texture.id(), rect, Self::uv(entry), egui::Color32::WHITE);
    }

    fn uv(entry: &Entry) -> egui::Rect {
        egui::Rect::from_min_max(
            egui::pos2(entry.x as f32 / 1024.0, entry.y as f32 / 1024.0),
            egui::pos2(
                (entry.x + 64) as f32 / 1024.0,
                (entry.y + 64) as f32 / 1024.0,
            ),
        )
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
    fn inline_finds_emoji_graphemes_with_and_without_variation_marks() {
        let text = "fixed ☕ then 🙌🏽 and 👍 and ☺️ at 3 # 1";
        let found: Vec<_> = inline(text)
            .into_iter()
            .map(|(range, entry)| (&text[range], entry.id.as_str()))
            .collect();
        assert_eq!(
            found,
            vec![
                ("☕", "2615"),
                ("🙌🏽", "1f64c-1f3fd"),
                ("👍", "1f44d"),
                ("☺️", "263a"),
            ]
        );
        // Plain digits and punctuation are text, not keycaps.
        assert!(inline("Room 3 # 1 * 2").is_empty());
        assert!(inline("café — naïve").is_empty());
    }

    #[test]
    fn text_default_characters_are_emoji_only_with_their_variation_mark() {
        for text in ["©", "®", "™", "♥", "✔", "➡", "▶", "☺"] {
            assert!(inline(text).is_empty(), "{text} stays text");
            let colored = format!("{text}\u{fe0f}");
            assert_eq!(inline(&colored).len(), 1, "{colored} is an emoji");
        }
        let text = "© 2026 ™ ♥ ❤️ ☕";
        let found: Vec<_> = inline(text)
            .into_iter()
            .map(|(range, _)| &text[range])
            .collect();
        assert_eq!(found, vec!["❤️", "☕"]);
    }

    #[test]
    fn picker_order_starts_with_smileys_and_ends_with_flags() {
        let mut choices: Vec<_> = catalog().iter().filter(|entry| entry.selectable).collect();
        choices.sort_by_key(|entry| picker_order(entry));
        assert_eq!(choices[0].emoji, "😀");
        assert_eq!(choices.last().unwrap().category, "flags");
        let people = choices
            .iter()
            .position(|entry| entry.emoji == "👍")
            .unwrap();
        let faces = choices
            .iter()
            .position(|entry| entry.emoji == "🤣")
            .unwrap();
        assert!(faces < people, "faces come before people, as on web");
    }
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
    fn composer_names_follow_graphemes_and_persist_during_a_touch_hold() {
        let context = egui::Context::default();
        context.style_mut(|style| style.interaction.tooltip_delay = 0.0);
        let original = "prefix 🚀\n👩‍💻 🎎 suffix";
        let mut text = original.to_owned();
        let mut frame = |time, events| {
            let mut editor = None;
            let painted = context.run(
                egui::RawInput {
                    time: Some(time),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 400.0),
                    )),
                    events,
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        let output = egui::TextEdit::multiline(&mut text).show(ui);
                        editor_tooltip(&output, &text);
                        editor = Some(output);
                    });
                },
            );
            (painted, editor.unwrap())
        };
        let (_, output) = frame(0.0, vec![]);
        let point = |prefix: &str| {
            output.galley_pos
                + output
                    .galley
                    .pos_from_cursor(egui::text::CCursor::new(prefix.chars().count()))
                    .center()
                    .to_vec2()
                + egui::vec2(3.0, 0.0)
        };
        let has_label = |painted: &egui::FullOutput, label| {
            painted.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == label)
            })
        };
        for (time, prefix, label) in [
            (1.0, "prefix ", ":rocket:"),
            (2.0, "prefix 🚀\n", ":woman-technologist:"),
            (3.0, "prefix 🚀\n👩‍💻 ", ":Japanese-dolls:"),
        ] {
            frame(time, vec![egui::Event::PointerMoved(point(prefix))]);
            frame(time + 0.1, vec![]);
            assert!(has_label(&frame(time + 0.2, vec![]).0, label), "{label}");
        }
        frame(4.0, vec![egui::Event::PointerMoved(point("pre"))]);
        assert!(!has_label(&frame(4.1, vec![]).0, ":Japanese-dolls:"));
        let pos = point("prefix 🚀\n👩‍💻 ");
        frame(
            5.0,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::Touch {
                    device_id: egui::TouchDeviceId(1),
                    id: egui::TouchId(1),
                    phase: egui::TouchPhase::Start,
                    pos,
                    force: None,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        frame(6.0, vec![]);
        assert!(has_label(&frame(6.1, vec![]).0, ":Japanese-dolls:"));
        assert!(has_label(&frame(6.4, vec![]).0, ":Japanese-dolls:"));
        assert_eq!(text, original, "inspection never changes the draft");
    }

    #[test]
    fn autocomplete_boundaries_search_and_unicode_insertion() {
        for value in ["word:tom", "12:30", "https://tom", ":tom:"] {
            assert!(token(value, value.chars().count()).is_none(), "{value}");
        }
        assert!(token(":tomato", 4).is_none());
        assert!(token(":tom:", 4).is_none());
        assert!(token("hi (:tom", 8).is_some());
        for text in [":D", "ok :P", "hi (:3"] {
            assert!(token(text, text.chars().count()).is_none(), "{text}");
        }
        assert!(token(":", 1).is_some_and(|token| token.query.is_empty()));
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
        let text = format!("{} :xy", "😀".repeat(3998));
        let active = token(&text, text.chars().count()).unwrap();
        assert!(insert(&text, &active, "🚀").is_some());
        assert!(insert(&text, &active, "👩‍💻").is_none());
    }
}
