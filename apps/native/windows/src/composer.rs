//! The composer: a UTF-16 text buffer with caret/anchor selection, IME composition shown inline,
//! word movement, DirectWrite layout, hit testing and drawing.
//!
//! The message composers (`Composer::formatted`) are formatted, as web's,
//! desktop's and Linux's: the buffer holds the draft's Markdown source, the
//! layout shows it with its delimiters hidden and its runs bold and italic,
//! and the caret and selection keep to visible places. Each typed edit goes
//! through the shared `composer::edit` (`shared/messages/composer.rs`), so the
//! style web carries between keys follows `shared/messages/composer-cases.json`
//! and markers typed by hand stay as typed. Programmatic writes (drafts,
//! inserted emoji, dictation, UIA's SetValue) and an IME's committed text keep
//! the source they write; each of those, a placed caret and leaving the field
//! end the carried style. Arrows, Backspace and Delete step over visible
//! graphemes, Ctrl+B and Ctrl+I toggle a selection's style, and Copy and Cut
//! take the visible text. Other fields edit their text as typed.
use crate::gfx::{self, Gfx, fill_round, rect};
use caper_client::formatting::{self, normalized_selection, projection, source_runs};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::core::*;

pub const PAD: f32 = 12.0;
pub const LINE: f32 = 15.0 * 1.35;
pub const MAX_LINES: f32 = 6.0;
/// The longest draft a style toggle may leave, as desktop's.
const MAX_LENGTH: usize = 4000;

#[derive(Default)]
pub struct Composer {
    pub text: Vec<u16>,
    pub caret: usize,
    pub anchor: usize,
    /// IME composition string (not yet committed) inserted at the caret, and its cursor.
    pub comp: Vec<u16>,
    pub comp_cursor: usize,
    pub composing: bool,
    pub focused: bool,
    pub caret_on: bool,
    pub layout: Option<IDWriteTextLayout>,
    pub width: f32,
    pub text_h: f32,
    /// Room before the text inside the box, left and top (the main
    /// composer's Attach files button and staged files).
    pub inset: (f32, f32),
    /// Room under the text inside the box (the main composer's link card).
    pub inset_bottom: f32,
    /// What typing left in a formatted composer; `None` in plain fields.
    pub(crate) rich: Option<Typing>,
    /// A typed high surrogate waiting for its low half (formatted only).
    pub(crate) high: Option<u16>,
    pub(crate) undo: Vec<Snapshot>,
    pub(crate) redo: Vec<Snapshot>,
}

pub(crate) struct Snapshot {
    text: Vec<u16>,
    caret: usize,
    anchor: usize,
}

/// The two selection actions, shared by the channel and thread boxes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Bold,
    Italic,
}

impl Style {
    pub fn name(self) -> &'static str {
        match self {
            Self::Bold => "Bold",
            Self::Italic => "Italic",
        }
    }

    pub fn marker(self) -> &'static str {
        match self {
            Self::Bold => "**",
            Self::Italic => "_",
        }
    }
}

/// What typing left: the source, the caret the last key left, and the style
/// web carries to a key typed there next (`composer::edit`'s `pending`).
/// Offsets are Unicode scalars.
pub(crate) struct Typing {
    text: String,
    caret: Option<usize>,
    pending: i32,
}

impl Default for Typing {
    fn default() -> Self {
        Self {
            text: String::new(),
            caret: None,
            pending: -1,
        }
    }
}

impl Typing {
    /// A user's edit that left `next` with selection `insert..bound`: the
    /// source and selection the field shows instead.
    fn edit(&mut self, next: &str, insert: usize, bound: usize) -> (String, usize, usize) {
        let typed = next.chars().count().checked_sub(self.text.chars().count());
        let continuing = match (
            typed.and_then(|count| insert.checked_sub(count)),
            self.caret,
        ) {
            (Some(at), Some(caret)) if insert == bound && at == caret => self.pending,
            _ => -1,
        };
        let edited = formatting::edit(&self.text, next, insert, bound, continuing);
        let (insert, bound) = normalized_selection(&edited.text, edited.start, edited.end);
        self.pending = edited.pending;
        self.caret = (insert == bound).then_some(insert);
        self.text.clone_from(&edited.text);
        (edited.text, insert, bound)
    }

    /// Anything but typing left `text` with selection `insert..bound`: the
    /// next key takes the style around the caret.
    fn ended(&mut self, text: &str, insert: usize, bound: usize) {
        self.text = text.into();
        self.caret = (insert == bound).then_some(insert);
        self.pending = -1;
    }
}

/// A formatted source as shown: the visible text, each source offset's
/// visible offset, each visible offset's source caret (just before the next
/// visible character, or the source's end), and the bold and italic runs.
/// Offsets are UTF-16.
struct Shown {
    visible: Vec<u16>,
    to_visible: Vec<usize>,
    to_source: Vec<usize>,
    styles: Vec<(usize, usize, bool, bool)>,
}

