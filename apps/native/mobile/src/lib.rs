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

/// Composer edits as web makes them; it finds the grammar as `super::parser`.
#[path = "../../../../shared/messages/composer.rs"]
pub mod composer;
/// Web's emoji autocomplete; it finds the grammar as `super::parser`.
#[path = "../../../../shared/messages/emoji.rs"]
pub mod emoji;
#[path = "../../../../shared/messages/formatting.rs"]
pub mod formatting;
use formatting as parser;
#[path = "../../../../shared/messages/history_diff.rs"]
pub mod history_diff;
/// Web's pasted-invite parser; it returns only the code.
#[path = "../../../../shared/invites/code.rs"]
pub mod invites;
#[path = "../../../../shared/messages/links.rs"]
pub mod links;
pub mod media;
#[path = "../../../../shared/messages/mentions.rs"]
pub mod mentions;
#[path = "../../../../shared/media/metadata.rs"]
pub mod metadata;
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

/// The `@mention` tokens in message text. Each is `[utf16_start,
/// utf16_end]`, or `[utf16_start, utf16_end, utf8_start, utf8_end]` with
/// `utf8`, covering the `@` and its name; the name is ASCII, so hosts
/// lowercase it themselves.
pub fn mention_spans(text: &str, utf8: bool) -> Vec<i32> {
    let mut out = Vec::new();
    let mut cursor = Utf16Cursor::new(text);
    for (range, _) in mentions::tokens(text) {
        out.push(offset(cursor.at(range.start)));
        out.push(offset(cursor.at(range.end)));
        if utf8 {
            out.extend([offset(range.start), offset(range.end)]);
        }
    }
    out
}

/// What an edit-history comparison highlights (`history_diff`): `[count,
/// ...before spans, ...after spans]`, where `count` is the number of before
/// spans and each span is `[utf16_start, utf16_end]`, or `[utf16_start,
/// utf16_end, utf8_start, utf8_end]` with `utf8`. Text between spans is not
/// highlighted.
pub fn history_diff_spans(before: &str, after: &str, utf8: bool) -> Vec<i32> {
    let (old, new) = history_diff::highlights(before, after);
    let push = |out: &mut Vec<i32>, text: &str, runs: history_diff::Highlights| {
        let mut cursor = Utf16Cursor::new(text);
        let mut count = 0;
        for (range, _) in runs.into_iter().filter(|(_, changed)| *changed) {
            out.push(offset(cursor.at(range.start)));
            out.push(offset(cursor.at(range.end)));
            if utf8 {
                out.extend([offset(range.start), offset(range.end)]);
            }
            count += 1;
        }
        count
    };
    let mut out = vec![0];
    out[0] = push(&mut out, before, old);
    push(&mut out, after, new);
    out
}

/// The scalar offset of UTF-16 offset `unit` in `text` (a split surrogate
/// pair counts as its scalar's start).
fn scalar(text: &str, unit: usize) -> usize {
    let mut units = 0;
    text.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= unit
        })
        .count()
}

fn utf16(text: &str, scalar: usize) -> usize {
    text.chars().take(scalar).map(char::len_utf16).sum()
}

/// One composer edit (`composer::edit`) with UTF-16 selection offsets into
/// `next`: `[start, end, pending, ...]` followed by the edited text's UTF-16
/// code units.
pub fn composer_edit(
    previous: &str,
    next: &str,
    start: i32,
    end: i32,
    continuing: i32,
) -> Vec<i32> {
    let unit = |value: i32| usize::try_from(value).unwrap_or_default();
    let edited = composer::edit(
        previous,
        next,
        scalar(next, unit(start)),
        scalar(next, unit(end)),
        continuing,
    );
    let mut out = vec![
        offset(utf16(&edited.text, edited.start)),
        offset(utf16(&edited.text, edited.end)),
        edited.pending,
    ];
    out.extend(edited.text.encode_utf16().map(i32::from));
    out
}

/// The host's catalog search, from the bytes it ships (`emoji_catalog`).
static EMOJI: std::sync::OnceLock<emoji::Search> = std::sync::OnceLock::new();

