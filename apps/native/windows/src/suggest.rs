//! A composer's `:` emoji and `@` mention suggestions, as on the egui
//! desktop: one popup above the focused composer (the conversation's, the
//! thread's reply box or a profile card's). Mentions span the composer;
//! emoji start at its left edge, at most 260 DIPs wide. Arrows move the
//! highlight, Enter or Tab accepts it without sending, Escape dismisses it
//! for that token, and a click accepts a row. The grammar and ranking live
//! in `caper_client` (`emoji::token`, `mentions::token` and their
//! `suggestions` and `insert`).
use crate::avatars::{self, Avatar};
use crate::composer::Composer;
use crate::gfx::{fill_round, rect, stroke_round};
use crate::overlay::{Rect, height, layout};
use crate::theme::*;
use crate::{App, EV_SCREEN};
use caper_client::emoji::{self, Entry};
use caper_client::mentions::{self, Candidate};
use windows::Win32::Foundation::{LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Result;

const ROW_H: f32 = 44.0;
const PAD: f32 = 4.0;
const EMOJI_W: f32 = 260.0;
/// The highlighted row, as in the people lists.
const SELECTED: u32 = 0x43241E;

/// Which composer the popup belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Main,
    Thread,
    Profile,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Emoji(emoji::Token),
    Mention(mentions::Token),
}

impl Token {
    fn at(text: &str, caret: usize) -> Option<Self> {
        emoji::token(text, caret)
            .map(Self::Emoji)
            .or_else(|| mentions::token(text, caret).map(Self::Mention))
    }
}

#[derive(Clone)]
pub enum Choice {
    Emoji(&'static Entry),
    Mention(Candidate),
}

pub struct Suggest {
    pub field: Field,
    token: Token,
    pub choices: Vec<Choice>,
    pub selected: usize,
    /// Where it was last drawn, so a moved composer moves it.
    drawn: Option<Rect>,
}

/// Scalars before UTF-16 offset `i`.
fn scalars(text: &[u16], i: usize) -> usize {
    char::decode_utf16(text[..i.min(text.len())].iter().copied()).count()
}

/// The UTF-16 offset of scalar `n`.
fn units(text: &str, n: usize) -> usize {
    text.chars().take(n).map(char::len_utf16).sum()
}

impl Choice {
    /// Its two lines (an emoji's one), for drawing and UI Automation.
    pub fn lines(&self) -> (String, Option<String>) {
        match self {
            Self::Emoji(e) => (format!(":{}:", e.name.replace(' ', "_")), None),
            Self::Mention(Candidate::Person(p)) => (
                p.display_name.clone(),
                Some(if p.invited {
                    format!("@{} \u{b7} Already invited", p.username)
                } else {
                    format!("@{}", p.username)
                }),
            ),
            Self::Mention(Candidate::Everyone) => {
                ("@everyone".into(), Some("Everyone in this channel".into()))
            }
            Self::Mention(Candidate::Here) => (
                "@here".into(),
                Some("Everyone online in this channel".into()),
            ),
            Self::Mention(Candidate::Feedback(name)) => {
                (format!("@{name}"), Some(mentions::FEEDBACK_LABEL.into()))
            }
        }
    }
}

impl App {
    /// The focused composer, while one is.
    fn suggest_field(&self) -> Option<(Field, &Composer)> {
        if self.dialog.is_some() || self.signin.is_some() || self.viewer.is_some() {
            return None;
        }
        if let Some(c) = &self.profile
            && c.field.focused
        {
            return Some((Field::Profile, &c.field));
        }
        if let Some(t) = &self.thread
            && t.composer.focused
        {
            return Some((Field::Thread, &t.composer));
        }
        self.composer
            .focused
            .then_some((Field::Main, &self.composer))
    }

    fn suggest_field_mut(&mut self, field: Field) -> Option<&mut Composer> {
        match field {
            Field::Profile => self.profile.as_mut().map(|c| &mut c.field),
            Field::Thread => self.thread.as_mut().map(|t| &mut t.composer),
            Field::Main => Some(&mut self.composer),
        }
    }