fn shown(text: &[u16]) -> Shown {
    let source = String::from_utf16_lossy(text);
    let mut s = Shown {
        visible: Vec::with_capacity(text.len()),
        to_visible: Vec::with_capacity(text.len() + 1),
        to_source: Vec::with_capacity(text.len() + 1),
        styles: Vec::new(),
    };
    let mut at = 0;
    for run in source_runs(&source) {
        let start = s.visible.len();
        for c in source[run.range].chars() {
            let mut units = [0u16; 2];
            let units = c.encode_utf16(&mut units);
            for (k, unit) in units.iter().enumerate() {
                if run.hidden {
                    s.to_visible.push(s.visible.len());
                } else {
                    s.to_visible.push(s.visible.len());
                    s.to_source.push(at + k);
                    s.visible.push(*unit);
                }
            }
            at += units.len();
        }
        if !run.hidden && (run.bold || run.italic) {
            s.styles
                .push((start, s.visible.len() - start, run.bold, run.italic));
        }
    }
    s.to_visible.push(s.visible.len());
    s.to_source.push(at);
    s
}

/// Unicode scalars in `units`.
fn scalars(units: &[u16]) -> usize {
    char::decode_utf16(units.iter().copied()).count()
}

/// UTF-16 units in `text`'s first `count` scalars.
fn units(text: &str, count: usize) -> usize {
    text.chars().take(count).map(char::len_utf16).sum()
}

fn is_word(c: u16) -> bool {
    char::from_u32(c as u32).is_some_and(|c| c.is_alphanumeric() || c == '_')
        || (0xD800..0xE000).contains(&c)
}

/// The word boundary before (`backward`) or after visible scalar `at`, as the
/// plain fields' Ctrl+arrows step.
fn word_step(visible: &str, mut at: usize, backward: bool) -> usize {
    let word = |c: &char| c.is_alphanumeric() || *c == '_';
    let c: Vec<char> = visible.chars().collect();
    if backward {
        while at > 0 && !word(&c[at - 1]) {
            at -= 1;
        }
        while at > 0 && word(&c[at - 1]) {
            at -= 1;
        }
    } else {
        while at < c.len() && word(&c[at]) {
            at += 1;
        }
        while at < c.len() && !word(&c[at]) {
            at += 1;
        }
    }
    at
}

/// The source range Backspace (`backward`) or Delete removes at source
/// scalar `caret`: the visible grapheme or word beside it, or `None` when
/// there is none.
fn removal(text: &str, caret: usize, backward: bool, word: bool) -> Option<(usize, usize)> {
    let (visible, indices) = projection(text);
    let at = indices.partition_point(|index| *index < caret);
    let next = if word {
        word_step(&visible, at, backward)
    } else {
        formatting::adjacent_grapheme(&visible, at, backward)
    };
    (next != at).then(|| {
        let (a, b) = formatting::source_selection(&indices, text.chars().count(), next, at);
        (a.min(b), a.max(b))
    })
}

/// Where a formatted composer's caret goes.
#[derive(Clone, Copy)]
enum Go {
    Left,
    Right,
    WordLeft,
    WordRight,
    Home,
    End,
}

impl Composer {
    /// A message composer: formatted, as web's.
    pub fn formatted() -> Self {
        Self {
            rich: Some(Typing::default()),
            ..Self::default()
        }
    }

    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    /// The text as laid out: a formatted composer's visible text, and the
    /// IME's composition at the caret.
    pub fn display(&self) -> Vec<u16> {
        let (mut d, caret) = match self.rich {
            Some(_) => {
                let s = shown(&self.text);
                let caret = s.to_visible[self.caret.min(self.text.len())];
                (s.visible, caret)
            }
            None => (self.text.clone(), self.caret),
        };
        if !self.comp.is_empty() {
            d.splice(caret..caret, self.comp.iter().copied());
        }
        d
    }

    /// The text people read and select (UIA's document): a formatted
    /// composer's visible text.
    pub fn shown_text(&self) -> Vec<u16> {
        match self.rich {
            Some(_) => shown(&self.text).visible,
            None => self.text.clone(),
        }
    }

    /// `shown_text`'s offsets of the caret and the anchor.
    pub fn shown_ends(&self) -> (usize, usize) {
        match self.rich {
            Some(_) => {
                let s = shown(&self.text);
                let n = self.text.len();
                (
                    s.to_visible[self.caret.min(n)],
                    s.to_visible[self.anchor.min(n)],
                )
            }
            None => (self.caret, self.anchor),
        }
    }

    /// Selects `anchor..caret`, `shown_text` offsets (UIA's Select).
    pub fn select_shown(&mut self, anchor: usize, caret: usize) {
        if self.rich.is_some() {
            let s = shown(&self.text);
            let n = s.visible.len();
            self.anchor = s.to_source[anchor.min(n)];
            self.caret = s.to_source[caret.min(n)];
            self.placed();
        } else {
            self.anchor = anchor.min(self.text.len());
            self.caret = caret.min(self.text.len());
        }
    }

