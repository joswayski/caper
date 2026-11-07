//! Edits in the formatted composer, made as web's composer makes them
//! (`apps/web/src/chat/rich-text.ts`, ProseMirror). Text typed at the caret
//! takes the caret's style; web keeps whitespace at a style's edge outside
//! its delimiters, and the next key typed straight after it reopens that
//! style. Other edits only drop delimiters they orphaned. Shared by the native
//! composers (`apps/native/client`, and Android and Apple through
//! `apps/native/mobile`); every client runs
//! `shared/messages/composer-cases.json`, generated from web. Positions are
//! Unicode scalar offsets into the source text.
//!
//! The includer provides the formatting grammar as `super::parser` and the
//! link grammar as `super::links`.

use super::parser;

pub const BOLD: i32 = 1;
pub const ITALIC: i32 = 2;

/// An edit's result: the source text and selection, and `pending`, the style
/// web carries to the next key typed at this caret (`BOLD | ITALIC` bits), or
/// -1 when that key takes the style around the caret.
#[derive(Debug, PartialEq, Eq)]
pub struct Edited {
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub pending: i32,
}

/// Repairs `next`, the source after one native edit of `previous` with
/// selection `start..end`. `continuing` is the `pending` the previous call
/// returned when this edit types at the caret that call left, and -1
/// otherwise (the caret was placed, or the host does not track it).
pub fn edit(previous: &str, next: &str, start: usize, end: usize, continuing: i32) -> Edited {
    if let Some(typed) = typed(previous, next, start, end, continuing) {
        return typed;
    }
    let (text, start, end) = repair(previous, next, start, end);
    Edited {
        text,
        start,
        end,
        pending: -1,
    }
}

/// The visible characters, their styles and their source positions.
struct Visible {
    chars: Vec<char>,
    styles: Vec<i32>,
    sources: Vec<usize>,
}

fn visible(text: &str) -> Visible {
    let starts: Vec<usize> = text.char_indices().map(|(byte, _)| byte).collect();
    let mut out = Visible {
        chars: Vec::new(),
        styles: Vec::new(),
        sources: Vec::new(),
    };
    parser::visit_runs(text, |range, bold, italic| {
        let first = starts.partition_point(|byte| *byte < range.start);
        for (offset, c) in text[range].chars().enumerate() {
            out.chars.push(c);
            out.styles
                .push(if bold { BOLD } else { 0 } | if italic { ITALIC } else { 0 });
            out.sources.push(first + offset);
        }
    });
    out
}

/// White space as web's serializer trims it (JavaScript's `\s`).
fn space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// One delimiter `composer_source` wrote: its position in the source and the
/// characters of the style it bounds.
struct Delimiter {
    style: i32,
    from: usize,
    to: usize,
    position: usize,
}

/// Web's `composerEncoding`: the source for styled characters, each
/// character's source position, and the delimiters written. The longer style
/// is the outer pair; whitespace at a style's edge goes outside its markers.
fn encode(chars: &[char], styles: &[i32]) -> (String, Vec<usize>, Vec<Delimiter>) {
    enum Token {
        Char(usize),
        Marker(i32, usize, usize),
    }
    fn write(
        chars: &[char],
        styles: &[i32],
        range: std::ops::Range<usize>,
        removed: i32,
    ) -> Vec<Token> {
        let style = |index: usize| styles[index] & !removed;
        let mut out = Vec::new();
        let mut index = range.start;
        while index < range.end {
            if style(index) == 0 {
                out.push(Token::Char(index));
                index += 1;
                continue;
            }
            let end_of = |bit: i32| {
                (index..range.end)
                    .find(|i| style(*i) & bit == 0)
                    .unwrap_or(range.end)
            };
            let bit = if end_of(BOLD) >= end_of(ITALIC) {
                BOLD
            } else {
                ITALIC
            };
            let end = end_of(bit);
            let inner = write(chars, styles, index..end, removed | bit);
            let blank = |token: &Token| matches!(token, Token::Char(i) if space(chars[*i]));
            let mut start = 0;
            let mut limit = inner.len();
            while start < limit && blank(&inner[start]) {
                start += 1;
            }
            while limit > start && blank(&inner[limit - 1]) {
                limit -= 1;
            }
            let mut inner = inner.into_iter();
            out.extend(inner.by_ref().take(start));
            if start < limit {
                out.push(Token::Marker(bit, index, end));
                out.extend(inner.by_ref().take(limit - start));
                out.push(Token::Marker(bit, index, end));
            }
            out.extend(inner);
            index = end;
        }
        out
    }
    let mut source = String::new();
    let mut length = 0;
    let mut indices = vec![0; chars.len()];
    let mut delimiters = Vec::new();
    for token in write(chars, styles, 0..chars.len(), 0) {
        match token {
            Token::Char(index) => {
                indices[index] = length;
                source.push(chars[index]);
                length += 1;
            }
            Token::Marker(style, from, to) => {
                delimiters.push(Delimiter {
                    style,
                    from,
                    to,
                    position: length,
                });
                let marker = if style == BOLD { "**" } else { "_" };
                source.push_str(marker);
                length += marker.len();
            }
        }
    }
    (source, indices, delimiters)
}

