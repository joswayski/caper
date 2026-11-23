//! Member search, as egui's: the member list's heading has Search members,
//! which opens a field under it. The list then shows only members whose
//! display name or username contains the trimmed query, ignoring case, or
//! "No members match your search." Clear and close, or Escape, empties
//! and closes it; it also closes when the space changes or the list hides.
use crate::composer::{Composer, LINE};
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::overlay::Rect;
use crate::theme::*;
use crate::{App, EV_FOCUS, EV_SCREEN, EV_TEXT, TIMER_CARET};
use windows::Win32::Foundation::{LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Result;

/// The field's row under the heading, which moves the list down.
pub const ROW_H: f32 = 44.0;
const FIELD_Y: f32 = HEADER_H + 6.0;
const FIELD_H: f32 = 32.0;
const BUTTON: f32 = 28.0;
pub const PLACEHOLDER: &str = "Search members";
pub const NO_MATCH: &str = "No members match your search.";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    /// Search members in the heading, while the search is closed.
    Open,
    Field,
    Close,
}

pub struct MemberSearch {
    pub field: Composer,
    hover: Option<Part>,
    /// The space it was opened in.
    space: Option<String>,
}

/// Whether a member shows for `query`, as egui filters its roster.
pub fn matches(query: &str, name: &str, username: Option<&str>) -> bool {
    let query = query.trim().to_lowercase();
    name.to_lowercase().contains(&query)
        || username.is_some_and(|u| u.to_lowercase().contains(&query))
}

