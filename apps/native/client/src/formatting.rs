//! The composer's Markdown subset, aligned with the web formatter: which
//! source characters show, and edits that keep its delimiters balanced.
//! Positions are Unicode scalars unless a range says bytes.

/// Composer edits as web makes them (`composer::edit`).
#[path = "../../../../shared/messages/composer.rs"]
pub mod composer;
#[path = "../../../../shared/messages/formatting.rs"]
pub mod parser;
use crate::links;
pub use parser::runs;

#[derive(Debug)]
pub struct SourceRun {
    pub range: std::ops::Range<usize>,
    pub bold: bool,
    pub italic: bool,
    pub hidden: bool,
}

/// The shared renderer's visible spans and intervening delimiter ranges for egui.
/// Byte ranges describe the unchanged buffer, so native undo/IME/cursors still work.
pub fn source_runs(text: &str) -> Vec<SourceRun> {
    let mut out = Vec::new();
    let mut end = 0;
    parser::visit_runs(text, |range, bold, italic| {
        if end < range.start {
            out.push(SourceRun {
                range: end..range.start,
                bold: false,
                italic: false,
                hidden: true,
            });
        }
        end = range.end;
        out.push(SourceRun {
            range,
            bold,
            italic,
            hidden: false,
        });
    });
    if end < text.len() {
        out.push(SourceRun {
            range: end..text.len(),
            bold: false,
            italic: false,
            hidden: true,
        });
    }
    out
}

/// Visible text and its source scalar positions, used by native token completion.
pub fn projection(text: &str) -> (String, Vec<usize>) {
    let mut visible = String::new();
    let mut indices = Vec::new();
    let mut offset = 0;
    for run in source_runs(text) {
        let value = &text[run.range];
        let end = offset + value.chars().count();
        if !run.hidden {
            visible.push_str(value);
            indices.extend(offset..end);
        }
        offset = end;
    }
    (visible, indices)
}

pub use composer::{Edited, edit};

/// One composer edit without a style carried from the previous key
/// (`composer::edit` with `continuing` -1).
pub fn repair_edit(previous: &str, next: &str, start: usize, end: usize) -> (String, usize, usize) {
    let edited = edit(previous, next, start, end, -1);
    (edited.text, edited.start, edited.end)
}

/// Selection offsets are Unicode scalar positions, as used by egui.
pub fn toggle(
    text: &str,
    mut start: usize,
    mut end: usize,
    marker: &str,
) -> (String, usize, usize) {
    if start == end {
        return (text.into(), start, end);
    }
    let c: Vec<char> = text.chars().collect();
    let mut offset = 0;
    for part in links::segments(text) {
        let limit = offset + part.text.chars().count();
        if part.href.is_some()
            && ((start > offset && start < limit) || (end > offset && end < limit))
        {
            return (text.into(), start, end);
        }
        offset = limit;
    }
    if start != end {
        let whitespace = |c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}';
        while start < end && whitespace(c[start]) {
            start += 1;
        }
        while end > start && whitespace(c[end - 1]) {
            end -= 1;
        }
        if start == end {
            return (text.into(), start, end);
        }
    }
    let m: Vec<char> = marker.chars().collect();
    let w = m.len();
    if start != end {
        let styles = source_runs(text);
        let visible: Vec<_> = text
            .char_indices()
            .enumerate()
            .filter(|(index, (byte, _))| {
                *index >= start
                    && *index < end
                    && styles
                        .iter()
                        .any(|run| !run.hidden && run.range.contains(byte))
            })
            .map(|(index, _)| index)
            .collect();
        if let (Some(&first), Some(&last)) = (visible.first(), visible.last())
            && first >= w
            && c.get(first - w..first) == Some(&m[..])
            && c.get(last + 1..last + 1 + w) == Some(&m[..])
            && runs(&c[first - w..last + 1 + w].iter().collect::<String>())
                .iter()
                .all(|run| if marker == "**" { run.bold } else { run.italic })
        {
            start = first;
            end = last + 1;
        }
    }
    let mut selected: String = c[start..end].iter().collect();
    let parsed = runs(&selected);
    let has_style = |run: &parser::Run| if marker == "**" { run.bold } else { run.italic };
    let (a, b, value, s, e) = if start >= w
        && c.get(start - w..start) == Some(&m[..])
        && c.get(end..end + w) == Some(&m[..])
    {
        (
            start - w,
            end + w,
            c[start..end].iter().collect::<String>(),
            start - w,
            end - w,
        )
    } else if end - start >= w * 2
        && c[start..end].starts_with(&m)
        && c[start..end].ends_with(&m)
        && parsed.iter().all(has_style)
    {
        (
            start,
            end,
            c[start + w..end - w].iter().collect(),
            start,
            end - w * 2,
        )
    } else {
        if parsed.iter().any(has_style) {
            let mut remaining: Vec<(String, bool)> = Vec::new();
            for run in parsed {
                let styled = if marker == "**" { run.italic } else { run.bold };
                if let Some(last) = remaining.last_mut().filter(|last| last.1 == styled) {
                    last.0.push_str(&run.text);
                } else {
                    remaining.push((run.text, styled));
                }
            }
            let other = if marker == "**" { "_" } else { "**" };
            selected = remaining
                .into_iter()
                .map(|(text, styled)| {
                    if styled {
                        format!("{other}{text}{other}")
                    } else {
                        text
                    }
                })
                .collect();
        }
        (
            start,
            end,
            format!("{marker}{selected}{marker}"),
            start + w,
            start + w + selected.chars().count(),
        )
    };
    let result = format!(
        "{}{}{}",
        c[..a].iter().collect::<String>(),
        value,
        c[b..].iter().collect::<String>()
    );
    let displayed = |value: &str| {
        runs(value)
            .into_iter()
            .map(|run| run.text)
            .collect::<String>()
    };
    if start != end && displayed(&result) != displayed(text) {
        return (text.into(), start, end);
    }
    (result, s, e)
}