/// Text typed at a caret, as web's `normalizeComposer` leaves it, or `None`
/// for any other edit.
fn typed(previous: &str, next: &str, start: usize, end: usize, continuing: i32) -> Option<Edited> {
    let before: Vec<char> = previous.chars().collect();
    let after: Vec<char> = next.chars().collect();
    let count = after
        .len()
        .checked_sub(before.len())
        .filter(|count| *count > 0)?;
    let at = start.checked_sub(count).filter(|_| start == end)?;
    if after[..at] != before[..at] || after[start..] != before[at..] {
        return None;
    }
    let old = visible(previous);
    let caret = old.sources.partition_point(|index| *index < at);
    // ProseMirror's marks at a caret: the text before it, or after it at the start.
    let style = if continuing >= 0 {
        continuing & (BOLD | ITALIC)
    } else if caret > 0 {
        old.styles[caret - 1]
    } else {
        old.styles.first().copied().unwrap_or(0)
    };
    let mut chars = old.chars;
    let mut styles = old.styles;
    chars.splice(caret..caret, after[at..start].iter().copied());
    styles.splice(caret..caret, std::iter::repeat_n(style, count));
    // A styled link's address keeps no partial styling.
    let mut offset = 0;
    for segment in super::links::segments(&chars.iter().collect::<String>()) {
        let limit = offset + segment.text.chars().count();
        if segment.href.is_some() {
            for bit in [BOLD, ITALIC] {
                let marked = styles[offset..limit]
                    .iter()
                    .filter(|s| **s & bit != 0)
                    .count();
                if marked > 0 && marked < limit - offset {
                    styles[offset..limit].iter_mut().for_each(|s| *s &= !bit);
                }
            }
        }
        offset = limit;
    }
    // Styles whose delimiters would show are dropped, keeping the text.
    let (mut source, mut indices, mut delimiters) = encode(&chars, &styles);
    loop {
        let shown = visible(&source).sources;
        let invalid: Vec<&Delimiter> = delimiters
            .iter()
            .filter(|marker| shown.binary_search(&marker.position).is_ok())
            .collect();
        if invalid.is_empty() {
            break;
        }
        for marker in invalid {
            styles[marker.from..marker.to]
                .iter_mut()
                .for_each(|s| *s &= !marker.style);
        }
        (source, indices, delimiters) = encode(&chars, &styles);
    }
    let parsed = visible(&source);
    let (head, pending) = if parsed.chars == chars && parsed.styles == styles {
        (caret + count, -1)
    } else {
        // Web re-parses its source; the caret keeps its place in that source,
        // and the typed style carries on when the caret's own style differs.
        let position = indices[caret + count - 1] + 1;
        let head = parsed.sources.partition_point(|index| *index < position);
        let around = head
            .checked_sub(1)
            .or((!parsed.styles.is_empty()).then_some(0))
            .map_or(0, |index| parsed.styles[index]);
        (head, if around == style { -1 } else { style })
    };
    // The composer shows its document's own encoding.
    let (text, _, _) = encode(&parsed.chars, &parsed.styles);
    let shown = visible(&text).sources;
    let source_caret = shown
        .get(head)
        .copied()
        .unwrap_or_else(|| shown.last().map_or(0, |index| index + 1));
    Some(Edited {
        text,
        start: source_caret,
        end: source_caret,
        pending,
    })
}

