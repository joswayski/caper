//! Message formatting shared by the API's mention resolver and the native
//! clients: Discord's inline `**bold**`, `_italic_`, `__underline__`,
//! `~~strikethrough~~`, `||spoiler||`, `` `code` ``, `> ` quoted lines, and
//! fenced ```` ``` ```` code blocks.
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
/// Inline code (`` `...` ``). Markdown delimiters inside stay literal.
pub const CODE: i32 = 64;
/// Fenced code block (```` ```[lang]\n...\n``` ```` or single-line ```` ```...``` ````).
/// Fence characters stay in the visible stream so composers preserve language tags;
/// message renderers split code blocks with `code::parse_blocks`.
pub const CODE_BLOCK: i32 = 128;

/// Inline delimiters in the order they are tried at one position; `__` before
/// `_` so `___x___` is underlined italics, and `**` before `*`.
pub const MARKERS: [(i32, &[char]); 7] = [
    (BOLD, &['*', '*']),
    (UNDERLINE, &['_', '_']),
    (STRIKETHROUGH, &['~', '~']),
    (SPOILER, &['|', '|']),
    (ITALIC, &['_']),
    (ITALIC, &['*']),
    (CODE, &['`']),
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
    pub code: bool,
    #[serde(rename = "codeBlock")]
    pub code_block: bool,
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
            (self.code, CODE),
            (self.code_block, CODE_BLOCK),
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
                code: style & CODE != 0,
                code_block: style & CODE_BLOCK != 0,
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

/// Whether a fenced code block opening info string is valid (optional language
/// tag containing ASCII identifier/symbol chars and optional surrounding spaces).
pub fn valid_fence_info(c: &[char], start: usize, end: usize) -> bool {
    let mut a = start;
    let mut b = end;
    while a < b && whitespace(c[a]) {
        a += 1;
    }
    while b > a && whitespace(c[b - 1]) {
        b -= 1;
    }
    c[a..b]
        .iter()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '+' | '#' | '.' | '-'))
}

/// Whether `c[start..end]` is a closing fence line: ```` ``` ```` + optional whitespace.
pub fn is_closing_fence(c: &[char], start: usize, end: usize) -> bool {
    end >= start + 3
        && c[start] == '`'
        && c[start + 1] == '`'
        && c[start + 2] == '`'
        && c[start + 3..end].iter().all(|ch| whitespace(*ch))
}

/// Whether `c[start..end]` is a single-line fenced block: ```` ```code``` ````
/// (with optional trailing spaces, non-blank inner content, and no extra ```` ``` ````).
pub fn single_line_fence(c: &[char], start: usize, end: usize) -> Option<(usize, usize)> {
    let mut trim_end = end;
    while trim_end > start && whitespace(c[trim_end - 1]) {
        trim_end -= 1;
    }
    if trim_end <= start + 6 {
        return None;
    }
    if c[start..start + 3] != ['`', '`', '`'] || c[trim_end - 3..trim_end] != ['`', '`', '`'] {
        return None;
    }
    let inner_start = start + 3;
    let inner_end = trim_end - 3;
    if c[inner_start] == '`' || c[inner_end - 1] == '`' {
        return None;
    }
    if c[inner_start..inner_end].contains(&'\n') {
        return None;
    }
    if c[inner_start..inner_end].iter().all(|ch| whitespace(*ch)) {
        return None;
    }
    if c[inner_start..inner_end]
        .windows(3)
        .any(|w| w == ['`', '`', '`'])
    {
        return None;
    }
    Some((inner_start, inner_end))
}

