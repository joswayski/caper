//! Message formatting shared by the API's mention resolver and the native
//! clients: Discord's inline `**bold**`, `_italic_`, `__underline__`,
//! `~~strikethrough~~` and `||spoiler||`, and `> ` quoted lines.
use crate::links;

/// Style bits of a run. Hosts receive these bits through the mobile bridge.
pub const BOLD: i32 = 1;
pub const ITALIC: i32 = 2;
pub const UNDERLINE: i32 = 4;
pub const STRIKETHROUGH: i32 = 8;
pub const SPOILER: i32 = 16;
/// A whole `> ` line. The newline between two quoted lines is quoted too, so
/// consecutive quoted characters form one block.
pub const QUOTE: i32 = 32;

/// Inline delimiters in the order they are tried at one position; `__` before
/// `_` so `___x___` is underlined italics.
pub const MARKERS: [(i32, &[char]); 5] = [
    (BOLD, &['*', '*']),
    (UNDERLINE, &['_', '_']),
    (STRIKETHROUGH, &['~', '~']),
    (SPOILER, &['|', '|']),
    (ITALIC, &['_']),
];

#[derive(Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub spoiler: bool,
    pub quote: bool,
}

impl Run {
    pub fn style(&self) -> i32 {
        [
            (self.bold, BOLD),
            (self.italic, ITALIC),
            (self.underline, UNDERLINE),
            (self.strikethrough, STRIKETHROUGH),
            (self.spoiler, SPOILER),
            (self.quote, QUOTE),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .fold(0, |bits, (_, bit)| bits | bit)
    }
}

pub fn runs(text: &str) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    visit_runs(text, |range, style| {
        let value = &text[range];
        if let Some(last) = out.last_mut().filter(|r| r.style() == style) {
            last.text.push_str(value);
        } else {
            out.push(Run {
                text: value.to_owned(),
                bold: style & BOLD != 0,
                italic: style & ITALIC != 0,
                underline: style & UNDERLINE != 0,
                strikethrough: style & STRIKETHROUGH != 0,
                spoiler: style & SPOILER != 0,
                quote: style & QUOTE != 0,
            });
        }
    });
    out
}

/// White space as web's parser sees it (JavaScript's `\s`).
pub fn whitespace(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// Whether the line `c[start..end]` (no newline) is a quote: `> ` and some
/// non-blank text.
pub fn quoted_line(c: &[char], start: usize, end: usize) -> bool {
    end > start + 2
        && c[start] == '>'
        && c[start + 1] == ' '
        && !c[start + 2..end].iter().all(|c| whitespace(*c))
}

/// Visits visible source byte ranges in order with their style bits. Gaps
/// are recognized delimiters. Both rendering and editable projections use
/// this single grammar.
pub fn visit_runs(text: &str, mut emit: impl FnMut(std::ops::Range<usize>, i32)) {
    let chars: Vec<char> = text.chars().collect();
    let mut protected = Vec::new();
    for part in links::segments(text) {
        protected.extend(std::iter::repeat_n(
            part.href.is_some(),
            part.text.chars().count(),
        ));
    }
    let offsets: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain([text.len()])
        .collect();
    let mut out = |start: usize, end: usize, style: i32| {
        if start < end {
            emit(offsets[start]..offsets[end], style);
        }
    };
    // Unquoted lines parse together, so emphasis may span them; each quoted
    // line parses alone.
    let n = chars.len();
    let mut lines = Vec::new();
    let mut start = 0;
    loop {
        let end = (start..n).find(|i| chars[*i] == '\n').unwrap_or(n);
        lines.push((start, end, quoted_line(&chars, start, end)));
        if end == n {
            break;
        }
        start = end + 1;
    }
    let mut index = 0;
    while index < lines.len() {
        let (start, end, quoted) = lines[index];
        if quoted {
            parse(&chars, &protected, start + 2, end, QUOTE, &mut out);
            index += 1;
        } else {
            let mut last = index;
            while last + 1 < lines.len() && !lines[last + 1].2 {
                last += 1;
            }
            parse(&chars, &protected, start, lines[last].1, 0, &mut out);
            index = last + 1;
        }
        let (_, end, quoted) = lines[index - 1];
        if let Some(&(_, _, next)) = lines.get(index) {
            out(end, end + 1, if quoted && next { QUOTE } else { 0 });
        }
    }
}

fn word(c: Option<&char>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '@'))
}

fn marker(c: &[char], protected: &[bool], i: usize, m: &[char]) -> bool {
    c.get(i..i + m.len()) == Some(m)
        && !protected[i]
        && c[..i].iter().rev().take_while(|c| **c == '\\').count() % 2 == 0
        // `***`, `~~~` and `|||` stay literal; underscores follow word rules.
        && (m.len() != 2
            || m[0] == '_'
            || (c.get(i.wrapping_sub(1)) != Some(&m[0]) && c.get(i + 2) != Some(&m[0])))
}

/// Parses `c[start..end]`, already styled `style`, emitting visible ranges.
/// Underscore delimiters may not touch a word character inside the range; a
/// delimiter just outside it belongs to an enclosing style.
fn parse(
    c: &[char],
    p: &[bool],
    start: usize,
    end: usize,
    style: i32,
    out: &mut impl FnMut(usize, usize, i32),
) {
    let mut plain = start;
    let mut i = start;
    'scan: while i < end {
        for (bit, m) in MARKERS {
            let len = m.len();
            let underscore = m[0] == '_';
            let inner = i + len;
            if style & bit != 0
                || inner >= end
                || !marker(c, p, i, m)
                || whitespace(c[inner])
                || (underscore && i > start && word(c.get(i - 1)))
                || (len == 1 && c[inner] == '_')
            {
                continue;
            }
            let mut close = inner + 1;
            while close + len <= end {
                if marker(c, p, close, m)
                    && !whitespace(c[close - 1])
                    && !(underscore && close + len < end && word(c.get(close + len)))
                    && !(len == 1 && c[close - 1] == '_')
                {
                    out(plain, i, style);
                    parse(c, p, inner, close, style | bit, out);
                    i = close + len;
                    plain = i;
                    continue 'scan;
                }
                close += 1;
            }
        }
        i += 1;
    }
    out(plain, end, style);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_parser_contract() {
        #[derive(serde::Deserialize)]
        struct Case {
            text: String,
            runs: Vec<Run>,
        }
        #[derive(serde::Deserialize)]
        struct Cases {
            cases: Vec<Case>,
        }
        let cases: Cases = serde_json::from_str(include_str!("formatting-cases.json")).unwrap();
        for case in cases.cases {
            assert_eq!(runs(&case.text), case.runs, "{}", case.text);
        }
    }
}
