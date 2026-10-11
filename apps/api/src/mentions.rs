//! `@mention` grammar shared with every client: an `@` at the start of the text,
//! after whitespace or after `(`, `[` or `{`, followed by 1–32 ASCII word
//! characters. Names compare case-insensitively.

/// Distinct user names resolved per message; later names stay plain text.
pub(crate) const MAX_USER_MENTIONS: usize = 20;
const MAX_NAME: usize = 32;

/// Reserved names that mean the Caper team. They resolve to the team account
/// (whatever its own username), which sends the message to `@feedback`.
pub(crate) const TEAM_ALIASES: &[&str] = &[
    "feedback",
    "caper",
    "capers",
    "caperchat",
    "caper_feedback",
    "caper_support",
    "caper_team",
    "support",
    "team",
    "staff",
    "admin",
    "admins",
    "administrator",
    "administrators",
    "mod",
    "mods",
    "moderator",
    "moderators",
];

/// Whether `name` (any case) means the Caper team.
pub(crate) fn is_team_alias(name: &str) -> bool {
    TEAM_ALIASES
        .iter()
        .any(|alias| alias.eq_ignore_ascii_case(name))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Mention {
    User(String),
    Everyone,
    Here,
}

/// Every `@name` token as `(byte range including the @, lowercase name)`,
/// in order. Names may be empty, too short or too long; `parse` filters them.
fn tokens(text: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let mut found = Vec::new();
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
        found.push((
            index..name_end,
            text[name_start..name_end].to_ascii_lowercase(),
        ));
    }
    found
}

/// Whether the text mentions the team and says nothing else: no other
/// letters or digits, just the alias, spacing and punctuation.
pub(crate) fn only_team_mentions(text: &str) -> bool {
    let mut team = false;
    let mut rest = 0;
    let mut said = false;
    for (range, name) in tokens(text) {
        if name.len() <= MAX_NAME && is_team_alias(&name) {
            team = true;
            said |= text[rest..range.start].chars().any(char::is_alphanumeric);
            rest = range.end;
        }
    }
    team && !said && !text[rest..].chars().any(char::is_alphanumeric)
}

/// Mentions in first-appearance order without duplicates. `everyone`/`here`
/// are only special where `specials` is true (space channels).
pub(crate) fn parse(text: &str, specials: bool) -> Vec<Mention> {
    let mut found = Vec::new();
    let mut users = 0;
    for (_, name) in tokens(text) {
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
    fn team_aliases_are_reserved_lowercase_mentionable_names() {
        let reserved = crate::accounts::ReservedUsernames::default();
        for alias in TEAM_ALIASES {
            assert!(reserved.contains(alias), "{alias} must be reserved");
            assert_eq!(parse(&format!("@{alias}"), true), [user(alias)]);
        }
        assert!(is_team_alias("Feedback"));
        assert!(is_team_alias("ADMINS"));
        assert!(!is_team_alias("everyone"));
        assert!(!is_team_alias("jose"));
    }

    #[test]
    fn only_team_mentions_ignores_spacing_and_punctuation() {
        for bare in [
            "@feedback",
            " @Feedback! ",
            "@admins @mods",
            "(@caper) ^^",
            "@team 👆",
        ] {
            assert!(only_team_mentions(bare), "{bare:?}");
        }
        for said in [
            "@feedback uploads fail",
            "this @feedback",
            "@feedback @alice",
            "@alice",
            "feedback",
            "x@feedback",
            "",
        ] {
            assert!(!only_team_mentions(said), "{said:?}");
        }
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