/// Finds the closing line index for a fenced code block starting at `lines[index]`,
/// returning `Some(close_index)` (`close_index == index` for single-line fences).
pub fn fenced_block_end(c: &[char], lines: &[(usize, usize)], index: usize) -> Option<usize> {
    let (start, end) = lines[index];
    if end < start + 3 || c[start..start + 3] != ['`', '`', '`'] {
        return None;
    }
    if single_line_fence(c, start, end).is_some() {
        return Some(index);
    }
    if !valid_fence_info(c, start + 3, end) {
        return None;
    }
    ((index + 1)..lines.len()).find(|&j| is_closing_fence(c, lines[j].0, lines[j].1))
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
    let n = chars.len();
    let mut raw_lines = Vec::new();
    let mut start = 0;
    loop {
        let end = (start..n).find(|i| chars[*i] == '\n').unwrap_or(n);
        raw_lines.push((start, end));
        if end == n {
            break;
        }
        start = end + 1;
    }
    // Tag lines that belong to fenced code blocks first so their contents
    // never participate in quote or inline Markdown parsing.
    let mut is_fence = vec![false; raw_lines.len()];
    let mut idx = 0;
    while idx < raw_lines.len() {
        if let Some(close) = fenced_block_end(&chars, &raw_lines, idx) {
            for flag in &mut is_fence[idx..=close] {
                *flag = true;
            }
            idx = close + 1;
        } else {
            idx += 1;
        }
    }
    let lines: Vec<(usize, usize, bool, bool)> = raw_lines
        .iter()
        .enumerate()
        .map(|(i, &(s, e))| (s, e, !is_fence[i] && quoted_line(&chars, s, e), is_fence[i]))
        .collect();
    // Unquoted non-fence lines parse together, so emphasis may span them; each
    // quoted line parses alone; each fenced block emits verbatim with CODE_BLOCK.
    let mut index = 0;
    while index < lines.len() {
        let (start, end, quoted, fence) = lines[index];
        if fence {
            let close = fenced_block_end(&chars, &raw_lines, index).unwrap_or(index);
            out(start, lines[close].1, CODE_BLOCK);
            index = close + 1;
        } else if quoted {
            parse(&chars, &protected, start + 2, end, QUOTE, &mut out);
            index += 1;
        } else {
            let mut last = index;
            while last + 1 < lines.len() && !lines[last + 1].2 && !lines[last + 1].3 {
                last += 1;
            }
            parse(&chars, &protected, start, lines[last].1, 0, &mut out);
            index = last + 1;
        }
        let (_, end, quoted, _) = lines[index - 1];
        if let Some(&(_, _, next_quoted, _)) = lines.get(index) {
            out(end, end + 1, if quoted && next_quoted { QUOTE } else { 0 });
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
        // `***`, `~~~`, `|||` and `` `` `` stay literal; underscores follow word rules.
        && (m[0] == '_'
            || (c.get(i.wrapping_sub(1)) != Some(&m[0]) && c.get(i + m.len()) != Some(&m[0])))
}

/// Parses `c[start..end]`, already styled `style`, emitting visible ranges.
/// Underscore delimiters may not touch a word character inside the range; a
/// delimiter just outside it belongs to an enclosing style. Inline code
/// (`` `...` ``) does not span newlines and suppresses nested Markdown.
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
            let single_underscore = m == ['_'];
            let backtick = bit == CODE;
            let inner = i + len;
            if style & bit != 0
                || inner >= end
                || !marker(c, p, i, m)
                || whitespace(c[inner])
                || (single_underscore && i > start && word(c.get(i - 1)))
                || (single_underscore && c[inner] == '_')
            {
                continue;
            }
            let mut close = inner + 1;
            while close + len <= end {
                if backtick && c[close - 1] == '\n' {
                    break;
                }
                if marker(c, p, close, m)
                    && !whitespace(c[close - 1])
                    && !(single_underscore && close + len < end && word(c.get(close + len)))
                    && !(single_underscore && c[close - 1] == '_')
                    && !(m == ['_', '_'] && c.get(close + len) == Some(&'_'))
                {
                    out(plain, i, style);
                    if backtick {
                        // Everything inside inline code stays literal.
                        out(inner, close, style | CODE);
                    } else {
                        parse(c, p, inner, close, style | bit, out);
                    }
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