/// Loads `shared/emoji/catalog.json` as the app ships it, for
/// `emoji_suggestions`. The first catalog stays; false when `json` is not one.
pub fn emoji_catalog(json: &[u8]) -> bool {
    #[derive(serde::Deserialize)]
    struct Entry {
        id: String,
        name: String,
        keywords: String,
        selectable: bool,
    }
    if EMOJI.get().is_some() {
        return true;
    }
    let Ok(entries) = serde_json::from_slice::<Vec<Entry>>(json) else {
        return false;
    };
    let _ = EMOJI.set(emoji::Search::new(entries.iter().map(|entry| {
        emoji::Choice {
            id: &entry.id,
            name: &entry.name,
            keywords: &entry.keywords,
            selectable: entry.selectable,
        }
    })));
    true
}

/// The `:` token with the caret at UTF-16 offset `caret`: `[start, end]` as a
/// UTF-16 source range, then the query's characters (ASCII), or nothing.
pub fn emoji_token(text: &str, caret: i32) -> Vec<i32> {
    let caret = scalar(text, usize::try_from(caret).unwrap_or_default());
    emoji::token(text, caret).map_or_else(Vec::new, |token| {
        let mut out = vec![
            offset(utf16(text, token.start)),
            offset(utf16(text, token.end)),
        ];
        out.extend(token.query.chars().map(|c| c as i32));
        out
    })
}

/// Positions in the loaded catalog web offers for `query`, best first; none
/// before `emoji_catalog`.
pub fn emoji_suggestions(query: &str) -> Vec<i32> {
    EMOJI.get().map_or_else(Vec::new, |search| {
        search.suggestions(query).into_iter().map(offset).collect()
    })
}

/// `part`, a slice of `text`, as `[utf16_start, utf16_end]`, or `[utf16_start,
/// utf16_end, utf8_start, utf8_end]` with `utf8`.
fn slice_span(text: &str, part: &str, utf8: bool) -> Vec<i32> {
    let start = part.as_ptr() as usize - text.as_ptr() as usize;
    let end = start + part.len();
    let mut cursor = Utf16Cursor::new(text);
    let mut out = vec![offset(cursor.at(start)), offset(cursor.at(end))];
    if utf8 {
        out.extend([offset(start), offset(end)]);
    }
    out
}

/// One value per character of ASCII text.
fn ascii(text: &str) -> Vec<i32> {
    text.bytes().map(i32::from).collect()
}

/// Web's `normalizeChannelName` (`names::normalize_channel`): the channel
/// name as typed, one ASCII character per value.
pub fn channel_name(text: &str) -> Vec<i32> {
    ascii(&names::normalize_channel(text))
}

/// Web's `channelNameError` (`channel`) or `spaceNameError` for `text`: the
/// message's ASCII characters, or nothing when the name is valid.
pub fn name_error(text: &str, channel: bool) -> Vec<i32> {
    let error = if channel {
        names::channel_name_error(text)
    } else {
        names::space_name_error(text)
    };
    error.map_or_else(Vec::new, ascii)
}

/// The space name web sends for `text` (`name.trim()`), as `slice_span`.
pub fn space_name_span(text: &str, utf8: bool) -> Vec<i32> {
    slice_span(text, names::space_name(text), utf8)
}