    /// The selected text as Copy takes it: a formatted composer's visible
    /// characters.
    pub fn copied(&self) -> Vec<u16> {
        let (a, b) = self.selection();
        match self.rich {
            Some(_) => {
                let s = shown(&self.text);
                s.visible[s.to_visible[a]..s.to_visible[b]].to_vec()
            }
            None => self.text[a..b].to_vec(),
        }
    }

    fn source(&self) -> String {
        String::from_utf16_lossy(&self.text)
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            caret: self.caret,
            anchor: self.anchor,
        }
    }

    fn remember(&mut self, before: Snapshot) {
        if self.rich.is_some() && before.text != self.text {
            if self.undo.len() == 128 {
                self.undo.remove(0);
            }
            self.undo.push(before);
            self.redo.clear();
        }
    }

    /// Undo restores source, never treating the restored delimiters as typing.
    fn undo(&mut self, redo: bool) {
        let saved = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        };
        if let Some(saved) = saved {
            let current = self.snapshot();
            if redo {
                self.undo.push(current);
            } else {
                self.redo.push(current);
            }
            self.text = saved.text;
            self.caret = saved.caret;
            self.anchor = saved.anchor;
            self.comp.clear();
        }
        self.end();
    }

    /// Source scalars before UTF-16 offset `at`.
    fn scalar(&self, at: usize) -> usize {
        scalars(&self.text[..at.min(self.text.len())])
    }

    /// Writes source `text` with its caret and anchor at scalars.
    fn set_source(&mut self, text: &str, caret: usize, anchor: usize) {
        self.text = text.encode_utf16().collect();
        self.caret = units(text, caret);
        self.anchor = units(text, anchor);
    }

    /// The selection moved to its visible places, and the carried style
    /// ended (a placed caret, even where it was).
    fn placed(&mut self) {
        self.high = None;
        let text = self.source();
        let (caret, anchor) =
            normalized_selection(&text, self.scalar(self.caret), self.scalar(self.anchor));
        self.caret = units(&text, caret);
        self.anchor = units(&text, anchor);
        if let Some(typing) = &mut self.rich {
            typing.ended(&text, caret, anchor);
        }
    }

    /// Ends the style carried to the next key, as when the field loses the
    /// focus or shows another conversation.
    pub fn end(&mut self) {
        if self.rich.is_some() {
            self.placed();
        }
    }

    /// A formatted composer's user edit: the source became `next` with the
    /// caret at scalar `caret`, repaired as web's composer would.
    fn typed(&mut self, next: &str, caret: usize) {
        let before = self.snapshot();
        let previous = self.source();
        let (at, anchor) = (self.scalar(self.caret), self.scalar(self.anchor));
        let Some(typing) = &mut self.rich else { return };
        // Anything else that wrote the source ended the carried style.
        if typing.text != previous {
            typing.ended(&previous, at, anchor);
        }
        let (text, insert, bound) = typing.edit(next, caret, caret);
        self.set_source(&text, insert, bound);
        self.remember(before);
    }

    fn delete_selection(&mut self) -> bool {
        let (a, b) = self.selection();
        if a == b {
            return false;
        }
        self.text.drain(a..b);
        self.caret = a;
        self.anchor = a;
        true
    }

    /// Typed or pasted text over the selection.
    pub fn insert(&mut self, s: &[u16]) {
        if self.rich.is_none() {
            self.delete_selection();
            self.text.splice(self.caret..self.caret, s.iter().copied());
            self.caret += s.len();
            self.anchor = self.caret;
            return;
        }
        // A character outside the BMP arrives as two WM_CHARs.
        let mut s = s.to_vec();
        if let Some(high) = self.high.take() {
            s.insert(0, high);
        }
        if let Some(&last) = s.last()
            && (0xD800..0xDC00).contains(&last)
        {
            self.high = s.pop();
        }
        let (a, b) = self.selection();
        // Wait for the entire character before replacing a selection. An
        // empty insertion without a pending surrogate is still Cut/delete.
        if s.is_empty() && (self.high.is_some() || a == b) {
            return;
        }
        let mut next = self.text[..a].to_vec();
        next.extend_from_slice(&s);
        let caret = scalars(&next);
        next.extend_from_slice(&self.text[b..]);
        self.typed(&String::from_utf16_lossy(&next), caret);
    }

    /// Text written over the selection as it is, not typed: an IME's
    /// commit, an inserted emoji or dictated words.
    pub fn commit(&mut self, s: &[u16]) {
        let before = self.snapshot();
        let rich = self.rich.take();
        self.insert(s);
        self.rich = rich;
        self.end();
        self.remember(before);
    }

    /// The source and selection replaced by `text`, caret at its end (a
    /// draft, a sent message, UIA's SetValue).
    pub fn set_text(&mut self, text: Vec<u16>) {
        self.text = text;
        self.caret = self.text.len();
        self.anchor = self.caret;
        self.comp.clear();
        self.composing = false;
        self.undo.clear();
        self.redo.clear();
        self.end();
    }

    /// Backspace (`backward`) or Delete: the selection, else the visible
    /// character or word beside the caret.
    fn remove(&mut self, backward: bool, word: bool) {
        self.high = None;
        if self.rich.is_none() {
            return if backward {
                self.backspace(word)
            } else {
                self.delete(word)
            };
        }
        let (a, b) = self.selection();
        let text = self.source();
        let range = if a != b {
            Some((self.scalar(a), self.scalar(b)))
        } else {
            removal(&text, self.scalar(self.caret), backward, word)
        };
        let Some((start, end)) = range else { return };
        let next: String = text
            .chars()
            .take(start)
            .chain(text.chars().skip(end))
            .collect();
        self.typed(&next, start);
    }

    pub fn backspace(&mut self, word: bool) {
        if self.rich.is_some() {
            return self.remove(true, word);
        }
        if self.delete_selection() || self.caret == 0 {
            return;
        }
        let to = if word {
            self.word_left(self.caret)
        } else {
            self.prev(self.caret)
        };
        self.text.drain(to..self.caret);
        self.caret = to;
        self.anchor = to;
    }

    pub fn delete(&mut self, word: bool) {
        if self.rich.is_some() {
            return self.remove(false, word);
        }
        if self.delete_selection() || self.caret >= self.text.len() {
            return;
        }
        let to = if word {
            self.word_right(self.caret)
        } else {
            self.next(self.caret)
        };
        self.text.drain(self.caret..to);
        self.anchor = self.caret;
    }

    pub fn prev(&self, i: usize) -> usize {
        if i == 0 {
            return 0;
        }
        let mut j = i - 1;
        if j > 0
            && (0xDC00..0xE000).contains(&self.text[j])
            && (0xD800..0xDC00).contains(&self.text[j - 1])
        {
            j -= 1;
        }
        j
    }

    pub fn next(&self, i: usize) -> usize {
        if i >= self.text.len() {
            return self.text.len();
        }
        if (0xD800..0xDC00).contains(&self.text[i]) && i + 1 < self.text.len() {
            i + 2
        } else {
            i + 1
        }
    }

    pub fn word_left(&self, mut i: usize) -> usize {
        while i > 0 && !is_word(self.text[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(self.text[i - 1]) {
            i -= 1;
        }
        i
    }

    pub fn word_right(&self, mut i: usize) -> usize {
        let n = self.text.len();
        while i < n && is_word(self.text[i]) {
            i += 1;
        }
        while i < n && !is_word(self.text[i]) {
            i += 1;
        }
        i
    }

    pub fn move_to(&mut self, i: usize, extend: bool) {
        self.caret = i.min(self.text.len());
        if !extend {
            self.anchor = self.caret;
        }
        self.end();
    }

    /// Moves a formatted composer's caret through its visible text, as
    /// GTK's and egui's composers do; `extend` keeps the anchor.
    fn go(&mut self, go: Go, extend: bool) {
        self.high = None;
        let text = self.source();
        let (visible, indices) = projection(&text);
        let place = |source: usize| indices.partition_point(|index| *index < source);
        let primary = place(self.scalar(self.caret));
        let secondary = place(self.scalar(self.anchor));
        let collapse = primary != secondary && !extend;
        let next = match go {
            Go::Left | Go::WordLeft if collapse => primary.min(secondary),
            Go::Right | Go::WordRight if collapse => primary.max(secondary),
            Go::Left => formatting::adjacent_grapheme(&visible, primary, true),
            Go::Right => formatting::adjacent_grapheme(&visible, primary, false),
            Go::WordLeft => word_step(&visible, primary, true),
            Go::WordRight => word_step(&visible, primary, false),
            Go::Home => 0,
            Go::End => visible.chars().count(),
        };
        let (caret, anchor) = formatting::source_selection(
            &indices,
            text.chars().count(),
            next,
            if extend { secondary } else { next },
        );
        self.caret = units(&text, caret);
        self.anchor = units(&text, anchor);
        if let Some(typing) = &mut self.rich {
            typing.ended(&text, caret, anchor);
        }
    }

    /// Bold (`**`) or italic (`_`) on, or off, for a formatted composer's
    /// selection. False when nothing changed.
    pub fn toggle(&mut self, marker: &str) -> bool {
        if self.rich.is_none() || self.composing || !self.comp.is_empty() {
            return false;
        }
        let before = self.snapshot();
        let current = self.source();
        let (a, b) = self.selection();
        let (next, start, end) =
            formatting::toggle(&current, self.scalar(a), self.scalar(b), marker);
        if next == current || next.chars().count() > MAX_LENGTH {
            return false;
        }
        let backward = self.caret < self.anchor;
        self.set_source(
            &next,
            if backward { start } else { end },
            if backward { end } else { start },
        );
        self.placed();
        self.remember(before);
        true
    }

    /// Room above the editor for a selection's Bold/Italic actions.
    pub fn format_height(&self) -> f32 {
        let (caret, anchor) = self.shown_ends();
        if self.rich.is_some()
            && self.focused
            && !self.composing
            && self.comp.is_empty()
            && caret != anchor
        {
            44.0
        } else {
            0.0
        }
    }

    pub fn format_rect(&self, style: Style) -> Option<(f32, f32, f32, f32)> {
        (self.format_height() > 0.0).then_some((
            PAD + if style == Style::Italic { 40.0 } else { 0.0 },
            PAD,
            36.0,
            36.0,
        ))
    }

    /// Lays out the visible text at `width` (inner width). Returns the box height in DIPs.
    pub fn relayout(&mut self, g: &Gfx, width: f32) -> Result<f32> {
        let text_h = self.relayout_with(g, &g.formats.composer, 15.0, width)?;
        Ok(text_h.min(LINE * MAX_LINES) + 2.0 * PAD)
    }

    /// Lays out the visible text in `format` (`size` DIPs) at `width`; returns its height.
    pub fn relayout_with(
        &mut self,
        g: &Gfx,
        format: &IDWriteTextFormat,
        size: f32,
        width: f32,
    ) -> Result<f32> {
        self.width = width;
        let d = self.display();
        let layout = g.layout(&d, format, width)?;
        crate::emoji::apply(&layout, &String::from_utf16_lossy(&d), size, 1.2)?;
        let caret = self.shown_ends().0;
        unsafe {
            if self.rich.is_some() {
                // Runs after the composition move past it.
                let comp = self.comp.len();
                let at = |p: usize| if comp > 0 && p >= caret { p + comp } else { p };
                for (start, len, bold, italic) in shown(&self.text).styles {
                    let range = DWRITE_TEXT_RANGE {
                        startPosition: at(start) as u32,
                        length: (at(start + len) - at(start)) as u32,
                    };
                    if bold {
                        layout.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, range)?;
                    }
                    if italic {
                        layout.SetFontStyle(DWRITE_FONT_STYLE_ITALIC, range)?;
                    }
                }
            }
            if !self.comp.is_empty() {
                layout.SetUnderline(
                    true,
                    DWRITE_TEXT_RANGE {
                        startPosition: caret as u32,
                        length: self.comp.len() as u32,
                    },
                )?;
            }
        }
        self.text_h = gfx::metrics(&layout).height.max(LINE);
        self.layout = Some(layout);
        Ok(self.text_h)
    }

    /// Editing keys every text field shares: caret movement, deletion,
    /// select all, cut and paste, and a formatted composer's Ctrl+B and
    /// Ctrl+I. Enter, Escape and copy belong to the caller. False when `vk`
    /// is not one of them.
    pub fn edit_key(
        &mut self,
        hwnd: windows::Win32::Foundation::HWND,
        vk: VIRTUAL_KEY,
        ctrl: bool,
        shift: bool,
        multiline: bool,
    ) -> bool {
        let (a, b) = self.selection();
        let rich = self.rich.is_some();
        if rich && (self.composing || !self.comp.is_empty()) {
            return false;
        }
        match vk {
            VK_LEFT | VK_RIGHT | VK_HOME | VK_END if rich => {
                let go = match (vk, ctrl) {
                    (VK_LEFT, false) => Go::Left,
                    (VK_LEFT, true) => Go::WordLeft,
                    (VK_RIGHT, false) => Go::Right,
                    (VK_RIGHT, true) => Go::WordRight,
                    (VK_HOME, _) => Go::Home,
                    _ => Go::End,
                };
                self.go(go, shift);
            }
            VK_LEFT => {
                let to = if !shift && a != b {
                    a
                } else if ctrl {
                    self.word_left(self.caret)
                } else {
                    self.prev(self.caret)
                };
                self.move_to(to, shift);
            }
            VK_RIGHT => {
                let to = if !shift && a != b {
                    b
                } else if ctrl {
                    self.word_right(self.caret)
                } else {
                    self.next(self.caret)
                };
                self.move_to(to, shift);
            }
            VK_HOME => self.move_to(0, shift),
            VK_END => self.move_to(self.text.len(), shift),
            VK_BACK => self.remove(true, ctrl),
            VK_DELETE => self.remove(false, ctrl),
            _ if ctrl && vk == VIRTUAL_KEY('A' as u16) => {
                self.anchor = 0;
                self.caret = self.text.len();
                self.end();
            }
            _ if ctrl && vk == VIRTUAL_KEY('X' as u16) => {
                crate::set_clipboard(hwnd, &self.copied());
                self.insert(&[]);
            }
            _ if ctrl && vk == VIRTUAL_KEY('V' as u16) => {
                // A single-line field takes a pasted line break as a space.
                let t: Vec<u16> = crate::get_clipboard(hwnd)
                    .into_iter()
                    .filter(|c| *c != '\r' as u16)
                    .map(|c| {
                        if !multiline && c == '\n' as u16 {
                            ' ' as u16
                        } else {
                            c
                        }
                    })
                    .collect();
                self.insert(&t);
            }
            _ if rich && ctrl && vk == VIRTUAL_KEY('B' as u16) => {
                self.toggle("**");
            }
            _ if rich && ctrl && vk == VIRTUAL_KEY('I' as u16) => {
                self.toggle("_");
            }
            _ if rich && ctrl && vk == VIRTUAL_KEY('Z' as u16) => self.undo(shift),
            _ if rich && ctrl && vk == VIRTUAL_KEY('Y' as u16) => self.undo(true),
            _ => return false,
        }
        true
    }

    /// Draws the selection, text (or `placeholder` when empty) and caret
    /// with the layout origin at (ox, oy). The caller clips.
    pub fn draw_text(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        ox: f32,
        oy: f32,
        placeholder: Option<(&IDWriteTextLayout, &ID2D1SolidColorBrush)>,
    ) {
        unsafe {
            if self.text.is_empty() && self.comp.is_empty() {
                if let Some((l, brush)) = placeholder {
                    crate::text::draw(dc, l, ox, oy, brush);
                }
            } else if let Some(l) = &self.layout {
                let (caret, anchor) = self.shown_ends();
                let (a, b) = (caret.min(anchor), caret.max(anchor));
                if b > a && self.comp.is_empty() {
                    let mut buf = [DWRITE_HIT_TEST_METRICS::default(); 16];
                    let mut n = 0u32;
                    if l.HitTestTextRange(a as u32, (b - a) as u32, ox, oy, Some(&mut buf), &mut n)
                        .is_ok()
                    {
                        for m in &buf[..(n as usize).min(16)] {
                            dc.FillRectangle(
                                &rect(m.left, m.top, m.left + m.width, m.top + m.height),
                                &g.brushes.selection,
                            );
                        }
                    }
                }
                crate::text::draw(dc, l, ox, oy, &g.brushes.text);
            }
            if self.focused && self.caret_on {
                let (x, y, ch) = self.caret_pos(self.display_caret());
                dc.FillRectangle(
                    &rect(ox + x, oy + y, ox + x + 1.5, oy + y + ch),
                    &g.brushes.text,
                );
            }
        }
    }

    /// Display-string position of the caret (inside the composition while composing).
    pub fn display_caret(&self) -> usize {
        self.shown_ends().0
            + if self.comp.is_empty() {
                0
            } else {
                self.comp_cursor
            }
    }

    /// Caret rectangle (x, y, height) in DIPs relative to the text origin.
    pub fn caret_pos(&self, pos: usize) -> (f32, f32, f32) {
        let Some(l) = &self.layout else {
            return (0.0, 0.0, LINE);
        };
        let (mut x, mut y) = (0.0, 0.0);
        let mut m = DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            let _ = l.HitTestTextPosition(pos as u32, false, &mut x, &mut y, &mut m);
        }
        (x, m.top, m.height.max(LINE))
    }

    /// Scroll offset so the caret stays visible when the text exceeds MAX_LINES.
    pub fn scroll(&self) -> f32 {
        let max = LINE * MAX_LINES;
        if self.text_h <= max {
            return 0.0;
        }
        let (_, y, h) = self.caret_pos(self.display_caret());
        (y + h - max).clamp(0.0, self.text_h - max)
    }

    /// The text position (a source offset) at layout point (x, y).
    pub fn hit(&self, x: f32, y: f32) -> usize {
        let Some(l) = &self.layout else { return 0 };
        let mut trailing = BOOL(0);
        let mut inside = BOOL(0);
        let mut m = DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            let _ = l.HitTestPoint(x, y + self.scroll(), &mut trailing, &mut inside, &mut m);
        }
        let mut p = m.textPosition as usize
            + if trailing.as_bool() {
                m.length as usize
            } else {
                0
            };
        let caret = self.shown_ends().0;
        if !self.comp.is_empty() && p > caret {
            p = p.saturating_sub(self.comp.len()).max(caret);
        }
        match self.rich {
            Some(_) => {
                let s = shown(&self.text);
                s.to_source[p.min(s.visible.len())]
            }
            None => p.min(self.text.len()),
        }
    }

    /// Draws the whole composer box (DIPs, origin at the box's top-left).
    pub fn draw(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        w: f32,
        h: f32,
        placeholder: &str,
    ) -> Result<()> {
        unsafe {
            dc.Clear(Some(&gfx::color(crate::theme::SURFACE_CONVERSATION, 1.0)));
            fill_round(
                dc,
                rect(0.0, 0.0, w, h),
                crate::theme::RADIUS,
                &g.brushes.composer,
            );
            for style in [Style::Bold, Style::Italic] {
                if let Some((x, y, w, h)) = self.format_rect(style) {
                    fill_round(
                        dc,
                        rect(x, y, x + w, y + h),
                        crate::theme::RADIUS,
                        &g.brushes.hover,
                    );
                    let label = if style == Style::Bold { "B" } else { "I" };
                    let l = g.layout(&gfx::wide(label), &g.formats.composer, w)?;
                    let range = DWRITE_TEXT_RANGE {
                        startPosition: 0,
                        length: 1,
                    };
                    if style == Style::Bold {
                        l.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, range)?;
                    } else {
                        l.SetFontStyle(DWRITE_FONT_STYLE_ITALIC, range)?;
                    }
                    let m = gfx::metrics(&l);
                    crate::text::draw(
                        dc,
                        &l,
                        x + (w - m.width) / 2.0,
                        y + (h - m.height) / 2.0,
                        &g.brushes.text,
                    );
                }
            }
            let (left, top) = self.inset;
            dc.PushAxisAlignedClip(
                &rect(PAD + left, PAD + top, w - PAD, h - PAD - self.inset_bottom),
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
            let (ox, oy) = (PAD + left, PAD + top - self.scroll());
            let hint = if self.text.is_empty() && self.comp.is_empty() {
                Some(g.layout(&gfx::wide(placeholder), &g.formats.composer, self.width)?)
            } else {
                None
            };
            self.draw_text(g, dc, ox, oy, hint.as_ref().map(|l| (l, &g.brushes.muted)));
            dc.PopAxisAlignedClip();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn source(c: &Composer) -> String {
        String::from_utf16(&c.text).unwrap()
    }

    /// Plain fields edit raw text, so markers typed or pasted by hand stay
    /// exactly as written, with the caret after the last key. (The shared
    /// `formatting::edit` repair assumes hidden delimiters and must not run
    /// on this buffer.)
    #[test]
    fn typed_and_pasted_markdown_stays_as_written() {
        for input in [
            "**bold**",
            "**bold**!",
            "*a* and _b_",
            "_under_score_",
            "`code` ``",
            "a ** b",
            "~~s~~",
        ] {
            let mut c = Composer::default();
            for ch in input.chars() {
                c.insert(&ch.to_string().encode_utf16().collect::<Vec<_>>());
            }
            let typed = (String::from_utf16(&c.text).unwrap(), c.caret);
            assert_eq!(typed, (input.to_owned(), input.len()), "typed {input:?}");
            let mut c = Composer::default();
            c.insert(&input.encode_utf16().collect::<Vec<_>>());
            let pasted = (String::from_utf16(&c.text).unwrap(), c.caret);
            assert_eq!(pasted, (input.to_owned(), input.len()), "pasted {input:?}");
        }
        // Backspace removes one character, even a closing marker's.
        let mut c = Composer::default();
        c.insert(&"**bold**".encode_utf16().collect::<Vec<_>>());
        c.backspace(false);
        assert_eq!(String::from_utf16(&c.text).unwrap(), "**bold*");
    }

    /// The message composers type the shared steps generated from web, one
    /// WM_CHAR at a time, with their own caret mapping. A placed caret (a
    /// click, even where it already is) ends the style the last key carried.
    #[test]
    fn typing_follows_the_shared_cases() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../shared/messages/composer-cases.json"
        ))
        .unwrap();
        let cases = cases["cases"].as_array().unwrap();
        assert!(cases.len() > 20);
        for case in cases {
            let note = case["note"].as_str().unwrap();
            let mut c = Composer::formatted();
            c.set_text(wide(case["text"].as_str().unwrap()));
            // Web counts visible UTF-16 offsets, as `shown_text` does.
            let at = case["caret"].as_u64().unwrap() as usize;
            c.select_shown(at, at);
            for step in case["steps"].as_array().unwrap() {
                if step["placed"].as_bool().unwrap() {
                    c.move_to(c.caret, false);
                }
                for unit in wide(step["key"].as_str().unwrap()) {
                    c.insert(&[unit]);
                }
                assert_eq!(c.caret, c.anchor, "{note}");
                assert_eq!(
                    (
                        source(&c),
                        c.shown_ends().0,
                        c.rich.as_ref().unwrap().pending
                    ),
                    (
                        step["text"].as_str().unwrap().to_owned(),
                        step["caret"].as_u64().unwrap() as usize,
                        step["pending"].as_i64().unwrap() as i32
                    ),
                    "{note}"
                );
            }
        }
    }

    /// Markers typed by hand stay as typed: a pair that closes becomes its
    /// style, and markers that close nothing stay plain text.
    #[test]
    fn hand_typed_markers_keep_their_source() {
        for (typed, shown) in [
            ("**bold**", "bold"),
            ("_it_", "it"),
            ("snake_case_name", "snake_case_name"),
            ("2*3*4", "2*3*4"),
            ("a ** b", "a ** b"),
            ("a *b", "a *b"),
        ] {
            let mut c = Composer::formatted();
            for unit in wide(typed) {
                c.insert(&[unit]);
            }
            assert_eq!(
                (source(&c), String::from_utf16(&c.shown_text()).unwrap()),
                (typed.to_owned(), shown.to_owned())
            );
            assert_eq!(c.display(), c.shown_text());
            let mut pasted = Composer::formatted();
            pasted.insert(&wide(typed));
            assert_eq!(source(&pasted), typed, "pasted Markdown remains source");
            assert_eq!(pasted.shown_text(), wide(shown));
        }
    }

    #[test]
    fn keys_step_over_visible_graphemes_and_copy_takes_visible_text() {
        // a, the three-scalar 👩‍💻 and b, inside bold's markers.
        let mut c = Composer::formatted();
        c.set_text(wide("**a\u{1F469}\u{200D}\u{1F4BB}b**"));
        assert_eq!(c.shown_ends().0, 7);
        let left = |c: &mut Composer, shift| c.go(Go::Left, shift);
        left(&mut c, false);
        assert_eq!(c.shown_ends().0, 6, "before b");
        left(&mut c, false);
        assert_eq!(c.shown_ends().0, 1, "over the whole emoji");
        // Backspace removes a, and what remains keeps its style.
        c.remove(true, false);
        assert_eq!(source(&c), "**\u{1F469}\u{200D}\u{1F4BB}b**");
        c.remove(false, false);
        assert_eq!(source(&c), "**b**");
        assert_eq!(c.caret, 2);
        // Nothing visible before the caret: Backspace leaves the markers.
        c.remove(true, false);
        assert_eq!(source(&c), "**b**");
        // A selection's copy, and Cut's, take the visible text.
        let mut c = Composer::formatted();
        c.set_text(wide("keep **this**"));
        c.go(Go::Home, false);
        c.go(Go::End, true);
        assert_eq!(String::from_utf16(&c.copied()).unwrap(), "keep this");
        // A click among hidden markers lands at the visible place.
        c.move_to(6, false);
        assert_eq!((c.caret, c.shown_ends().0), (7, 5));
    }

    #[test]
    fn styles_toggle_on_a_selection_and_writes_are_kept_as_written() {
        let mut c = Composer::formatted();
        for unit in wide("make it bold") {
            c.insert(&[unit]);
        }
        c.select_shown(8, 12);
        assert!(c.toggle("**"));
        assert_eq!(source(&c), "make it **bold**");
        assert_eq!(String::from_utf16(&c.copied()).unwrap(), "bold");
        assert!(c.toggle("**"));
        assert_eq!(source(&c), "make it bold");
        // Plain fields have no styles.
        let mut plain = Composer::default();
        plain.insert(&wide("x"));
        plain.anchor = 0;
        assert!(!plain.toggle("**"));
        // An IME's commit and an inserted emoji keep their source.
        let mut c = Composer::formatted();
        c.commit(&wide("**"));
        assert_eq!((source(&c), c.caret), ("**".to_owned(), 2));
    }

    #[test]
    fn backward_selection_and_undo_restore_source_without_repair() {
        let mut c = Composer::formatted();
        c.set_text(wide("left word right"));
        c.select_shown(9, 5);
        assert!(c.toggle("**"));
        assert_eq!(source(&c), "left **word** right");
        assert_eq!(c.shown_ends(), (5, 9));
        assert_eq!(c.copied(), wide("word"));
        let hwnd = windows::Win32::Foundation::HWND::default();
        assert!(c.edit_key(hwnd, VIRTUAL_KEY('Z' as u16), true, false, true));
        assert_eq!(source(&c), "left word right");
        assert_eq!(c.shown_ends(), (5, 9));
        assert!(c.edit_key(hwnd, VIRTUAL_KEY('Y' as u16), true, false, true));
        assert_eq!(source(&c), "left **word** right");
        assert_eq!(c.shown_ends(), (5, 9));
        c.commit(&wide("_new_"));
        assert_eq!(source(&c), "left **_new_** right");
        assert!(c.edit_key(hwnd, VIRTUAL_KEY('Z' as u16), true, false, true));
        assert_eq!(source(&c), "left **word** right");
        // Opening another draft must not allow Undo to resurrect this one.
        c.set_text(wide("next draft"));
        c.edit_key(hwnd, VIRTUAL_KEY('Z' as u16), true, false, true);
        assert_eq!(source(&c), "next draft");
    }

    #[test]
    fn external_writes_and_focus_changes_end_carried_style() {
        for commit in [false, true] {
            let mut c = Composer::formatted();
            c.set_text(wide("**word**"));
            c.insert(&wide(" "));
            assert_eq!(source(&c), "**word** ");
            if commit {
                c.commit(&wide("raw "));
            } else {
                c.end();
            }
            c.insert(&wide("x"));
            assert_eq!(
                source(&c),
                if commit {
                    "**word** raw x"
                } else {
                    "**word** x"
                }
            );
        }
    }

    #[test]
    fn surrogate_pair_replaces_a_selection_only_when_complete() {
        let mut c = Composer::formatted();
        c.set_text(wide("keep **old** here"));
        c.select_shown(5, 8);
        c.insert(&[0xD83D]);
        assert_eq!(source(&c), "keep **old** here");
        assert_eq!(String::from_utf16(&c.copied()).unwrap(), "old");
        c.insert(&[0xDE42]);
        assert_eq!(source(&c), "keep **\u{1F642}** here");
        assert_eq!(c.shown_ends(), (7, 7));

        // A draft replacement or caret placement cannot carry half a
        // character into the next edit, even if the source is unchanged.
        c.insert(&[0xD83D]);
        c.set_text(wide("next"));
        c.insert(&wide("a"));
        assert_eq!(source(&c), "nexta");
        c.insert(&[0xD83D]);
        c.move_to(0, false);
        c.insert(&wide("b"));
        assert_eq!(source(&c), "bnexta");
    }
}
