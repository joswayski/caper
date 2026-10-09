//! Plain-text link detection for message text, shared by every client.
//!
//! Messages stay plain text on the wire; links are found at render time, after
//! mentions, on the plain runs between them. The rules follow GitHub Flavored
//! Markdown's autolink literals (so a later Markdown renderer can reuse this
//! for bare URLs), with quotes and unbalanced `]` also treated as trailing
//! punctuation. Only `http://`, `https://` and `www.` ever become links.
//!
//! Every client implements this exact algorithm (web: `apps/web/src/chat/links.ts`)
//! and runs the cases in `shared/messages/link-cases.json`. Change them together.

/// A run of message text; `href` is set for a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment<'a> {
    pub text: &'a str,
    pub href: Option<String>,
}

/// Unicode White_Space; links end at the first of these or `<`.
fn whitespace(value: char) -> bool {
    matches!(
        value,
        '\u{9}'..='\u{d}'
            | ' '
            | '\u{85}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

/// A link may start at the beginning, after whitespace, or after one of these.
fn opener(value: char) -> bool {
    whitespace(value) || matches!(value, '(' | '[' | '{' | '<' | '"' | '\'' | '*' | '_' | '~')
}

fn trailing(value: char) -> bool {
    matches!(
        value,
        '?' | '!' | '.' | ',' | ':' | ';' | '*' | '_' | '~' | '"' | '\'' | '>'
    )
}

fn host_character(value: char) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, '_' | '.' | '-')
}

/// The scheme's length in bytes (0 for `www.`) if a link may start `rest`.
fn starts(rest: &str) -> Option<usize> {
    let prefix = |pattern: &str| {
        rest.as_bytes()
            .get(..pattern.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(pattern.as_bytes()))
    };
    if prefix("https://") {
        Some(8)
    } else if prefix("http://") {
        Some(7)
    } else if prefix("www.") {
        Some(0)
    } else {
        None
    }
}

fn count(text: &str, character: char) -> usize {
    text.chars().filter(|value| *value == character).count()
}

/// Drops trailing punctuation, unbalanced closers and a trailing `&entity;`.
fn trim(candidate: &str) -> &str {
    let mut value = candidate;
    while let Some(last) = value.chars().next_back() {
        if trailing(last) {
            let without = &value[..value.len() - last.len_utf8()];
            // `&` + ASCII letters or digits + `;` goes as a whole.
            let name = without.trim_end_matches(|c: char| c.is_ascii_alphanumeric());
            value = if last == ';' && name.len() < without.len() && name.ends_with('&') {
                &name[..name.len() - 1]
            } else {
                without
            };
        } else if (last == ')' && count(value, ')') > count(value, '('))
            || (last == ']' && count(value, ']') > count(value, '['))
        {
            value = &value[..value.len() - 1];
        } else {
            break;
        }
    }
    value
}

/// At least two non-empty labels (three for `www.`), no `_` in the last two.
fn valid_host(host: &str, www: bool) -> bool {
    let labels: Vec<_> = host.split('.').collect();
    labels.len() >= if www { 3 } else { 2 }
        && labels.iter().all(|label| !label.is_empty())
        && !labels[labels.len() - 2..]
            .iter()
            .any(|label| label.contains('_'))
}

/// Splits plain text into text runs and `http(s)://` / `www.` links. The runs
/// concatenate back to `text`; a link's text is never shortened.
pub fn segments(text: &str) -> Vec<Segment<'_>> {
    let mut segments = Vec::new();
    let mut plain = 0;
    let mut previous: Option<char> = None;
    let mut index = 0;
    while let Some(current) = text[index..].chars().next() {
        let scheme = previous
            .is_none_or(opener)
            .then(|| starts(&text[index..]))
            .flatten();
        let Some(scheme) = scheme else {
            previous = Some(current);
            index += current.len_utf8();
            continue;
        };
        let end = text[index..]
            .find(|value: char| whitespace(value) || value == '<')
            .map_or(text.len(), |offset| index + offset);
        let candidate = trim(&text[index..end]);
        let rest = candidate.get(scheme..).unwrap_or_default();
        let host = rest
            .find(|value: char| !host_character(value))
            .map_or(rest, |offset| &rest[..offset]);
        if !valid_host(host, scheme == 0) {
            previous = Some(current);
            index += current.len_utf8();
            continue;
        }
        if index > plain {
            segments.push(Segment {
                text: &text[plain..index],
                href: None,
            });
        }
        let href = if scheme == 0 {
            format!("https://{candidate}")
        } else {
            format!(
                "{}{}",
                candidate[..scheme].to_ascii_lowercase(),
                &candidate[scheme..]
            )
        };
        segments.push(Segment {
            text: candidate,
            href: Some(href),
        });
        index += candidate.len();
        plain = index;
        previous = candidate.chars().next_back();
    }
    if plain < text.len() {
        segments.push(Segment {
            text: &text[plain..],
            href: None,
        });
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Cases {
        cases: Vec<Case>,
    }

    #[derive(serde::Deserialize)]
    struct Case {
        note: String,
        text: String,
        segments: Vec<Expected>,
    }

    #[derive(serde::Deserialize, Debug, PartialEq)]
    struct Expected {
        text: String,
        href: Option<String>,
    }

    #[test]
    fn shared_cases_produce_identical_segments() {
        let cases: Cases =
            serde_json::from_str(include_str!("../../../../shared/messages/link-cases.json"))
                .expect("valid shared link cases");
        assert!(cases.cases.len() > 30, "the shared cases loaded");
        for case in cases.cases {
            let actual: Vec<_> = segments(&case.text)
                .into_iter()
                .map(|segment| Expected {
                    text: segment.text.to_owned(),
                    href: segment.href,
                })
                .collect();
            assert_eq!(actual, case.segments, "{}: {:?}", case.note, case.text);
        }
    }

    #[test]
    fn segments_cover_the_text_and_only_link_http_or_https() {
        for text in [
            "",
            "a (https://example.com/x_(y)) b",
            "www.example.com/a&b; mailto:a@b.com javascript:alert(1)",
            "😀(www.a.b.c)]",
        ] {
            let parts = segments(text);
            assert_eq!(parts.iter().map(|part| part.text).collect::<String>(), text);
            assert!(parts.iter().all(|part| {
                part.href
                    .as_ref()
                    .is_none_or(|href| href.starts_with("https://") || href.starts_with("http://"))
            }));
        }
    }
}