fn contains((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

impl App {
    /// The member column's left edge (client DIPs).
    pub fn members_x(&self) -> f32 {
        self.w - self.member_width() - if self.is_narrow() { 8.0 } else { 0.0 }
    }

    /// Search members in the heading while the search is closed.
    pub fn member_search_button_rect(&self) -> Option<Rect> {
        (self.members_w() > 0.0 && self.c.is_live() && self.member_search.is_none()).then(|| {
            (
                self.members_x() + self.member_width()
                    - 16.0
                    - BUTTON
                    - if self.members_tiled() { 0.0 } else { 36.0 },
                self.members_y() + (HEADER_H - BUTTON) / 2.0,
                BUTTON,
                BUTTON,
            )
        })
    }

    pub fn member_search_rect(&self, p: Part) -> Option<Rect> {
        if p == Part::Open {
            return self.member_search_button_rect();
        }
        if self.member_search.is_none() || self.members_w() <= 0.0 {
            return None;
        }
        let x = self.members_x();
        Some(match p {
            Part::Open => return None,
            Part::Field => (
                x + 8.0,
                self.members_y() + FIELD_Y,
                self.member_width() - 16.0 - BUTTON - 4.0,
                FIELD_H,
            ),
            Part::Close => (
                x + self.member_width() - 8.0 - BUTTON,
                self.members_y() + FIELD_Y + (FIELD_H - BUTTON) / 2.0,
                BUTTON,
                BUTTON,
            ),
        })
    }

    fn member_search_hit(&self, x: f32, y: f32) -> Option<Part> {
        [Part::Open, Part::Field, Part::Close]
            .into_iter()
            .find(|p| {
                self.member_search_rect(*p)
                    .is_some_and(|r| contains(r, x, y))
            })
    }

    pub fn member_search_text_origin(&self) -> (f32, f32) {
        let (x, y, _, h) = self.member_search_rect(Part::Field).unwrap_or_default();
        (x + 10.0, y + (h - LINE) / 2.0)
    }

    /// How far the list moves down for the search field.
    pub fn members_top(&self) -> f32 {
        if self.member_search.is_some() {
            ROW_H
        } else {
            0.0
        }
    }

    /// The listed members that show, in order (indices into `c.members`).
    pub fn shown_members(&self) -> Vec<usize> {
        let all = 0..self.c.listed_members;
        let Some(s) = &self.member_search else {
            return all.collect();
        };
        let query = String::from_utf16_lossy(&s.field.text);
        if query.trim().is_empty() {
            return all.collect();
        }
        let usernames = self
            .live
            .as_ref()
            .map(|l| l.member_usernames())
            .unwrap_or_default();
        all.filter(|&i| {
            let username = self
                .ids
                .members
                .get(i)
                .and_then(|id| usernames.get(id.as_str()).copied());
            matches(&query, &self.c.members[i].name, username)
        })
        .collect()
    }

    pub fn open_member_search(&mut self) {
        if self.member_search.is_some() || self.members_w() <= 0.0 {
            return;
        }
        self.member_search = Some(MemberSearch {
            field: Composer {
                width: self.member_width() - 16.0 - BUTTON - 4.0 - 20.0,
                ..Default::default()
            },
            hover: None,
            space: self.ids.spaces.get(self.c.space).cloned(),
        });
        self.hover_member = None;
        self.set_member_search_focus(true);
        self.member_search_changed(true);
    }

    pub fn close_member_search(&mut self) {
        if self.member_search.is_none() {
            return;
        }
        self.set_member_search_focus(false);
        self.member_search = None;
        self.hover_member = None;
        self.dirty.members = true;
        self.events |= EV_SCREEN;
        self.need = true;
    }

    /// Clear and close and Escape give the keyboard back to the composer,
    /// or to the narrow drawer's Close while the drawer covers it.
    fn close_member_search_refocus(&mut self) {
        self.close_member_search();
        if self.members_w() > 0.0 && !self.members_tiled() {
            self.focus_narrow(crate::narrow::Part::CloseMembers);
        } else {
            self.set_composer_focus(true);
        }
    }

    /// After the space or the list's visibility changed.
    pub fn member_search_follow(&mut self) {
        let space = self.ids.spaces.get(self.c.space);
        if self
            .member_search
            .as_ref()
            .is_some_and(|s| self.members_w() <= 0.0 || s.space.as_ref() != space)
        {
            self.close_member_search();
        }
    }

    pub fn set_member_search_focus(&mut self, focused: bool) {
        let hwnd = self.hwnd;
        let Some(s) = &mut self.member_search else {
            return;
        };
        if s.field.focused == focused {
            return;
        }
        s.field.focused = focused;
        s.field.caret_on = true;
        if focused {
            self.set_composer_focus(false);
            self.set_thread_focus(false);
        } else if let Some(s) = &mut self.member_search {
            s.field.end();
        }
        unsafe {
            if focused {
                SetTimer(Some(hwnd), TIMER_CARET, GetCaretBlinkTime(), None);
            } else {
                let _ = KillTimer(Some(hwnd), TIMER_CARET);
            }
        }
        self.dirty.members = true;
        self.events |= EV_FOCUS;
        self.need = true;
    }

    /// The field's text or caret changed: laid out again, and the list
    /// filtered again.
    pub fn member_search_changed(&mut self, text: bool) {
        let g = &self.g;
        if let Some(s) = &mut self.member_search {
            s.field.caret_on = true;
            let w = s.field.width;
            if let Err(e) = s.field.relayout_with(g, &g.formats.composer, 15.0, w) {
                eprintln!("member search layout failed: {e:?}");
            }
        }
        if text {
            self.hover_member = None;
            self.events |= EV_TEXT | EV_SCREEN;
        }
        self.dirty.members = true;
        self.need = true;
    }

    pub fn member_search_activate(&mut self, p: Part) {
        match p {
            Part::Open => self.open_member_search(),
            Part::Field => self.set_member_search_focus(true),
            Part::Close => self.close_member_search_refocus(),
        }
    }

    pub fn member_search_name(p: Part) -> &'static str {
        match p {
            Part::Open | Part::Field => PLACEHOLDER,
            Part::Close => "Clear and close member search",
        }
    }

    fn set_member_search_hover(&mut self, button: bool, hover: Option<Part>) {
        let mut changed = button != self.member_search_hover;
        self.member_search_hover = button;
        if let Some(s) = &mut self.member_search
            && s.hover != hover
        {
            s.hover = hover;
            changed = true;
        }
        if changed {
            self.dirty.members = true;
            self.need = true;
        }
    }

    /// The heading's button, the field and its Clear and close; other
    /// input passes on.
    pub fn member_search_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    ) -> Option<Option<LRESULT>> {
        let done = Some(Some(LRESULT(0)));
        let focused = self.member_search.as_ref().is_some_and(|s| s.field.focused);
        match msg {
            WM_MOUSEMOVE => {
                let hover = self.member_search_hit(x, y);
                self.set_member_search_hover(hover == Some(Part::Open), hover);
                None
            }
            WM_LBUTTONDOWN => {
                match self.member_search_hit(x, y) {
                    Some(Part::Open) => {
                        self.open_member_search();
                        done
                    }
                    Some(Part::Close) => {
                        self.close_member_search_refocus();
                        done
                    }
                    Some(Part::Field) => {
                        let (ox, oy) = self.member_search_text_origin();
                        self.set_member_search_focus(true);
                        let s = self.member_search.as_mut()?;
                        let pos = s.field.hit(x - ox, y - oy);
                        s.field.move_to(pos, shift);
                        self.member_search_changed(false);
                        done
                    }
                    None => {
                        // Somewhere else takes the click and the keys.
                        self.set_member_search_focus(false);
                        None
                    }
                }
            }
            WM_LBUTTONDBLCLK if self.member_search_hit(x, y).is_some() => done,
            WM_SETCURSOR => {
                let (px, py) = self.pointer?;
                let cursor = match self.member_search_hit(px, py)? {
                    Part::Field => IDC_IBEAM,
                    Part::Open | Part::Close => IDC_HAND,
                };
                unsafe {
                    SetCursor(LoadCursorW(None, cursor).ok());
                }
                Some(Some(LRESULT(1)))
            }
            WM_KEYDOWN if focused => {
                let hwnd = self.hwnd;
                let vk = VIRTUAL_KEY(wp.0 as u16);
                let s = self.member_search.as_mut()?;
                if vk == VK_ESCAPE {
                    self.close_member_search_refocus();
                } else if ctrl && vk == VIRTUAL_KEY('C' as u16) {
                    let (a, b) = s.field.selection();
                    if b > a {
                        crate::set_clipboard(hwnd, &s.field.text[a..b]);
                    }
                } else if s.field.edit_key(hwnd, vk, ctrl, shift, false) {
                    self.member_search_changed(true);
                }
                done
            }
            WM_CHAR if focused => {
                let c = wp.0 as u16;
                if !ctrl && c >= 0x20 && c != 0x7f {
                    let s = self.member_search.as_mut()?;
                    s.field.insert(&[c]);
                    self.member_search_changed(true);
                }
                done
            }
            _ => None,
        }
    }
}

