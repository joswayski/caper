//! Bold/italic message parsing shared by the API's mention resolver and desktop.
use crate::links;

#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
}

pub fn runs(text: &str) -> Vec<Run> {
    let chars: Vec<char> = text.chars().collect();
    let mut protected = Vec::new();
    for part in links::segments(text) {
        protected.extend(std::iter::repeat_n(
            part.href.is_some(),
            part.text.chars().count(),
        ));
    }
    fn word(c: Option<&char>) -> bool {
        c.is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '@'))
    }
    fn whitespace(c: char) -> bool {
        (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
    }
    fn marker(c: &[char], protected: &[bool], i: usize, m: &[char]) -> bool {
        c.get(i..i + m.len()) == Some(m)
            && !protected[i]
            && c[..i].iter().rev().take_while(|c| **c == '\\').count() % 2 == 0
            && (m.len() != 2
                || (c.get(i.wrapping_sub(1)) != Some(&'*') && c.get(i + 2) != Some(&'*')))
    }
    fn append(out: &mut Vec<Run>, c: &[char], bold: bool, italic: bool) {
        if c.is_empty() {
            return;
        }
        let text: String = c.iter().collect();
        if let Some(last) = out
            .last_mut()
            .filter(|r| r.bold == bold && r.italic == italic)
        {
            last.text.push_str(&text);
        } else {
            out.push(Run { text, bold, italic });
        }
    }
    fn parse(
        c: &[char],
        p: &[bool],
        start: usize,
        end: usize,
        bold: bool,
        italic: bool,
        out: &mut Vec<Run>,
    ) {
        let mut plain = start;
        let mut i = start;
        while i < end {
            let m: &[char] = if !bold && marker(c, p, i, &['*', '*']) {
                &['*', '*']
            } else if !italic && marker(c, p, i, &['_']) {
                &['_']
            } else {
                i += 1;
                continue;
            };
            let inner = i + m.len();
            if (m.len() == 1 && word(c.get(i.wrapping_sub(1))))
                || inner >= end
                || whitespace(c[inner])
            {
                i += 1;
                continue;
            }
            let mut close = inner;
            while close + m.len() <= end {
                if close != inner
                    && marker(c, p, close, m)
                    && !whitespace(c[close - 1])
                    && (m.len() != 1 || !word(c.get(close + 1)))
                {
                    append(out, &c[plain..i], bold, italic);
                    parse(
                        c,
                        p,
                        inner,
                        close,
                        bold || m.len() == 2,
                        italic || m.len() == 1,
                        out,
                    );
                    i = close + m.len() - 1;
                    plain = i + 1;
                    break;
                }
                close += 1;
            }
            i += 1;
        }
        append(out, &c[plain..end], bold, italic);
    }
    let mut out = Vec::new();
    parse(&chars, &protected, 0, chars.len(), false, false, &mut out);
    out
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
