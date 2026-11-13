//! Emoji autocomplete as web makes it (`apps/web/src/chat/emoji-autocomplete.ts`,
//! mapped through the visible text as `ComposerSuggestions.tsx` does): the
//! `:` token at the caret and the catalog entries offered for its query.
//! Shared by the native composers (`apps/native/client`, and Android and Apple
//! through `apps/native/mobile`); every client runs
//! `shared/messages/emoji-cases.json`, generated from web. Positions are
//! Unicode scalar offsets into the source text. The catalog is the host's:
//! this module only searches the entries it is given.
//!
//! The includer provides the formatting grammar as `super::parser`.

use super::parser;

/// The `:` token to replace, as a source range, and its visible query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub query: String,
}

fn query_character(value: char) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, '_' | '+' | '-')
}

/// White space as web's `\s` matches it.
fn space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// The open `:` token with the caret at source offset `caret`, read in the
/// visible text, so delimiters around or inside the query don't hide it.
pub fn token(text: &str, caret: usize) -> Option<Token> {
    let starts: Vec<usize> = text.char_indices().map(|(byte, _)| byte).collect();
    if caret > starts.len() {
        return None;
    }
    let mut chars = Vec::new();
    let mut sources = Vec::new();
    parser::visit_runs(text, |range, _| {
        let first = starts.partition_point(|byte| *byte < range.start);
        for (offset, c) in text[range].chars().enumerate() {
            chars.push(c);
            sources.push(first + offset);
        }
    });
    let caret = sources.partition_point(|index| *index < caret);
    if chars
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
        || (colon > 0 && !space(chars[colon - 1]) && !"([{".contains(chars[colon - 1]))
    {
        return None;
    }
    // One character is an emoticon (":D", ":P", ":3"), not a search; Enter sends it as typed.
    if caret - start == 1 {
        return None;
    }
    Some(Token {
        start: sources[colon],
        end: sources[caret - 1] + 1,
        query: chars[start..caret].iter().collect(),
    })
}

/// One catalog entry as the search reads it.
pub struct Choice<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub keywords: &'a str,
    pub selectable: bool,
}

fn normalized(value: &str) -> String {
    value.to_lowercase().replace(['_', '-'], " ")
}

/// A catalog's search keys, normalized once rather than on every keystroke.
pub struct Search {
    ids: Vec<String>,
    names: Vec<String>,
    keywords: Vec<String>,
    /// The name's length as web compares it (UTF-16 units).
    lengths: Vec<usize>,
    selectable: Vec<bool>,
}

impl Search {
    pub fn new<'a>(choices: impl IntoIterator<Item = Choice<'a>>) -> Self {
        let mut search = Search {
            ids: Vec::new(),
            names: Vec::new(),
            keywords: Vec::new(),
            lengths: Vec::new(),
            selectable: Vec::new(),
        };
        for choice in choices {
            search.ids.push(choice.id.to_owned());
            search.names.push(normalized(choice.name));
            search.keywords.push(normalized(choice.keywords));
            search.lengths.push(choice.name.encode_utf16().count());
            search.selectable.push(choice.selectable);
        }
        search
    }

    /// Up to six catalog positions web offers for `query`, best first: the
    /// defaults while it is empty, else exact names, name prefixes, keyword
    /// prefixes, then substrings, shorter names first within each.
    pub fn suggestions(&self, query: &str) -> Vec<usize> {
        let offered = |index: &usize| self.selectable[*index];
        if query.is_empty() {
            return ["1f44d", "1f600", "2764", "1f389", "1f680", "1f440"]
                .iter()
                .filter_map(|id| {
                    (0..self.ids.len())
                        .filter(offered)
                        .find(|i| self.ids[*i] == *id)
                })
                .collect();
        }
        let needle = normalized(query);
        let spaced = format!(" {needle}");
        let mut matches: Vec<(u8, usize)> = (0..self.ids.len())
            .filter(offered)
            .filter_map(|index| {
                let (name, keywords) = (&self.names[index], &self.keywords[index]);
                let rank = if *name == needle {
                    0
                } else if name.starts_with(&needle) {
                    1
                } else if keywords.starts_with(&needle) || keywords.contains(&spaced) {
                    2
                } else if name.contains(&needle) || keywords.contains(&needle) {
                    3
                } else {
                    return None;
                };
                Some((rank, index))
            })
            .collect();
        // Within a rank the shorter name is the closer match: ":fi" offers fire before film-frames.
        matches.sort_by_key(|(rank, index)| (*rank, self.lengths[*index]));
        matches
            .into_iter()
            .take(6)
            .map(|(_, index)| index)
            .collect()
    }
}
