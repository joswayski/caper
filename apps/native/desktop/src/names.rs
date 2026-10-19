//! The API's name-character rule (apps/api/src/names.rs): names (display and
//! space names) keep visible text but not control characters, Unicode
//! `Default_Ignorable_Code_Point` characters (bidi controls, zero-width
//! characters, soft hyphen, Hangul fillers, tags…) or the braille blank. Emoji
//! keep their parts: a lone ZWJ/ZWNJ between visible characters, one variation
//! selector after a visible character, and tags inside a subdivision flag.
//! Message text is not checked by this rule.

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

/// Whether `value` holds a character a name may not (see above).
pub(crate) fn has_hidden(value: &str) -> bool {
    let mut last = Last::Start;
    let mut joined = false;
    let mut tags = 0;
    for c in value.chars() {
        if tags > 0 && !tag(c) && c != CANCEL_TAG {
            return true;
        }
        if joined
            && (joiner(c)
                || selector(c)
                || tag(c)
                || c == CANCEL_TAG
                || hidden(c)
                || c.is_whitespace())
        {
            return true;
        }
        joined = false;
        if tag(c) {
            if last != Last::Flag && tags == 0 {
                return true;
            }
            tags += 1;
        } else if c == CANCEL_TAG {
            if tags == 0 {
                return true;
            }
            tags = 0;
            last = Last::Visible;
        } else if joiner(c) {
            if !matches!(last, Last::Visible | Last::Flag | Last::Selector) {
                return true;
            }
            joined = true;
        } else if selector(c) {
            if !matches!(last, Last::Visible | Last::Flag) {
                return true;
            }
            last = Last::Selector;
        } else if hidden(c) {
            return true;
        } else if c.is_whitespace() {
            last = Last::Space;
        } else {
            last = if c == FLAG { Last::Flag } else { Last::Visible };
        }
    }
    joined || tags > 0
}

/// Shown before saving a display name; `saved` (the account's current name)
/// stays savable unchanged even if it predates the invisible-character rule.
pub(crate) fn display_name_error(value: &str, saved: Option<&str>) -> Option<&'static str> {
    let name = value.trim();
    (Some(name) != saved.map(str::trim) && has_hidden(name))
        .then_some("Display names cannot contain invisible or text-direction characters.")
}

#[cfg(test)]
mod tests {
    use super::*;

    // The same cases as apps/api/src/names.rs.
    #[test]
    fn names_keep_visible_text_emoji_and_joined_scripts() {
        for name in [
            "Café Society",
            "José 🌱",
            "مرحبا بالعالم",
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
        ] {
            assert!(!has_hidden(name), "{name:?}");
        }
    }

    #[test]
    fn names_reject_invisible_and_direction_characters() {
        for name in [
            "a\u{0}b",
            "admin\u{202E}nimda",
            "\u{202A}x\u{202C}",
            "\u{2066}x\u{2069}",
            "a\u{200B}b",
            "\u{200E}left",
            "\u{200F}right",
            "\u{061C}x",
            "a\u{2060}b",
            "\u{FEFF}bom",
            "soft\u{00AD}hyphen",
            "a\u{034F}b",
            "\u{115F}",
            "\u{3164}",
            "\u{FFA0}",
            "\u{2800}",
            "a\u{180E}b",
            "hidden\u{E0041}\u{E0042}",
            "\u{FE0F}",
            "a \u{FE0F}",
            "a\u{FE0F}\u{FE0F}",
            "\u{200D}a",
            "a\u{200D}",
            "a\u{200D}\u{200D}b",
            "a \u{200D}b",
            "a\u{200D} b",
            "🏴\u{E0067}\u{E0062}",
            "🏴\u{E007F}",
            "x\u{E0067}\u{E007F}",
        ] {
            assert!(has_hidden(name), "{name:?}");
        }
    }

    #[test]
    fn display_names_keep_an_unchanged_saved_name() {
        assert_eq!(display_name_error(" José 🌱 ", None), None);
        assert!(display_name_error("Jo\u{200B}se", None).is_some());
        assert_eq!(
            display_name_error(" Legacy\u{200B}Name ", Some("Legacy\u{200B}Name")),
            None
        );
        assert!(display_name_error("Other\u{200B}Name", Some("Legacy\u{200B}Name")).is_some());
    }
}
