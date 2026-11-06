//! The shared Twemoji catalog (`shared/emoji`): lookup, names, the
//! picker's categories, order and search, composer suggestions, and the
//! emoji graphemes in message text. Drawing stays in each client.
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
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

/// The picker's choices in web's order: `category`'s entries while `query`
/// (trimmed and lowercased) is empty, else every entry whose name, keywords
/// or category contain it. The last answer is kept, so a picker left open
/// does no catalog work each frame.
pub fn picker_choices(category: &str, query: &str) -> Arc<Vec<&'static Entry>> {
    type Choices = (String, String, Arc<Vec<&'static Entry>>);
    static LAST: Mutex<Option<Choices>> = Mutex::new(None);
    static SEARCHABLE: OnceLock<Vec<(String, String, String)>> = OnceLock::new();
    let mut last = LAST.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((cached_category, cached_query, choices)) = last.as_ref()
        && cached_category == category
        && cached_query == query
    {
        return choices.clone();
    }
    let searchable = SEARCHABLE.get_or_init(|| {
        catalog()
            .iter()
            .map(|entry| {
                (
                    entry.name.to_lowercase(),
                    entry.keywords.to_lowercase(),
                    entry.category.replace('_', " "),
                )
            })
            .collect()
    });
    let mut choices: Vec<_> = catalog()
        .iter()
        .zip(searchable)
        .filter(|(entry, (name, keywords, words))| {
            entry.selectable
                && (if query.is_empty() {
                    entry.category == category
                } else {
                    name.contains(query) || keywords.contains(query) || words.contains(query)
                })
        })
        .map(|(entry, _)| entry)
        .collect();
    choices.sort_by_key(|entry| picker_order(entry));
    let choices = Arc::new(choices);
    *last = Some((category.to_owned(), query.to_owned(), choices.clone()));
    choices
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

/// Catalog emoji that are one character (no U+FE0F), by that character: a
/// single-character grapheme can only be one of these.
fn singles() -> &'static HashMap<char, usize> {
    static SINGLES: OnceLock<HashMap<char, usize>> = OnceLock::new();
    SINGLES.get_or_init(|| {
        let mut singles = HashMap::new();
        for (position, entry) in catalog().iter().enumerate() {
            let mut characters = entry.emoji.chars();
            if let (Some(character), None) = (characters.next(), characters.next()) {
                singles.entry(character).or_insert(position);
            }
        }
        singles
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
        // Most non-ASCII text (accents, dashes) is single characters: one
        // lookup, without formatting code points. These never match an
        // emoji that needs U+FE0F.
        let mut characters = grapheme.chars();
        let entry = if let (Some(character), None) = (characters.next(), characters.next()) {
            singles().get(&character)
        } else {
            index()
                .get(&unicode_id(grapheme))
                .or_else(|| index().get(&grapheme.replace('\u{fe0f}', "")))
        }
        .map(|&position| &catalog()[position]);
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
    fn picker_choices_search_in_order_and_reuse_an_unchanged_answer() {
        let smileys = picker_choices("smileys_people", "");
        assert_eq!(smileys[0].emoji, "😀");
        assert!(
            smileys
                .iter()
                .all(|entry| entry.category == "smileys_people")
        );
        let found = picker_choices("smileys_people", "thumbs");
        assert!(found.iter().any(|entry| entry.id == "1f44d"));
        assert!(Arc::ptr_eq(
            &found,
            &picker_choices("smileys_people", "thumbs")
        ));
        assert!(picker_choices("flags", "zzzz-no-such-emoji").is_empty());
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
    fn suggestions_rank_names_before_keywords() {
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
    }
}