    /// Who `@` can suggest in `field` now, and its specials.
    fn suggest_people(&self, field: Field) -> (Vec<mentions::Person>, mentions::Specials) {
        let Some(live) = &self.live else {
            return (Vec::new(), mentions::Specials::default());
        };
        let peer = match field {
            Field::Profile => self.profile_view().and_then(|v| {
                Some(mentions::Person {
                    username: v.profile.username.clone()?,
                    display_name: v.profile.title(),
                    avatar_id: v.profile.avatar_id,
                    avatar_asset: v.profile.avatar_asset.clone(),
                    avatar_url: v.profile.avatar_url.clone(),
                    invited: false,
                })
            }),
            _ => None,
        };
        live.mention_people(peer)
    }

    /// Recomputes the token at the focused composer's caret and its
    /// suggestions; called after every composer change.
    pub fn refresh_suggest(&mut self) {
        let before = self
            .suggest
            .as_ref()
            .map(|s| (s.field, s.token.clone(), s.choices.len(), s.selected));
        let found = self.suggest_field().and_then(|(field, f)| {
            if f.composing || !f.comp.is_empty() || f.caret != f.anchor {
                return None;
            }
            let text = String::from_utf16_lossy(&f.text);
            let caret = scalars(&f.text, f.caret);
            if let Some((df, dt, dc)) = &self.suggest_dismissed
                && (*df != field || *dt != f.text || *dc != f.caret)
            {
                return Some((field, text, caret, true));
            }
            Some((field, text, caret, false))
        });
        let Some((field, text, caret, undismiss)) = found else {
            self.set_suggest(None, before);
            return;
        };
        if undismiss {
            self.suggest_dismissed = None;
        }
        let dismissed = self.suggest_dismissed.is_some();
        let Some(token) = Token::at(&text, caret).filter(|_| !dismissed) else {
            self.set_suggest(None, before);
            return;
        };
        let choices: Vec<Choice> = match &token {
            Token::Emoji(t) => emoji::suggestions(&t.query)
                .into_iter()
                .map(Choice::Emoji)
                .collect(),
            Token::Mention(t) => {
                let (people, specials) = self.suggest_people(field);
                mentions::suggestions(&people, specials, &t.query)
                    .into_iter()
                    .map(Choice::Mention)
                    .collect()
            }
        };
        if choices.is_empty() {
            self.set_suggest(None, before);
            return;
        }
        let selected = match &self.suggest {
            Some(s) if s.field == field && s.token == token => s.selected.min(choices.len() - 1),
            _ => 0,
        };
        self.set_suggest(
            Some(Suggest {
                field,
                token,
                choices,
                selected,
                drawn: None,
            }),
            before,
        );
    }

    fn set_suggest(&mut self, next: Option<Suggest>, before: Option<(Field, Token, usize, usize)>) {
        let after = next
            .as_ref()
            .map(|s| (s.field, s.token.clone(), s.choices.len(), s.selected));
        self.suggest = next;
        if after != before {
            self.suggest_changed();
        }
    }

    fn suggest_changed(&mut self) {
        self.dirty.suggest = true;
        self.events |= EV_SCREEN;
        self.need = true;
    }

    /// The composer box the popup sits above, in client DIPs.
    fn suggest_anchor(&self, field: Field) -> Option<Rect> {
        match field {
            Field::Main => Some(self.box_rect()),
            Field::Thread => self.thread_box_rect(),
            Field::Profile => self.profile_field_rect(),
        }
    }

    /// The popup and its rows, in client DIPs.
    pub fn suggest_rects(&self) -> Option<(Rect, Vec<Rect>)> {
        let s = self.suggest.as_ref()?;
        let (ax, ay, aw, _) = self.suggest_anchor(s.field)?;
        let w = match s.token {
            Token::Mention(_) => aw,
            Token::Emoji(_) => aw.min(EMOJI_W),
        };
        let h = 2.0 * PAD + ROW_H * s.choices.len() as f32;
        let y = (ay - 6.0 - h).max(4.0);
        let rows = (0..s.choices.len())
            .map(|k| (ax + PAD, y + PAD + ROW_H * k as f32, w - 2.0 * PAD, ROW_H))
            .collect();
        Some(((ax, y, w, h), rows))
    }

