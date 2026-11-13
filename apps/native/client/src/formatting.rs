//! The composer's Markdown subset, aligned with the web formatter: which
//! source characters show, and edits that keep its delimiters balanced.
//! Styles are the bits of `shared/messages/formatting.rs`.
//! Positions are Unicode scalars unless a range says bytes.

/// Composer edits as web makes them (`composer::edit`).
#[path = "../../../../shared/messages/composer.rs"]
pub mod composer;
#[path = "../../../../shared/messages/formatting.rs"]
pub mod parser;
use crate::links;
pub use parser::runs;

pub use parser::{BOLD, ITALIC, QUOTE, SPOILER, STRIKETHROUGH, UNDERLINE};

#[derive(Debug)]
pub struct SourceRun {
    pub range: std::ops::Range<usize>,
    /// Style bits (`BOLD`, `ITALIC`, ...); 0 for hidden delimiters.
    pub style: i32,
    pub hidden: bool,
}

/// The shared renderer's visible spans and intervening delimiter ranges.
/// Byte ranges describe the unchanged buffer, so native undo/IME/cursors still work.
pub fn source_runs(text: &str) -> Vec<SourceRun> {
    let mut out = Vec::new();
    let mut end = 0;
    parser::visit_runs(text, |range, style| {
        if end < range.start {
            out.push(SourceRun {
                range: end..range.start,
                style: 0,
                hidden: true,
            });
        }
        end = range.end;
        out.push(SourceRun {
            range,
            style,
            hidden: false,
        });
    });
    if end < text.len() {
        out.push(SourceRun {
            range: end..text.len(),
            style: 0,
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

/// The source selection for visible offsets `primary` and `secondary`
/// (`indices` and `len` from `projection` and the source's scalar count). A
/// caret sits just before the next visible character, or at the end of the
/// source past the last one, so typing at the end of a styled run keeps its
/// markers; a range's far end sits just after its last visible character.
pub fn source_selection(
    indices: &[usize],
    len: usize,
    primary: usize,
    secondary: usize,
) -> (usize, usize) {
    let source = |offset: usize| {
        if primary != secondary && offset == primary.max(secondary) {
            indices[offset - 1] + 1
        } else {
            indices.get(offset).copied().unwrap_or(len)
        }
    };
    (source(primary), source(secondary))
}

/// A source selection with each end moved to its visible place
/// (`source_selection`), so no edit lands among hidden delimiters.
pub fn normalized_selection(text: &str, primary: usize, secondary: usize) -> (usize, usize) {
    let (_, indices) = projection(text);
    let visible = |source: usize| indices.partition_point(|index| *index < source);
    source_selection(
        &indices,
        text.chars().count(),
        visible(primary),
        visible(secondary),
    )
}

/// The grapheme boundary before (`backward`) or after scalar offset `offset`
/// of `text`, where moving and deleting by one character lands.
pub fn adjacent_grapheme(text: &str, offset: usize, backward: bool) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    let mut boundary = 0;
    let mut previous = 0;
    for grapheme in text.graphemes(true) {
        boundary += grapheme.chars().count();
        if backward && boundary >= offset {
            return previous;
        }
        if !backward && boundary > offset {
            return boundary;
        }
        previous = boundary;
    }
    boundary
}

pub use composer::{Edited, edit};

/// One composer edit without a style carried from the previous key
/// (`composer::edit` with `continuing` -1).
pub fn repair_edit(previous: &str, next: &str, start: usize, end: usize) -> (String, usize, usize) {
    let edited = edit(previous, next, start, end, -1);
    (edited.text, edited.start, edited.end)
}

/// A toolbar toggle of style bit `style` for the source selection
/// `start..end` (Unicode scalars), as web's toolbar makes it
/// (`composer::toggle`): the new text and selection, or the unchanged ones
/// when the selection cannot take that change.
pub fn toggle(text: &str, start: usize, end: usize, style: i32) -> (String, usize, usize) {
    composer::toggle(text, start, end, style).unwrap_or_else(|| (text.into(), start, end))
}

pub use composer::selection_styles;
