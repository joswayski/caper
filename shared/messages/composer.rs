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

pub use super::parser::{BOLD, ITALIC, QUOTE, SPOILER, STRIKETHROUGH, UNDERLINE};

/// Every style bit; a carried style is a set of these.
pub const STYLES: i32 = BOLD | ITALIC | UNDERLINE | STRIKETHROUGH | SPOILER | QUOTE;
/// Inline styles in the order web's serializer prefers one as the outer pair
/// when two cover the same characters.
const INLINE: [i32; 5] = [BOLD, UNDERLINE, ITALIC, STRIKETHROUGH, SPOILER];

/// An edit's result: the source text and selection, and `pending`, the style
/// web carries to the next key typed at this caret (style bits), or
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
    parser::visit_runs(text, |range, style| {
        let first = starts.partition_point(|byte| *byte < range.start);
        for (offset, c) in text[range].chars().enumerate() {
            out.chars.push(c);
            out.styles.push(style);
            out.sources.push(first + offset);
        }
    });
    out
}

use super::parser::whitespace as space;

/// One delimiter `composer_source` wrote: its position in the source and the
/// characters of the style it bounds.
struct Delimiter {
    style: i32,
    from: usize,
    to: usize,
    position: usize,
}

/// The source delimiter of a style bit.
fn marker(style: i32) -> String {
    if style == QUOTE {
        return "> ".into();
    }
    let (_, m) = parser::MARKERS
        .into_iter()
        .find(|(bit, _)| *bit == style)
        .unwrap();
    m.iter().collect()
}

/// The lines of `chars` as `(start, end, quoted)`, without their newlines. A
/// line is quoted when any of its characters is.
fn lines(chars: &[char], styles: &[i32]) -> Vec<(usize, usize, bool)> {
    let mut out = Vec::new();
    let mut start = 0;
    loop {
        let end = (start..chars.len())
            .find(|i| chars[*i] == '\n')
            .unwrap_or(chars.len());
        out.push((
            start,
            end,
            styles[start..end].iter().any(|s| s & QUOTE != 0),
        ));
        if end == chars.len() {
            return out;
        }
        start = end + 1;
    }
}

