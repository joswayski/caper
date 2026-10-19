//! Which characters short names may hold: display names, space names, guest
//! roster names and attachment file names. Message text is not checked here,
//! since people legitimately paste right-to-left and formatted text.
//!
//! A name keeps every visible character but loses control characters and the
//! Unicode `Default_Ignorable_Code_Point` set (bidi embeddings, overrides,
//! isolates and marks, zero-width characters, soft hyphen, Hangul fillers,
//! tag characters and so on), plus U+2800 BRAILLE PATTERN BLANK, which
//! renders as a blank. Those characters let a name look empty, hide text, or
//! reorder what it shows. Three of them have real uses and stay where they
//! do that job:
//!
//! - U+200D ZERO WIDTH JOINER and U+200C ZERO WIDTH NON-JOINER (emoji
//!   sequences, Indic and Persian spelling): one at a time, between two
//!   visible characters.
//! - Variation selectors (U+FE00–U+FE0F, U+E0100–U+E01EF and the Mongolian
//!   U+180B–U+180D, U+180F): one, directly after a visible character.
//! - Tag characters (U+E0020–U+E007E): only in an emoji subdivision flag,
//!   directly after U+1F3F4 and ended by U+E007F CANCEL TAG.
//!
//! Clients mirror this rule; keep `apps/web/src/names.ts`, the Android
//! `Names.kt`, Apple's `NameCharacters` and desktop's `names.rs` in step.

const FLAG: char = '\u{1F3F4}';
const CANCEL_TAG: char = '\u{E007F}';

fn joiner(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}')
}

fn selector(c: char) -> bool {
    matches!(c, '\u{180B}'..='\u{180D}' | '\u{180F}' | '\u{FE00}'..='\u{FE0F}' | '\u{E0100}'..='\u{E01EF}')
}

fn tag(c: char) -> bool {
    matches!(c, '\u{E0020}'..='\u{E007E}')
}

/// Control characters, `Default_Ignorable_Code_Point` (Unicode 16) and the
/// braille blank. The allowed uses above are handled before this is asked.
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{115F}'..='\u{1160}'
                | '\u{17B4}'..='\u{17B5}'
                | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{2800}'
                | '\u{3164}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF0}'..='\u{FFF8}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{1D173}'..='\u{1D17A}'
                | '\u{E0000}'..='\u{E0FFF}'
        )
}

#[derive(Clone, Copy, PartialEq)]
enum Last {
    Start,
    Space,
    Visible,
    Flag,
    Selector,
}

/// `value` without the characters a name may not hold (see the module docs).
pub(crate) fn without_hidden(value: &str) -> String {
    let mut kept = String::with_capacity(value.len());
    let mut last = Last::Start;
    // A joiner waits for the visible character it joins; a flag's tags wait
    // for their cancel tag. Either is dropped when that never comes.
    let mut pending_joiner = None;
    let mut tags = String::new();
    for c in value.chars() {
        let joined = pending_joiner.take();
        if tag(c) {
            if last == Last::Flag || !tags.is_empty() {
                tags.push(c);
            }
            continue;
        }
        if c == CANCEL_TAG {
            if !tags.is_empty() {
                kept.push_str(&tags);
                kept.push(c);
                last = Last::Visible;
            }
            tags.clear();
            continue;
        }
        tags.clear();
        if joiner(c) {
            if joined.is_none() && matches!(last, Last::Visible | Last::Flag | Last::Selector) {
                pending_joiner = Some(c);
            }
        } else if selector(c) {
            if matches!(last, Last::Visible | Last::Flag) {
                kept.push(c);
                last = Last::Selector;
            }
        } else if hidden(c) {
        } else if c.is_whitespace() {
            kept.push(c);
            last = Last::Space;
        } else {
            kept.extend(joined);
            kept.push(c);
            last = if c == FLAG { Last::Flag } else { Last::Visible };
        }
    }
    kept
}

/// Whether every character of `value` may appear in a name.
pub(crate) fn visible(value: &str) -> bool {
    without_hidden(value) == value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_keep_visible_text_emoji_and_joined_scripts() {
        for name in [
            "Café Society",
            "José 🌱",
            "مرحبا بالعالم",
            "שלום",
            "👨‍👩‍👧‍👦 family",
            "👩🏽‍💻",
            "❤️‍🔥",
            "🏳️‍🌈",
            "🏳️‍⚧️",
            "1️⃣",
            "☺︎ text style",
            "🏴\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F} England",
            "می\u{200C}خواهم",
            "क्\u{200D}ष",
            "葛\u{E0100}城",
            "Screenshot at 10.00.00\u{202F}AM",
            "a b",
        ] {
            assert!(visible(name), "{name:?}");
        }
    }

    #[test]
    fn names_reject_invisible_and_direction_characters() {
        for name in [
            "a\u{0}b",
            "a\nb",
            "admin\u{202E}nimda",
            "\u{202A}x\u{202C}",
            "\u{202B}x",
            "\u{202D}x",
            "\u{2066}x\u{2069}",
            "\u{2067}x",
            "\u{2068}x",
            "a\u{200B}b",
            "\u{200E}left",
            "\u{200F}right",
            "\u{061C}x",
            "a\u{2060}b",
            "a\u{2061}b",
            "a\u{2064}b",
            "\u{FEFF}bom",
            "soft\u{00AD}hyphen",
            "a\u{034F}b",
            "\u{115F}",
            "\u{1160}",
            "\u{3164}",
            "\u{FFA0}",
            "\u{2800}",
            "a\u{180E}b",
            "hidden\u{E0041}\u{E0042}",
            "\u{E0001}x",
            "\u{FE0F}",
            "a \u{FE0F}",
            "a\u{FE0F}\u{FE0F}",
            "a\u{E0100}\u{E0101}",
            "\u{200D}a",
            "a\u{200D}",
            "a\u{200D}\u{200D}b",
            "a \u{200D}b",
            "a\u{200D} b",
            "a\u{200C}",
            "🏴\u{E0067}\u{E0062}",
            "🏴\u{E007F}",
            "x\u{E0067}\u{E007F}",
            "\u{1D173}x",
            "\u{1BCA0}x",
        ] {
            assert!(!visible(name), "{name:?}");
        }
    }

    #[test]
    fn stripping_keeps_what_a_name_may_hold() {
        assert_eq!(without_hidden("invoice\u{202E}fdp.exe"), "invoicefdp.exe");
        assert_eq!(without_hidden("\u{200E}IMG-0001.jpg"), "IMG-0001.jpg");
        assert_eq!(without_hidden("a\u{200D}\u{200D}b"), "ab");
        assert_eq!(without_hidden("a\u{FE0F}\u{FE0F}"), "a\u{FE0F}");
        assert_eq!(without_hidden("🏴\u{E0067}\u{E0062} x"), "🏴 x");
        assert_eq!(without_hidden("👨‍👩‍👧"), "👨‍👩‍👧");
        for value in [
            "a\u{200D}\u{200B}b",
            "\u{200D}\u{FE0F}x",
            "🏴\u{E0067}\u{200D}y",
        ] {
            assert!(visible(&without_hidden(value)), "{value:?}");
        }
    }
}