    /// Inserts row `k` (None: the highlighted one) at the token.
    pub fn suggest_accept(&mut self, k: Option<usize>) {
        let Some(s) = self.suggest.take() else { return };
        let Some(choice) = s.choices.get(k.unwrap_or(s.selected)).cloned() else {
            return;
        };
        let field = s.field;
        let Some(f) = self.suggest_field_mut(field) else {
            return;
        };
        let text = String::from_utf16_lossy(&f.text);
        let inserted = match (&s.token, &choice) {
            (Token::Emoji(t), Choice::Emoji(e)) => emoji::insert(&text, t, &e.emoji),
            (Token::Mention(t), Choice::Mention(c)) => mentions::insert(&text, t, c.name()),
            _ => None,
        };
        let Some((next, caret)) = inserted else {
            return;
        };
        // Replace only what changed, through the field's own editing so
        // Undo takes it back.
        let old: Vec<u16> = text.encode_utf16().collect();
        let new: Vec<u16> = next.encode_utf16().collect();
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(new[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        f.anchor = prefix;
        f.caret = old.len() - suffix;
        f.commit(&new[prefix..new.len() - suffix]);
        f.move_to(units(&next, caret), false);
        self.suggest_changed();
        match field {
            Field::Main => self.composer_changed(),
            Field::Thread => self.thread_composer_changed(),
            Field::Profile => self.profile_changed(),
        }
    }

    /// Escape: hides the popup for this token until the text or caret moves.
    fn suggest_dismiss(&mut self) {
        let Some(s) = self.suggest.take() else { return };
        if let Some(f) = self.suggest_field_mut(s.field) {
            let (text, caret) = (f.text.clone(), f.caret);
            self.suggest_dismissed = Some((s.field, text, caret));
        }
        self.suggest_changed();
    }

    /// The popup's keys and pointer while it shows. None leaves the message
    /// to the composer and the rest of the window.
    pub fn suggest_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    ) -> Option<Option<LRESULT>> {
        let done = Some(Some(LRESULT(0)));
        let (popup, rows) = self.suggest_rects()?;
        let inside = |(rx, ry, rw, rh): Rect| x >= rx && x < rx + rw && y >= ry && y < ry + rh;
        let row = rows.iter().position(|r| inside(*r));
        match msg {
            WM_KEYDOWN if !ctrl && !shift => {
                let n = self.suggest.as_ref()?.choices.len();
                let s = self.suggest.as_mut()?;
                match VIRTUAL_KEY(wp.0 as u16) {
                    VK_DOWN => s.selected = (s.selected + 1) % n,
                    VK_UP => s.selected = (s.selected + n - 1) % n,
                    VK_RETURN | VK_TAB => {
                        self.suggest_accept(None);
                        return done;
                    }
                    VK_ESCAPE => {
                        self.suggest_dismiss();
                        return done;
                    }
                    _ => return None,
                }
                self.suggest_changed();
                done
            }
            // The characters those keys type.
            WM_CHAR if matches!(wp.0 as u16, 0x09 | 0x0D | 0x1B) && !ctrl => done,
            WM_MOUSEMOVE if inside(popup) => {
                self.pointer = Some((x, y));
                if let (Some(k), Some(s)) = (row, self.suggest.as_mut())
                    && s.selected != k
                {
                    s.selected = k;
                    self.suggest_changed();
                }
                done
            }
            WM_LBUTTONDOWN if inside(popup) => {
                if let Some(k) = row {
                    self.suggest_accept(Some(k));
                }
                done
            }
            WM_SETCURSOR if inside(popup) => {
                let cursor = if row.is_some() { IDC_HAND } else { IDC_ARROW };
                unsafe { SetCursor(LoadCursorW(None, cursor).ok()) };
                Some(Some(LRESULT(1)))
            }
            WM_MOUSEWHEEL | WM_LBUTTONDBLCLK if inside(popup) => done,
            _ => None,
        }
    }

    /// Redraws the popup's surface when it changed.
    pub fn draw_suggest(&mut self) -> Result<()> {
        let dirty = std::mem::take(&mut self.dirty.suggest);
        let placed = self.suggest_rects();
        let Some(((px, py, pw, ph), rows)) = placed else {
            return if dirty {
                self.suggest_p.offset(-100_000.0, 0.0)
            } else {
                Ok(())
            };
        };
        let Some(s) = self.suggest.as_mut() else {
            return Ok(());
        };
        if !dirty && s.drawn == Some((px, py, pw, ph)) {
            return Ok(());
        }
        s.drawn = Some((px, py, pw, ph));
        let Some(s) = self.suggest.as_ref() else {
            return Ok(());
        };
        let scale = self.scale;
        let size = |v: f32| (v * scale).ceil().max(1.0) as u32;
        let g = &self.g;
        self.suggest_p.ensure(g, size(pw), size(ph))?;
        self.suggest_p
            .offset((px * scale).round(), (py * scale).round())?;
        self.suggest_p.draw(scale, |dc| unsafe {
            dc.Clear(Some(&D2D1_COLOR_F::default()));
            let body = rect(0.5, 0.5, pw - 0.5, ph - 0.5);
            fill_round(dc, body, RADIUS, &g.solid(SURFACE_COMPOSER));
            stroke_round(dc, body, RADIUS, &g.brushes.border);
            for (k, (choice, &(rx, ry, rw, rh))) in s.choices.iter().zip(&rows).enumerate() {
                let (x, y) = (rx - px, ry - py);
                let r = rect(x, y, x + rw, y + rh);
                if k == s.selected {
                    fill_round(dc, r, 6.0, &g.solid(SELECTED));
                }
                let icon = rect(x + 8.0, y + 10.0, x + 32.0, y + 34.0);
                match choice {
                    Choice::Emoji(e) => crate::emoji::draw(dc, e, icon),
                    Choice::Mention(Candidate::Person(p)) => {
                        let avatar = Avatar::new(p.avatar_id, &p.avatar_asset, &p.avatar_url);
                        let face = crate::live::person(&p.username, &p.display_name, avatar);
                        avatars::circle(
                            g,
                            dc,
                            &face.avatar,
                            (x + 20.0, y + rh / 2.0, 12.0),
                            face.color,
                            &face.initials,
                        )?;
                    }
                    Choice::Mention(_) => {
                        let at = layout(g, "@", &g.formats.direct_name, 24.0)?;
                        crate::text::draw(
                            dc,
                            &at,
                            x + 14.0,
                            y + (rh - height(&at)) / 2.0,
                            &g.brushes.muted,
                        );
                    }
                }
                let tx = x + 44.0;
                let tw = (rw - 44.0 - 8.0).max(1.0);
                let (title, detail) = choice.lines();
                let t = layout(g, &title, &g.formats.direct_name, tw)?;
                t.SetWordWrapping(
                    windows::Win32::Graphics::DirectWrite::DWRITE_WORD_WRAPPING_NO_WRAP,
                )?;
                match detail {
                    Some(d) => {
                        crate::text::draw(dc, &t, tx, y + 5.0, &g.brushes.text);
                        let l = layout(g, &d, &g.formats.direct_note, tw)?;
                        l.SetWordWrapping(
                            windows::Win32::Graphics::DirectWrite::DWRITE_WORD_WRAPPING_NO_WRAP,
                        )?;
                        crate::text::draw(dc, &l, tx, y + 24.0, &g.brushes.muted);
                    }
                    None => {
                        crate::text::draw(dc, &t, tx, y + (rh - height(&t)) / 2.0, &g.brushes.text)
                    }
                }
            }
            Ok(())
        })
    }
}
