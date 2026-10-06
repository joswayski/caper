//! `@mention` contract v1, shared with the API and every client: token grammar,
//! composer suggestions and insertion, and which message tokens to highlight.
//! Mentions do not notify anyone yet.

use crate::model::Mention;
use std::ops::Range;

/// Longer runs after `@` are plain text.
const MAX_NAME: usize = 32;
/// Popup rows, as for `:` emoji suggestions.
const MAX_SUGGESTIONS: usize = 6;
/// The composer's message limit in Unicode scalars.
const MAX_MESSAGE: usize = 4_000;

fn name_character(value: char) -> bool {
    value.is_ascii_alphanumeric() || value == '_'
}

/// Same start rule as `:` emoji: text start, whitespace, `(`, `[` or `{`.
fn starts_token(previous: Option<char>) -> bool {
    previous.is_none_or(|value| value.is_whitespace() || matches!(value, '(' | '[' | '{'))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub query: String,
}

/// The `@` token ending at `caret`, if suggestions should open there. Like
/// egui cursors, positions count Unicode scalars, not UTF-8 bytes. The `:` emoji
/// token can never be active at the same caret: its run cannot reach an `@`.
pub fn token(text: &str, caret: usize) -> Option<Token> {
    let chars: Vec<_> = text.chars().collect();
    if caret > chars.len()
        || chars
            .get(caret)
            .is_some_and(|c| *c == '@' || name_character(*c))
    {
        return None;
    }
    let mut start = caret;
    while start > 0 && name_character(chars[start - 1]) {
        start -= 1;
    }
    let at = start.checked_sub(1)?;
    if caret - start > MAX_NAME
        || chars[at] != '@'
        || !starts_token(at.checked_sub(1).map(|index| chars[index]))
    {
        return None;
    }
    Some(Token {
        start: at,
        end: caret,
        query: chars[start..caret].iter().collect(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub username: String,
    pub display_name: String,
    pub avatar_id: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Candidate {
    Person(Person),
    Everyone,
    Here,
}

impl Candidate {
    /// The name inserted after `@`.
    pub fn name(&self) -> &str {
        match self {
            Self::Person(person) => &person.username,
            Self::Everyone => "everyone",
            Self::Here => "here",
        }
    }
}

/// Up to six rows: matching people by rank then username, then the matching
/// specials (`everyone`, `here`), which always keep their slots. Callers pass
/// people without the current user, and `specials` only in space channels.
pub fn suggestions(people: &[Person], specials: bool, query: &str) -> Vec<Candidate> {
    let query = query.to_ascii_lowercase();
    let specials: Vec<_> = [Candidate::Everyone, Candidate::Here]
        .into_iter()
        .filter(|special| specials && special.name().starts_with(&query))
        .collect();
    let mut ranked: Vec<_> = people
        .iter()
        .filter_map(|person| {
            let username = person.username.to_lowercase();
            let display_name = person.display_name.to_lowercase();
            // The query has no spaces, so the first word covers "starts with".
            let rank = if username == query {
                0
            } else if username.starts_with(&query) {
                1
            } else if display_name.split(' ').any(|word| word.starts_with(&query)) {
                2
            } else if username.contains(&query) {
                3
            } else {
                return None;
            };
            Some((rank, username, person))
        })
        .collect();
    ranked.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
    ranked
        .into_iter()
        .take(MAX_SUGGESTIONS - specials.len())
        .map(|(_, _, person)| Candidate::Person(person.clone()))
        .chain(specials)
        .collect()
}

/// Replaces the token with `@name ` and returns the draft and the caret after
/// the space, or `None` past the message limit (as emoji insertion does).
pub fn insert(text: &str, token: &Token, name: &str) -> Option<(String, usize)> {
    let mention = format!("@{name} ");
    let value: String = text
        .chars()
        .take(token.start)
        .chain(mention.chars())
        .chain(text.chars().skip(token.end))
        .collect();
    (value.chars().count() <= MAX_MESSAGE).then(|| (value, token.start + mention.chars().count()))
}

/// Every mention token in sent text, as UTF-8 byte ranges with lowercase names.
fn tokens(text: &str) -> Vec<(Range<usize>, String)> {
    let mut found = Vec::new();
    let mut previous = None;
    let mut chars = text.char_indices().peekable();
    while let Some((index, character)) = chars.next() {
        let starts = character == '@' && starts_token(previous);
        previous = Some(character);
        if !starts {
            continue;
        }
        let mut end = index + 1;
        while let Some(&(next_index, next)) = chars.peek() {
            if !name_character(next) {
                break;
            }
            end = next_index + next.len_utf8();
            previous = Some(next);
            chars.next();
        }
        // Name characters are ASCII, so bytes equal characters.
        if (1..=MAX_NAME).contains(&(end - index - 1)) {
            found.push((index..end, text[index + 1..end].to_ascii_lowercase()));
        }
    }
    found
}

/// Whether a token's lowercase name is backed by a server entry. Unresolved
/// names, and `everyone`/`here` without their entry (as in DMs), stay plain.
fn resolved(name: &str, mentions: &[Mention]) -> bool {
    mentions.iter().any(|entry| match entry.kind.as_str() {
        "everyone" | "here" => entry.kind == name,
        "user" => entry
            .username
            .as_deref()
            .is_some_and(|username| username.eq_ignore_ascii_case(name)),
        _ => false,
    })
}

/// Byte ranges of the tokens in `text` to draw as mention pills.
pub fn highlights(text: &str, mentions: &[Mention]) -> Vec<Range<usize>> {
    if mentions.is_empty() {
        return Vec::new();
    }
    tokens(text)
        .into_iter()
        .filter(|(_, name)| resolved(name, mentions))
        .map(|(range, _)| range)
        .collect()
}

/// A message mentions `me` when a `user` entry has my account id, or when it
/// has `everyone`/`here` and someone else wrote it.
pub fn mentions_me(mentions: &[Mention], author_id: &str, me: Option<&str>) -> bool {
    let Some(me) = me else {
        return false;
    };
    mentions.iter().any(|entry| match entry.kind.as_str() {
        "user" => entry.id.as_deref() == Some(me),
        "everyone" | "here" => author_id != me,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(username: &str, display_name: &str) -> Person {
        Person {
            username: username.into(),
            display_name: display_name.into(),
            avatar_id: None,
        }
    }

    fn names(candidates: &[Candidate]) -> Vec<&str> {
        candidates.iter().map(Candidate::name).collect()
    }

    fn entry(kind: &str, id: Option<&str>, username: Option<&str>) -> Mention {
        Mention {
            kind: kind.into(),
            id: id.map(Into::into),
            username: username.map(Into::into),
        }
    }

    fn user(id: &str, username: &str) -> Mention {
        entry("user", Some(id), Some(username))
    }

    #[test]
    fn composer_token_follows_the_start_rule_and_caret_guards() {
        for value in [
            "@",
            "@al",
            "hi @al",
            "(@al",
            "[@al",
            "{@al",
            "line\n@al",
            "a\t@al",
        ] {
            assert!(token(value, value.chars().count()).is_some(), "{value}");
        }
        for value in [
            "bob@alice",
            "x/@al",
            "@@al",
            "word@",
            ":@al",
            "@al-",
            "@al.",
        ] {
            assert!(token(value, value.chars().count()).is_none(), "{value}");
        }
        assert_eq!(
            token("hi @Al", 6),
            Some(Token {
                start: 3,
                end: 6,
                query: "Al".into()
            })
        );
        assert_eq!(token("@", 1).unwrap().query, "");
        // The caret must end the run: not before a name character or `@`.
        assert!(token("@alice", 3).is_none());
        assert!(token("@al@", 3).is_none());
        assert!(token("@al ", 3).is_some());
        assert!(token("@al!", 3).is_some());
        assert!(token("@al", 4).is_none(), "caret beyond the text");
        let longest = format!("@{}", "a".repeat(32));
        assert!(token(&longest, longest.chars().count()).is_some());
        let longer = format!("@{}", "a".repeat(33));
        assert!(token(&longer, longer.chars().count()).is_none());
        // Positions are Unicode scalars, as egui cursors are.
        let text = "👩‍💻 (@ma";
        assert_eq!(
            token(text, text.chars().count()),
            Some(Token {
                start: "👩‍💻 (".chars().count(),
                end: text.chars().count(),
                query: "ma".into()
            })
        );
        assert!(token("é@ma", 4).is_none());
        // `:` emoji and `@` mention tokens are never active together.
        for value in [":smile@al", "@al:smi", ":+1 @al", "@al :tom"] {
            let caret = value.chars().count();
            assert!(
                !(token(value, caret).is_some() && crate::emoji::token(value, caret).is_some()),
                "{value}"
            );
        }
    }

    #[test]
    fn suggestions_rank_members_then_specials_and_cap_at_six() {
        let people = [
            person("alexander", "Zed Q"),
            person("alex", "Alex Rivera"),
            person("bob", "Robert Alexson"),
            person("malex", "Mal"),
            person("carol", "Carol"),
        ];
        assert_eq!(
            names(&suggestions(&people, false, "ALEX")),
            ["alex", "alexander", "bob", "malex"],
            "exact, prefix, display-name word, contains"
        );
        assert_eq!(names(&suggestions(&people, false, "riv")), ["alex"]);
        assert_eq!(names(&suggestions(&people, false, "q")), ["alexander"]);
        assert!(suggestions(&people, true, "zzz").is_empty());
        assert_eq!(
            names(&suggestions(&people, true, "")),
            ["alex", "alexander", "bob", "carol", "everyone", "here"],
            "an empty query lists members alphabetically and keeps the specials"
        );
        assert_eq!(names(&suggestions(&people, true, "ev")), ["everyone"]);
        assert_eq!(
            names(&suggestions(&people, true, "e")),
            ["alex", "alexander", "malex", "everyone"],
            "usernames containing the query still rank above specials"
        );
        assert_eq!(
            names(&suggestions(&people, true, "h")),
            ["here"],
            "specials match by prefix only"
        );
        assert!(
            suggestions(&people, false, "every").is_empty(),
            "no specials in DMs"
        );
        let many: Vec<_> = (0..10)
            .map(|index| person(&format!("user{index}"), "Member"))
            .collect();
        assert_eq!(
            names(&suggestions(&many, false, "")),
            ["user0", "user1", "user2", "user3", "user4", "user5"]
        );
        assert_eq!(
            names(&suggestions(&many, true, "")),
            ["user0", "user1", "user2", "user3", "everyone", "here"]
        );
        assert!(suggestions(&[], false, "").is_empty());
        assert_eq!(names(&suggestions(&[], true, "")), ["everyone", "here"]);
    }

    #[test]
    fn insertion_adds_a_trailing_space_and_keeps_unicode_and_the_limit() {
        let text = "👩‍💻 hi @ma suffix 🚀";
        let active = token(text, "👩‍💻 hi @ma".chars().count()).unwrap();
        assert_eq!(
            insert(text, &active, "maya"),
            Some((
                "👩‍💻 hi @maya  suffix 🚀".into(),
                "👩‍💻 hi @maya ".chars().count()
            ))
        );
        let active = token("@", 1).unwrap();
        assert_eq!(
            insert("@", &active, "everyone"),
            Some(("@everyone ".into(), 10))
        );
        let text = format!("{} @x", "😀".repeat(3993));
        let active = token(&text, text.chars().count()).unwrap();
        assert_eq!(
            insert(&text, &active, "abcd").map(|(value, caret)| (value.chars().count(), caret)),
            Some((4_000, 4_000))
        );
        assert!(insert(&text, &active, "abcde").is_none());
    }

    fn pills<'a>(text: &'a str, mentions: &[Mention]) -> Vec<&'a str> {
        highlights(text, mentions)
            .into_iter()
            .map(|range| &text[range])
            .collect()
    }

    #[test]
    fn highlights_only_tokens_the_server_resolved() {
        let mentions = [
            user("user00000001", "alice"),
            entry("everyone", None, None),
            entry("role", Some("r"), Some("bob")),
        ];
        let text = "hey @Alice's (@alice) @everyone @here @bob bob@alice.com x/@alice @@alice @al";
        assert_eq!(pills(text, &mentions), ["@Alice", "@alice", "@everyone"]);
        assert!(pills("@alice", &[]).is_empty());
        assert!(
            pills("@everyone @here", &[user("user00000001", "alice")]).is_empty(),
            "specials need their own entries (DMs never have them)"
        );
        assert_eq!(pills("@here now", &[entry("here", None, None)]), ["@here"]);
        let long = format!("@{} @alice", "a".repeat(33));
        assert_eq!(pills(&long, &mentions), ["@alice"]);
        assert_eq!(pills("é @ALICE ✨", &mentions), ["@ALICE"]);
    }

    #[test]
    fn mentions_me_by_account_id_or_by_someone_elses_everyone_or_here() {
        let me = Some("user00000001");
        assert!(mentions_me(&[user("user00000001", "alice")], "author", me));
        assert!(mentions_me(
            &[user("user00000001", "alice")],
            "user00000001",
            me
        ));
        assert!(!mentions_me(&[user("user00000002", "alice")], "author", me));
        for special in ["everyone", "here"] {
            let mentions = [entry(special, None, None)];
            assert!(mentions_me(&mentions, "author", me), "{special}");
            assert!(!mentions_me(&mentions, "user00000001", me), "{special}");
        }
        assert!(!mentions_me(
            &[entry("role", Some("user00000001"), None)],
            "author",
            me
        ));
        assert!(!mentions_me(
            &[entry("everyone", None, None)],
            "author",
            None
        ));
        assert!(!mentions_me(&[], "author", me));
    }
}