/// Strips only old hidden delimiters that an edit has made visible or orphaned.
fn repair(previous: &str, next: &str, start: usize, end: usize) -> (String, usize, usize) {
    let hidden = |text: &str| {
        let shown = visible(text).sources;
        let mut flags = vec![true; text.chars().count()];
        for index in shown {
            flags[index] = false;
        }
        flags
    };
    let before: Vec<char> = previous.chars().collect();
    let after: Vec<char> = next.chars().collect();
    let old_hidden = hidden(previous);
    if previous == next || !old_hidden.contains(&true) {
        return (next.into(), start, end);
    }
    let prefix = before
        .iter()
        .zip(&after)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = before[prefix..]
        .iter()
        .rev()
        .zip(after[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    // Map old markers once. Re-diffing after a removal could mistake a newly
    // typed '*' or '_' for another marker from the original prefix/suffix.
    let mut inherited = vec![false; after.len()];
    for (index, old_hidden) in old_hidden.into_iter().enumerate() {
        if !old_hidden {
            continue;
        }
        let mapped = if index < prefix {
            index
        } else if index >= before.len() - suffix {
            after.len() - (before.len() - index)
        } else {
            continue;
        };
        inherited[mapped] = true;
    }
    let (mut text, mut start, mut end) = (next.to_owned(), start, end);
    loop {
        let removed: Vec<bool> = inherited
            .iter()
            .zip(hidden(&text))
            .map(|(old, hidden)| *old && !hidden)
            .collect();
        if !removed.contains(&true) {
            return (text, start, end);
        }
        let offset =
            |value: usize| value - removed[..value].iter().filter(|removed| **removed).count();
        (start, end) = (offset(start), offset(end));
        text = text
            .chars()
            .zip(&removed)
            .filter_map(|(c, removed)| (!*removed).then_some(c))
            .collect();
        // Removing one pair may orphan another; keep its original provenance.
        inherited = inherited
            .into_iter()
            .zip(removed)
            .filter_map(|(old, removed)| (!removed).then_some(old))
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacements_strip_old_orphans_without_consuming_new_literals() {
        for (before, next, start, end, text, expected_start, expected_end) in [
            ("**bold** a", "**x", 3, 3, "x", 1, 1),
            ("**bold** a", "**", 2, 2, "", 0, 0),
            ("", "*", 1, 1, "*", 1, 1),
            // Replacing the visible draft leaves its old hidden opening pair.
            ("**bold** a", "***", 3, 3, "*", 1, 1),
            // The corresponding case with an old closing pair instead.
            ("a **bold**", "***", 1, 1, "*", 1, 1),
            ("_italic_ a", "__", 2, 2, "_", 1, 1),
            ("**_bold_** a", "**_*", 4, 4, "*", 1, 1),
            ("🙂 **bold** a", "🙂 ***", 5, 1, "🙂 *", 3, 1),
            // Removing invalid bold exposes both old italic markers next to 'a'.
            ("a**_b_!**", "a**_b_ **", 7, 4, "ab ", 3, 1),
            // Still-valid formatting and an unchanged source are not orphans.
            ("**old** a", "**new**", 5, 5, "**new**", 5, 5),
            ("**old**", "**old**", 7, 2, "**old**", 7, 2),
        ] {
            assert_eq!(
                edit(before, next, start, end, -1),
                Edited {
                    text: text.into(),
                    start: expected_start,
                    end: expected_end,
                    pending: -1,
                },
                "{before:?} -> {next:?} at {start}..{end}"
            );
        }
    }
}