/// Web's `composerEncoding`: the source for styled characters, each
/// character's source position, and the delimiters written. Quoted lines are
/// encoded alone after `> `; consecutive unquoted lines together. Among
/// inline styles the longer is the outer pair; whitespace at a style's edge
/// goes outside its markers.
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
            let (bit, end) = INLINE
                .into_iter()
                .filter(|bit| style(index) & bit != 0)
                .map(|bit| (bit, end_of(bit)))
                .reduce(|best, next| if next.1 > best.1 { next } else { best })
                .unwrap();
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
    let lines = lines(chars, styles);
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let (start, end, quoted) = lines[index];
        if quoted {
            tokens.push(Token::Marker(QUOTE, start, end));
            tokens.extend(write(chars, styles, start..end, QUOTE));
            index += 1;
        } else {
            let mut last = index;
            while last + 1 < lines.len() && !lines[last + 1].2 {
                last += 1;
            }
            tokens.extend(write(chars, styles, start..lines[last].1, QUOTE));
            index = last + 1;
        }
        if index < lines.len() {
            tokens.push(Token::Char(lines[index - 1].1));
        }
    }
    let mut source = String::new();
    let mut length = 0;
    let mut indices = vec![0; chars.len()];
    let mut delimiters = Vec::new();
    for token in tokens {
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
                let marker = marker(style);
                source.push_str(&marker);
                length += marker.chars().count();
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
        continuing & STYLES
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
            for bit in [BOLD, ITALIC, UNDERLINE, STRIKETHROUGH, SPOILER, QUOTE] {
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
        // A quote belongs to lines: a newline typed in one continues it,
        // and a second leaves it.
        let style = if quoted_at(&parsed.chars, &parsed.styles, head) {
            style | QUOTE
        } else {
            style & !QUOTE
        };
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

/// Whether text typed at visible `head` is quoted: its line is, or the caret
/// starts an empty line after a quoted one.
fn quoted_at(chars: &[char], styles: &[i32], head: usize) -> bool {
    let line_start = |end: usize| {
        chars[..end]
            .iter()
            .rposition(|c| *c == '\n')
            .map_or(0, |index| index + 1)
    };
    let mut start = line_start(head);
    let mut end = (head..chars.len())
        .find(|i| chars[*i] == '\n')
        .unwrap_or(chars.len());
    if start == end {
        let Some(previous) = head.checked_sub(1) else {
            return false;
        };
        end = previous;
        start = line_start(end);
    }
    styles[start..end].iter().any(|s| s & QUOTE != 0)
}

/// Turns style `style` on or off for the visible characters of source
/// selection `start..end` (Unicode scalars), as web's toolbar does: inline
/// styles as ProseMirror's `toggleMark`, removing a style the selection has
/// anywhere and otherwise adding it without the selection's outer white
/// space; `QUOTE` for each non-blank line the selection touches. Returns the
/// source and its selection, or `None` when the change would split a link,
/// pass the 4000-character limit, or not survive its own encoding.
pub fn toggle(text: &str, start: usize, end: usize, style: i32) -> Option<(String, usize, usize)> {
    let Visible {
        chars,
        mut styles,
        sources,
    } = visible(text);
    let from = sources.partition_point(|index| *index < start.min(end));
    let to = sources.partition_point(|index| *index < start.max(end));
    if from == to {
        return None;
    }
    let shown: String = chars.iter().collect();
    let mut offset = 0;
    for segment in super::links::segments(&shown) {
        let limit = offset + segment.text.chars().count();
        if segment.href.is_some()
            && ((from > offset && from < limit) || (to > offset && to < limit))
        {
            return None;
        }
        offset = limit;
    }
    if style == QUOTE {
        let lines = lines(&chars, &styles);
        let touched: Vec<usize> = (0..lines.len())
            .filter(|line| {
                let (start, end, _) = lines[*line];
                start <= to - 1 && end >= from && !chars[start..end].iter().all(|c| space(*c))
            })
            .collect();
        if touched.is_empty() {
            return None;
        }
        let add = !touched.iter().any(|line| lines[*line].2);
        let mut quoted: Vec<bool> = lines.iter().map(|line| line.2).collect();
        for line in touched {
            let (start, end, _) = lines[line];
            quoted[line] = add;
            for s in &mut styles[start..end] {
                *s = if add { *s | QUOTE } else { *s & !QUOTE };
            }
        }
        // Newlines inside a quote are quoted; those at its edges are plain.
        for line in 1..lines.len() {
            let newline = lines[line].0 - 1;
            if quoted[line - 1] || quoted[line] {
                styles[newline] = if quoted[line - 1] && quoted[line] {
                    QUOTE
                } else {
                    0
                };
            } else if styles[newline] & QUOTE != 0 {
                styles[newline] = 0;
            }
        }
    } else {
        let range = from..to;
        if styles[range.clone()].iter().any(|s| s & style != 0) {
            styles[range].iter_mut().for_each(|s| *s &= !style);
        } else {
            // ProseMirror leaves white space at the selection's ends unmarked,
            // within the text node (one style) at each end.
            let leading = (from..to)
                .take_while(|i| styles[*i] == styles[from] && space(chars[*i]))
                .count();
            let trailing = (from..to)
                .rev()
                .take_while(|i| styles[*i] == styles[to - 1] && space(chars[*i]))
                .count();
            let (mut a, mut b) = (from, to);
            if from + leading < to {
                a += leading;
                b -= trailing;
            }
            if a < b {
                styles[a..b].iter_mut().for_each(|s| *s |= style);
            }
        }
    }
    let (source, _, _) = encode(&chars, &styles);
    let parsed = visible(&source);
    if source.chars().count() > 4000 || parsed.chars != chars || parsed.styles != styles {
        return None;
    }
    Some((source, parsed.sources[from], parsed.sources[to - 1] + 1))
}

/// The style bits any visible character of source selection `start..end`
/// has, for pressed toolbar buttons.
pub fn selection_styles(text: &str, start: usize, end: usize) -> i32 {
    let shown = visible(text);
    let from = shown
        .sources
        .partition_point(|index| *index < start.min(end));
    let to = shown
        .sources
        .partition_point(|index| *index < start.max(end));
    shown.styles[from..to].iter().fold(0, |bits, s| bits | s)
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

    /// Every toolbar toggle web makes (`toggle-cases.json`, from web).
    #[test]
    fn toggles_follow_the_shared_cases() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("toggle-cases.json")).unwrap();
        let cases = cases["cases"].as_array().unwrap();
        assert!(cases.len() > 300);
        for case in cases {
            let text = case["text"].as_str().unwrap();
            let style = match case["style"].as_str().unwrap() {
                "bold" => BOLD,
                "italic" => ITALIC,
                "underline" => UNDERLINE,
                "strikethrough" => STRIKETHROUGH,
                "spoiler" => SPOILER,
                _ => QUOTE,
            };
            // Visible UTF-16 offsets to visible scalars, then to the source.
            let shown = visible(text);
            let scalar = |key: &str| {
                let units = case[key].as_u64().unwrap() as usize;
                let mut sum = 0;
                shown
                    .chars
                    .iter()
                    .take_while(|c| {
                        sum += c.len_utf16();
                        sum <= units
                    })
                    .count()
            };
            let (from, to) = (scalar("from"), scalar("to"));
            let start = shown
                .sources
                .get(from)
                .copied()
                .unwrap_or(text.chars().count());
            let end = if to > from {
                shown.sources[to - 1] + 1
            } else {
                start
            };
            let result = toggle(text, start, end, style);
            let label = format!("{case}");
            match case["result"].as_str() {
                None => assert_eq!(result, None, "{label}"),
                Some(expected) => {
                    let (next, start, end) = result.expect(&label);
                    assert_eq!(next, expected, "{label}");
                    let after = visible(&next).sources;
                    let place = |source: usize| after.partition_point(|index| *index < source);
                    assert_eq!((place(start), place(end)), (from, to), "{label}");
                }
            }
        }
    }

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
