//! `@mention` grammar shared with every client: an `@` at the start of the text,
//! after whitespace or after `(`, `[` or `{`, followed by 1–32 ASCII word
//! characters. Names compare case-insensitively.

/// Distinct user names resolved per message; later names stay plain text.
pub(crate) const MAX_USER_MENTIONS: usize = 20;
const MAX_NAME: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Mention {
    User(String),
    Everyone,
    Here,
}

/// Mentions in first-appearance order without duplicates. `everyone`/`here`
/// are only special where `specials` is true (space channels).
pub(crate) fn parse(text: &str, specials: bool) -> Vec<Mention> {
    let mut found = Vec::new();
    let mut users = 0;
    let mut previous: Option<char> = None;
    let mut chars = text.char_indices().peekable();
    while let Some((index, character)) = chars.next() {
        let starts = character == '@'
            && previous.is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '[' | '{'));
        previous = Some(character);
        if !starts {
            continue;
        }
        let name_start = index + 1;
        let mut name_end = name_start;
        while let Some(&(next_index, next)) = chars.peek() {
            if !is_name_char(next) {
                break;
            }
            name_end = next_index + next.len_utf8();
            previous = Some(next);
            chars.next();
        }
        let name = text[name_start..name_end].to_ascii_lowercase();
        let mention = match name.as_str() {
            _ if name.len() > MAX_NAME => continue,
            "everyone" if specials => Mention::Everyone,
            "here" if specials => Mention::Here,
            "everyone" | "here" => continue,
            _ if name.len() >= 3 => Mention::User(name),
            _ => continue,
        };
        if found.contains(&mention) {
            continue;
        }
        if matches!(mention, Mention::User(_)) {
            if users == MAX_USER_MENTIONS {
                continue;
            }
            users += 1;
        }
        found.push(mention);
    }
    found
}

fn is_name_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(name: &str) -> Mention {
        Mention::User(name.into())
    }

    #[test]
    fn start_rule_matches_the_emoji_autocomplete_boundary() {
        assert_eq!(parse("@alice", true), [user("alice")]);
        assert_eq!(parse("hi @alice!", true), [user("alice")]);
        assert_eq!(
            parse("(@alice) [@bob] {@carol}", true),
            [user("alice"), user("bob"), user("carol")]
        );
        assert_eq!(
            parse("line\n@alice\t@bob", true),
            [user("alice"), user("bob")]
        );
        assert_eq!(parse("@Alice's", true), [user("alice")]);
        for plain in [
            "bob@alice.com",
            "x/@alice",
            "@@alice",
            "a,@alice",
            "@",
            "@ alice",
            "@al",
        ] {
            assert!(parse(plain, true).is_empty(), "{plain:?}");
        }
    }

    #[test]
    fn names_are_whole_runs_of_at_most_32_word_characters() {
        let longest = "a".repeat(32);
        assert_eq!(parse(&format!("@{longest}"), true), [user(&longest)]);
        assert!(parse(&format!("@{longest}b"), true).is_empty());
        assert_eq!(parse("@user_1.", true), [user("user_1")]);
        assert_eq!(parse("@alicé", true), [user("alic")]);
        assert_eq!(parse("🙂 @bob", true), [user("bob")]);
    }

    #[test]
    fn specials_apply_only_in_space_channels_and_dedupe_in_order() {
        assert_eq!(
            parse("@here @Everyone @bob @HERE @BOB", true),
            [Mention::Here, Mention::Everyone, user("bob")]
        );
        assert_eq!(parse("@everyone @here @bob", false), [user("bob")]);
    }

    #[test]
    fn user_names_are_capped_but_specials_are_not() {
        let names: String = (0..25).map(|i| format!("@user{i:02} ")).collect();
        let parsed = parse(&format!("{names}@everyone"), true);
        assert_eq!(parsed.len(), MAX_USER_MENTIONS + 1);
        assert_eq!(parsed[0], user("user00"));
        assert_eq!(parsed[MAX_USER_MENTIONS - 1], user("user19"));
        assert_eq!(parsed[MAX_USER_MENTIONS], Mention::Everyone);
    }
}
