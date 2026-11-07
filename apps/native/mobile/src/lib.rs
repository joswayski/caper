//! Shared Rust logic for the Android and Apple apps.
//!
//! The parsers are the same sources the desktop clients and the API compile
//! (`shared/messages`). Results cross the boundary as flat `i32` arrays of
//! source offsets, so the native UI keeps slicing its own strings: UTF-16
//! code units for both hosts, plus UTF-8 bytes for Swift, whose strings are
//! UTF-8 inside.
//!
//! - Apple: the C ABI in `include/caper_mobile.h`, linked statically from
//!   `CaperMobile.xcframework` (`build-apple.sh`).
//! - Android: JNI functions for `chat.caper.android.data.CaperMobile`
//!   (feature `jni`), built per ABI by cargo-ndk from Gradle.

#[path = "../../../../shared/messages/formatting.rs"]
pub mod formatting;
#[path = "../../../../shared/messages/links.rs"]
pub mod links;
/// The name-character rule, as the desktop clients compile it.
#[path = "../../client/src/names.rs"]
pub mod names;

mod c_abi;
#[cfg(feature = "jni")]
mod jni;

/// Style bits in a formatting span.
pub const BOLD: i32 = 1;
pub const ITALIC: i32 = 2;

/// Converts increasing byte offsets in one string to UTF-16 offsets.
struct Utf16Cursor<'a> {
    text: &'a str,
    byte: usize,
    unit: usize,
}

impl<'a> Utf16Cursor<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            byte: 0,
            unit: 0,
        }
    }

    fn at(&mut self, byte: usize) -> usize {
        self.unit += self.text[self.byte..byte]
            .chars()
            .map(char::len_utf16)
            .sum::<usize>();
        self.byte = byte;
        self.unit
    }
}