/// Draws the heading's button or the field row on the member column's
/// surface (`x0` is the column's client left edge).
pub fn draw(g: &Gfx, dc: &ID2D1DeviceContext, a: &App, x0: f32) -> Result<()> {
    let y0 = a.members_y();
    let local = |(x, y, w, h): Rect| rect(x - x0, y - y0, x - x0 + w, y - y0 + h);
    if let Some(r) = a.member_search_button_rect() {
        let r = local(r);
        let lit = a.member_search_hover;
        if lit {
            fill_round(dc, r, 6.0, &g.brushes.hover);
        }
        let icon = rect(r.left + 6.0, r.top + 6.0, r.right - 6.0, r.bottom - 6.0);
        icons::draw(
            dc,
            Icon::Search,
            icon,
            if lit {
                crate::theme::text()
            } else {
                crate::theme::text_muted()
            },
            1.0,
        );
    }
    let Some(s) = &a.member_search else {
        return Ok(());
    };
    if let Some(r) = a.member_search_rect(Part::Field) {
        let f = local(r);
        fill_round(dc, f, RADIUS, &g.brushes.composer);
        let stroke = if s.field.focused {
            &g.brushes.terracotta
        } else if s.hover == Some(Part::Field) {
            &g.brushes.terracotta_bright
        } else {
            &g.brushes.border
        };
        stroke_round(dc, f, RADIUS, stroke);
        unsafe {
            dc.PushAxisAlignedClip(
                &rect(f.left + 2.0, f.top + 1.0, f.right - 2.0, f.bottom - 1.0),
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
        }
        let hint = g.layout(&gfx::wide(PLACEHOLDER), &g.formats.composer, 1e6)?;
        let (ox, oy) = a.member_search_text_origin();
        s.field
            .draw_text(g, dc, ox - x0, oy - y0, Some((&hint, &g.brushes.hint)));
        unsafe { dc.PopAxisAlignedClip() };
    }
    if let Some(r) = a.member_search_rect(Part::Close) {
        let r = local(r);
        let lit = s.hover == Some(Part::Close);
        if lit {
            fill_round(dc, r, 6.0, &g.brushes.hover);
        }
        let icon = rect(r.left + 6.0, r.top + 6.0, r.right - 6.0, r.bottom - 6.0);
        icons::draw(
            dc,
            Icon::Close,
            icon,
            if lit {
                crate::theme::text()
            } else {
                crate::theme::text_muted()
            },
            1.0,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn matches_name_or_username_trimmed_and_ignoring_case() {
        // egui's cases: a padded, differently cased username and name.
        assert!(matches("  MEMBER_29  ", "Member 29", Some("member_29")));
        assert!(matches("  mEmBeR 29  ", "Member 29", Some("m29")));
        assert!(!matches("no such member", "Member 29", Some("member_29")));
        // A username alone matches, and a missing one never does.
        assert!(matches("ada", "Countess Lovelace", Some("ada")));
        assert!(!matches("ada", "Countess Lovelace", None));
        assert!(matches("   ", "Anyone", None));
    }
}
