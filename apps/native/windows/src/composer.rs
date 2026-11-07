//! The composer: a UTF-16 text buffer with caret/anchor selection, IME composition shown inline,
//! word movement, DirectWrite layout, hit testing and drawing.
use crate::gfx::{self, Gfx, fill_round, rect};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::core::*;

pub const PAD: f32 = 12.0;
pub const LINE: f32 = 15.0 * 1.35;
pub const MAX_LINES: f32 = 6.0;

#[derive(Default)]
pub struct Composer {
    pub text: Vec<u16>,
    pub caret: usize,
    pub anchor: usize,
    /// IME composition string (not yet committed) inserted at the caret, and its cursor.
    pub comp: Vec<u16>,
    pub comp_cursor: usize,
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
}

fn is_word(c: u16) -> bool {
    char::from_u32(c as u32).is_some_and(|c| c.is_alphanumeric() || c == '_')
        || (0xD800..0xE000).contains(&c)
}

impl Composer {
    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    pub fn display(&self) -> Vec<u16> {
        if self.comp.is_empty() {
            return self.text.clone();
        }
        let mut d = self.text[..self.caret].to_vec();
        d.extend_from_slice(&self.comp);
        d.extend_from_slice(&self.text[self.caret..]);
        d
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

    pub fn insert(&mut self, s: &[u16]) {
        self.delete_selection();
        self.text.splice(self.caret..self.caret, s.iter().copied());
        self.caret += s.len();
        self.anchor = self.caret;
    }

    pub fn backspace(&mut self, word: bool) {
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
        unsafe {
            if !self.comp.is_empty() {
                layout.SetUnderline(
                    true,
                    DWRITE_TEXT_RANGE {
                        startPosition: self.caret as u32,
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
    /// select all, cut and paste. Enter, Escape and copy belong to the
    /// caller. False when `vk` is not one of them.
    pub fn edit_key(
        &mut self,
        hwnd: windows::Win32::Foundation::HWND,
        vk: VIRTUAL_KEY,
        ctrl: bool,
        shift: bool,
        multiline: bool,
    ) -> bool {
        let (a, b) = self.selection();
        match vk {
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
            VK_BACK => self.backspace(ctrl),
            VK_DELETE => self.delete(ctrl),
            _ if ctrl && vk == VIRTUAL_KEY('A' as u16) => {
                self.anchor = 0;
                self.caret = self.text.len();
            }
            _ if ctrl && vk == VIRTUAL_KEY('X' as u16) => {
                crate::set_clipboard(hwnd, &self.text[a..b]);
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
                let (a, b) = self.selection();
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
        self.caret
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
        if !self.comp.is_empty() && p > self.caret {
            p = p.saturating_sub(self.comp.len()).max(self.caret);
        }
        p.min(self.text.len())
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

    /// The composer edits raw Markdown, so markers typed or pasted by hand
    /// stay exactly as written, with the caret after the last key. (The
    /// shared `formatting::edit` repair assumes hidden delimiters and must
    /// not run on this buffer.)
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
}