fn offset(value: usize) -> i32 {
    // Drafts and messages are far below 2 GiB; hosts index with 32-bit ints.
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// The visible source ranges of formatted message text, in order; the gaps
/// are its hidden `**` / `_` delimiters. Each span is `[utf16_start,
/// utf16_end, flags]`, or `[utf16_start, utf16_end, utf8_start, utf8_end,
/// flags]` with `utf8`. Adjacent spans may share a style.
pub fn format_spans(text: &str, utf8: bool) -> Vec<i32> {
    let mut out = Vec::new();
    let mut cursor = Utf16Cursor::new(text);
    formatting::visit_runs(text, |range, bold, italic| {
        out.push(offset(cursor.at(range.start)));
        out.push(offset(cursor.at(range.end)));
        if utf8 {
            out.extend([offset(range.start), offset(range.end)]);
        }
        out.push(if bold { BOLD } else { 0 } | if italic { ITALIC } else { 0 });
    });
    out
}

/// The links in plain message text. Each is `[utf16_start, utf16_end,
/// scheme]`, or `[utf16_start, utf16_end, utf8_start, utf8_end, scheme]` with
/// `utf8`. `scheme` is the length of the `https://` (8) or `http://` (7)
/// prefix the link starts with, ASCII case-insensitively, or 0 for `www.`:
/// the link opens that scheme in lowercase followed by the rest of its text,
/// and `https://` followed by its text for `www.`.
pub fn link_spans(text: &str, utf8: bool) -> Vec<i32> {
    let mut out = Vec::new();
    let mut cursor = Utf16Cursor::new(text);
    let mut start = 0;
    for segment in links::segments(text) {
        let end = start + segment.text.len();
        if segment.href.is_some() {
            out.push(offset(cursor.at(start)));
            out.push(offset(cursor.at(end)));
            if utf8 {
                out.extend([offset(start), offset(end)]);
            }
            let prefix = |scheme: &str| {
                segment
                    .text
                    .get(..scheme.len())
                    .is_some_and(|value| value.eq_ignore_ascii_case(scheme))
            };
            out.push(if prefix("https://") {
                8
            } else if prefix("http://") {
                7
            } else {
                0
            });
        }
        start = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str, range: std::ops::Range<i32>) -> String {
        let units: Vec<u16> = text.encode_utf16().collect();
        String::from_utf16(&units[range.start as usize..range.end as usize]).unwrap()
    }

    /// The hosts rebuild `formatting::runs` from the spans; every shared case agrees.
    #[test]
    fn spans_rebuild_the_shared_formatting_cases() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../shared/messages/formatting-cases.json"
        ))
        .unwrap();
        let cases = cases["cases"].as_array().unwrap();
        assert!(cases.len() > 10);
        for case in cases {
            let text = case["text"].as_str().unwrap();
            let expected: Vec<(String, bool, bool)> = formatting::runs(text)
                .into_iter()
                .map(|run| (run.text, run.bold, run.italic))
                .collect();
            for (utf8, width) in [(false, 3), (true, 5)] {
                let spans = format_spans(text, utf8);
                let mut runs: Vec<(String, bool, bool)> = Vec::new();
                for span in spans.chunks(width) {
                    let value = utf16(text, span[0]..span[1]);
                    if utf8 {
                        assert_eq!(value, text[span[2] as usize..span[3] as usize]);
                    }
                    let style = span[width - 1];
                    let (bold, italic) = (style & BOLD != 0, style & ITALIC != 0);
                    match runs.last_mut() {
                        Some(last) if last.1 == bold && last.2 == italic => last.0 += &value,
                        _ => runs.push((value, bold, italic)),
                    }
                }
                assert_eq!(runs, expected, "{text}");
            }
        }
    }

    /// Hosts rebuild each link's address from its scheme code; every shared case agrees.
    #[test]
    fn spans_rebuild_the_shared_link_cases() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../../../../shared/messages/link-cases.json"))
                .unwrap();
        let cases = cases["cases"].as_array().unwrap();
        assert!(cases.len() > 30);
        for case in cases {
            let text = case["text"].as_str().unwrap();
            let expected: Vec<(String, String)> = case["segments"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|segment| {
                    Some((
                        segment["text"].as_str()?.to_owned(),
                        segment["href"].as_str()?.to_owned(),
                    ))
                })
                .collect();
            for (utf8, width) in [(false, 3), (true, 5)] {
                let actual: Vec<(String, String)> = link_spans(text, utf8)
                    .chunks(width)
                    .map(|span| {
                        let value = utf16(text, span[0]..span[1]);
                        if utf8 {
                            assert_eq!(value, text[span[2] as usize..span[3] as usize]);
                        }
                        let href = match span[width - 1] {
                            0 => format!("https://{value}"),
                            8 => format!("https://{}", &value[8..]),
                            7 => format!("http://{}", &value[7..]),
                            other => panic!("scheme {other}"),
                        };
                        (value, href)
                    })
                    .collect();
                assert_eq!(actual, expected, "{text}");
            }
        }
    }

    #[test]
    fn names_follow_the_shared_cases() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../../../../shared/names/cases.json")).unwrap();
        for (key, hidden) in [("visible", false), ("hidden", true)] {
            let names = cases[key].as_array().unwrap();
            assert!(names.len() > 10);
            for name in names {
                let name = name.as_str().unwrap();
                assert_eq!(names::has_hidden(name), hidden, "{name:?}");
            }
        }
    }

    /// Offsets count UTF-16 units and UTF-8 bytes separately past astral and accented characters.
    #[test]
    fn offsets_differ_by_encoding_after_non_ascii_text() {
        let text = "é😀 **b** HTTPS://x.io";
        // "é" is 1 unit / 2 bytes, "😀" 2 units / 4 bytes.
        assert_eq!(
            format_spans(text, true),
            vec![0, 4, 0, 7, 0, 6, 7, 9, 10, 1, 9, 22, 12, 25, 0]
        );
        assert_eq!(link_spans(text, false), vec![10, 22, 8]);
        assert_eq!(link_spans("see www.a.b.c", true), vec![4, 13, 4, 13, 0]);
    }
}