/// Web's `inviteCode`: the ten-character code in a pasted or opened invite,
/// one ASCII character per value, or nothing. Never the link's host.
pub fn invite_code(text: &str) -> Vec<i32> {
    invites::invite_code(text).map_or_else(Vec::new, ascii)
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

    /// Hosts type each step at their own caret mapping; every shared case agrees.
    #[test]
    fn composer_edits_follow_the_shared_cases() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../shared/messages/composer-cases.json"
        ))
        .unwrap();
        let cases = cases["cases"].as_array().unwrap();
        assert!(cases.len() > 20);
        // The visible offset's source offset, keeping a trailing caret inside
        // closing delimiters (Android) or after them (Apple).
        let source = |text: &str, visible: usize, inside: bool| -> usize {
            let spans = format_spans(text, false);
            let indices: Vec<usize> = spans
                .chunks(3)
                .flat_map(|span| span[0] as usize..span[1] as usize)
                .collect();
            indices.get(visible).copied().unwrap_or(if inside {
                indices.last().map_or(0, |index| index + 1)
            } else {
                text.encode_utf16().count()
            })
        };
        let displayed = |text: &str, caret: usize| -> usize {
            format_spans(text, false)
                .chunks(3)
                .map(|span| {
                    (span[1] as usize)
                        .min(caret)
                        .saturating_sub(span[0] as usize)
                })
                .sum()
        };
        for case in cases {
            for inside in [true, false] {
                let mut text = case["text"].as_str().unwrap().to_owned();
                let mut caret = source(&text, case["caret"].as_u64().unwrap() as usize, inside);
                let mut pending = -1;
                for step in case["steps"].as_array().unwrap() {
                    if step["placed"].as_bool().unwrap() {
                        caret = source(&text, displayed(&text, caret), inside);
                        pending = -1;
                    }
                    let key = step["key"].as_str().unwrap();
                    let units: Vec<u16> = text.encode_utf16().collect();
                    let next = String::from_utf16(
                        &[
                            &units[..caret],
                            &key.encode_utf16().collect::<Vec<_>>(),
                            &units[caret..],
                        ]
                        .concat(),
                    )
                    .unwrap();
                    let at = (caret + key.encode_utf16().count()) as i32;
                    let result = composer_edit(&text, &next, at, at, pending);
                    text = String::from_utf16(
                        &result[3..]
                            .iter()
                            .map(|unit| *unit as u16)
                            .collect::<Vec<_>>(),
                    )
                    .unwrap();
                    caret = result[0] as usize;
                    pending = result[2];
                    assert_eq!(
                        (text.as_str(), displayed(&text, caret), pending),
                        (
                            step["text"].as_str().unwrap(),
                            step["caret"].as_u64().unwrap() as usize,
                            step["pending"].as_i64().unwrap() as i32
                        ),
                        "{} (inside {inside})",
                        case["note"]
                    );
                }
            }
        }
    }

    #[test]
    fn composer_replacement_preserves_literals_and_utf16_selection() {
        for prefix in ["", "🙂 ", "e\u{301} 👩‍💻 "] {
            let previous = format!("{prefix}**bold** a");
            let next = format!("{prefix}***");
            let expected = format!("{prefix}*");
            let end = prefix.encode_utf16().count() as i32;
            let result = composer_edit(
                &previous,
                &next,
                next.encode_utf16().count() as i32,
                end,
                -1,
            );
            assert_eq!(
                &result[..3],
                &[expected.encode_utf16().count() as i32, end, -1],
                "{prefix:?}"
            );
            assert_eq!(
                result[3..],
                expected.encode_utf16().map(i32::from).collect::<Vec<_>>(),
                "{prefix:?}"
            );
        }
    }

    /// Hosts rebuild each side's runs from the highlighted spans; every shared case agrees.
    #[test]
    fn spans_rebuild_the_shared_history_diff_cases() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../shared/messages/history-diff-cases.json"
        ))
        .unwrap();
        let cases = cases["cases"].as_array().unwrap();
        assert!(cases.len() > 20);
        for case in cases {
            let (before, after) = (
                case["before"].as_str().unwrap(),
                case["after"].as_str().unwrap(),
            );
            for (utf8, width) in [(false, 2), (true, 4)] {
                let spans = history_diff_spans(before, after, utf8);
                let split = 1 + spans[0] as usize * width;
                for (text, spans, key) in [
                    (before, &spans[1..split], "old"),
                    (after, &spans[split..], "new"),
                ] {
                    let units = text.encode_utf16().count() as i32;
                    let mut runs: Vec<(String, bool)> = Vec::new();
                    let mut push = |value: String, changed: bool| {
                        if !value.is_empty() {
                            runs.push((value, changed));
                        }
                    };
                    let mut end = 0;
                    for span in spans.chunks(width) {
                        if utf8 {
                            assert_eq!(
                                utf16(text, span[0]..span[1]),
                                text[span[2] as usize..span[3] as usize]
                            );
                        }
                        push(utf16(text, end..span[0]), false);
                        push(utf16(text, span[0]..span[1]), true);
                        end = span[1];
                    }
                    push(utf16(text, end..units), false);
                    let expected: Vec<(String, bool)> =
                        serde_json::from_value(case[key].clone()).unwrap();
                    assert_eq!(runs, expected, "{}", case["note"]);
                }
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

    /// The hosts pass the catalog they ship; every shared case agrees.
    #[test]
    fn emoji_autocomplete_follows_the_shared_cases() {
        // Only this test loads the catalog.
        assert!(emoji_suggestions("fire").is_empty());
        assert!(!emoji_catalog(b"not a catalog"));
        let bytes = include_bytes!("../../../../shared/emoji/catalog.json");
        assert!(emoji_catalog(bytes));
        let catalog: Vec<serde_json::Value> = serde_json::from_slice(bytes).unwrap();
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../../../../shared/messages/emoji-cases.json"))
                .unwrap();
        for case in cases["tokens"].as_array().unwrap() {
            let text = case["text"].as_str().unwrap();
            let caret = case["caret"].as_i64().unwrap() as i32;
            let expected: Vec<i32> = case["token"].as_object().map_or_else(Vec::new, |token| {
                let mut out = vec![
                    token["start"].as_i64().unwrap() as i32,
                    token["end"].as_i64().unwrap() as i32,
                ];
                out.extend(token["query"].as_str().unwrap().chars().map(|c| c as i32));
                out
            });
            assert_eq!(emoji_token(text, caret), expected, "{text:?} at {caret}");
        }
        for case in cases["suggestions"].as_array().unwrap() {
            let query = case["query"].as_str().unwrap();
            let ids: Vec<&str> = emoji_suggestions(query)
                .into_iter()
                .map(|index| catalog[index as usize]["id"].as_str().unwrap())
                .collect();
            let expected: Vec<&str> = case["ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap())
                .collect();
            assert_eq!(ids, expected, "{query:?}");
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

    #[test]
    fn space_and_channel_names_follow_the_shared_cases() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../../../../shared/names/channel-cases.json"))
                .unwrap();
        let chars = |values: Vec<i32>| {
            values
                .into_iter()
                .map(|c| c as u8 as char)
                .collect::<String>()
        };
        let message = |case: &serde_json::Value| case["error"].as_str().unwrap_or("").to_owned();
        for case in cases["normalize"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            assert_eq!(
                chars(channel_name(input)),
                case["output"].as_str().unwrap(),
                "{input:?}"
            );
        }
        for case in cases["channelErrors"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            assert_eq!(chars(name_error(name, true)), message(case), "{name:?}");
        }
        for case in cases["spaceNames"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            assert_eq!(chars(name_error(name, false)), message(case), "{name:?}");
            let span = space_name_span(name, true);
            let trimmed = case["trimmed"].as_str().unwrap();
            assert_eq!(utf16(name, span[0]..span[1]), trimmed, "{name:?}");
            assert_eq!(
                &name[span[2] as usize..span[3] as usize],
                trimmed,
                "{name:?}"
            );
        }
    }

    #[test]
    fn invite_codes_follow_the_shared_cases() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../../../../shared/invites/invite-cases.json"))
                .unwrap();
        for case in cases["cases"].as_array().unwrap() {
            let input = case["input"].as_str().unwrap();
            let code: String = invite_code(input)
                .into_iter()
                .map(|c| c as u8 as char)
                .collect();
            assert_eq!(code, case["code"].as_str().unwrap_or(""), "{input:?}");
        }
    }

    #[test]
    fn mention_spans_rebuild_the_shared_cases() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../shared/messages/mention-cases.json"
        ))
        .unwrap();
        for case in cases["cases"].as_array().unwrap() {
            let text = case["text"].as_str().unwrap();
            let expected: Vec<&str> = case["tokens"]
                .as_array()
                .unwrap()
                .iter()
                .map(|token| token["text"].as_str().unwrap())
                .collect();
            for (utf8, width) in [(false, 2), (true, 4)] {
                let actual: Vec<String> = mention_spans(text, utf8)
                    .chunks(width)
                    .map(|span| {
                        let value = utf16(text, span[0]..span[1]);
                        if utf8 {
                            assert_eq!(value, text[span[2] as usize..span[3] as usize]);
                        }
                        value
                    })
                    .collect();
                assert_eq!(actual, expected, "{text:?}");
            }
        }
        // "😀 " is 3 UTF-16 units and 5 bytes.
        assert_eq!(mention_spans("😀 @Ab", true), vec![3, 6, 5, 8]);
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
