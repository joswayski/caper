//! A private profile draft. Uploading starts only on Save; failed saves retain
//! the uploaded asset for retry. Requests belong to this editor and sign-in.
use crate::avatars::{self, Face};
use crate::composer::Composer;
use crate::gfx::{Gfx, fill_round, rect, stroke_round};
use crate::overlay::{Dialog, Kind, Part, Rect, height, layout};
use crate::{App, EV_FOCUS, EV_SCREEN, theme::*};
use caper_client::{
    api::Api, model::Account, names, pictures, profile_edit::Draft, uploads::Scratch,
};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Result;

type Answer<T> = mpsc::Receiver<std::result::Result<T, String>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Choose,
    Edit,
    Remove,
    Regenerate,
    Gallery,
    Previous(usize),
    More,
}

impl Control {
    pub fn index(self) -> usize {
        match self {
            Self::Choose => 0,
            Self::Edit => 1,
            Self::Remove => 2,
            Self::Regenerate => 3,
            Self::Gallery => 4,
            Self::More => 5,
            Self::Previous(k) => 10 + k,
        }
    }
}

pub struct Editor {
    epoch: u64,
    api: Api,
    token: String,
    pub draft: Option<Draft>,
    pub name: Composer,
    pub focus: Part,
    loading: Option<Answer<Account>>,
    saving: Option<Answer<Account>>,
    picking: Option<mpsc::Receiver<Vec<PathBuf>>>,
    editing: Option<Answer<Arc<Scratch>>>,
    history: Vec<pictures::PreviousPicture>,
    history_open: bool,
    history_loading: Option<Answer<pictures::History>>,
    cursor: Option<String>,
    history_error: Option<String>,
    local: Option<String>,
}

impl Drop for Editor {
    fn drop(&mut self) {
        if let Some(key) = &self.local {
            avatars::forget_local(key);
        }
    }
}

fn request<T: Send + 'static>(
    hwnd: HWND,
    run: impl FnOnce() -> std::result::Result<T, String> + Send + 'static,
) -> Answer<T> {
    let (tx, rx) = mpsc::channel();
    let window = hwnd.0 as isize;
    std::thread::spawn(move || {
        let _ = tx.send(run());
        wake(window);
    });
    rx
}

fn wake(window: isize) {
    unsafe {
        let _ = PostMessageW(
            Some(HWND(window as _)),
            crate::live::WM_LIVE,
            WPARAM(0),
            LPARAM(0),
        );
    }
}

fn take<T>(pending: &mut Option<Answer<T>>) -> Option<std::result::Result<T, String>> {
    let result = match pending.as_ref()?.try_recv() {
        Ok(result) => result,
        Err(mpsc::TryRecvError::Empty) => return None,
        Err(_) => Err("The request stopped. Try again.".into()),
    };
    *pending = None;
    Some(result)
}

fn body_part(p: Part) -> bool {
    !matches!(p, Part::Title | Part::Close | Part::Cancel | Part::Primary)
}

pub fn focusable(p: Part) -> bool {
    matches!(
        p,
        Part::Field
            | Part::Number(0)
            | Part::Close
            | Part::Cancel
            | Part::Primary
            | Part::Retry
            | Part::Profile(_)
    )
}

fn contains(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

impl Editor {
    fn new(epoch: u64, api: Api, token: String) -> Self {
        Self {
            epoch,
            api,
            token,
            draft: None,
            name: Composer::default(),
            focus: Part::Close,
            loading: None,
            saving: None,
            picking: None,
            editing: None,
            history: Vec::new(),
            history_open: false,
            history_loading: None,
            cursor: None,
            history_error: None,
            local: None,
        }
    }

    fn protected(&self) -> bool {
        self.saving.is_some() || self.picking.is_some()
    }

    fn pending(&self) -> bool {
        self.loading.is_some()
            || self.saving.is_some()
            || self.picking.is_some()
            || self.editing.is_some()
            || self.draft.as_ref().is_some_and(|d| d.preparing.is_some())
    }

    fn dirty(&self, username: &str) -> bool {
        self.draft
            .as_ref()
            .is_some_and(|d| d.dirty(username, &String::from_utf16_lossy(&self.name.text)))
    }

    fn confirm(&self) -> bool {
        self.draft.as_ref().is_some_and(|d| d.confirm_discard)
    }
}

impl Dialog {
    pub fn profile_name(&self, part: Part) -> String {
        let Some(e) = &self.profile_edit else {
            return String::new();
        };
        match part {
            Part::Title => "Edit profile".into(),
            Part::Close => "Close Edit profile".into(),
            Part::Field | Part::Line(0) => "Username".into(),
            Part::Number(0) | Part::Line(2) => "Display name".into(),
            Part::Line(1) => {
                "3–32 lowercase letters, numbers, or underscores. Unique across Caper.".into()
            }
            Part::Line(3) => "Previous pictures".into(),
            Part::Line(4) => if e.history_loading.is_some() {
                "Loading pictures…"
            } else {
                "No previous pictures."
            }
            .into(),
            Part::Line(5) => e.history_error.clone().unwrap_or_default(),
            Part::Loading => if e.draft.as_ref().is_some_and(|d| d.preparing.is_some()) {
                "Preparing picture…"
            } else {
                "Loading profile…"
            }
            .into(),
            Part::Preview => "Profile picture preview".into(),
            Part::Error => self.error.clone().unwrap_or_default(),
            Part::Hint => "Discard your profile changes?".into(),
            Part::Retry => "Retry loading profile".into(),
            Part::Cancel => if e.confirm() {
                "Keep editing"
            } else {
                "Cancel"
            }
            .into(),
            Part::Primary => if e.confirm() {
                "Discard changes"
            } else if self.busy {
                "Saving…"
            } else {
                "Save profile"
            }
            .into(),
            Part::Profile(c) => match c {
                Control::Choose => "Choose profile picture".into(),
                Control::Edit => "Edit picture".into(),
                Control::Remove => "Remove picture".into(),
                Control::Regenerate => "Regenerate avatar".into(),
                Control::Gallery => "Previous pictures".into(),
                Control::More => if e.history_error.is_some() {
                    "Retry pictures"
                } else {
                    "Show more pictures"
                }
                .into(),
                Control::Previous(k) => format!(
                    "Use previous picture {}{}",
                    k + 1,
                    if e.history
                        .get(k)
                        .is_some_and(|p| e.draft.as_ref().is_some_and(|d| d
                            .preview
                            .avatar_asset
                            .as_ref()
                            == Some(&p.avatar_asset)))
                    {
                        ", selected"
                    } else {
                        ""
                    }
                ),
            },
            _ => String::new(),
        }
    }

    pub fn profile_enabled(&self, p: Part) -> bool {
        let Some(e) = &self.profile_edit else {
            return false;
        };
        if !focusable(p) {
            return false;
        }
        if matches!(p, Part::Close | Part::Cancel) {
            return !e.protected();
        }
        if e.confirm() {
            return p == Part::Primary && !e.protected();
        }
        if matches!(p, Part::Field | Part::Number(0)) {
            return e.draft.is_some() && !e.protected();
        }
        if e.pending() {
            return false;
        }
        let Some(d) = &e.draft else {
            return p == Part::Retry;
        };
        match p {
            Part::Primary => {
                e.dirty(&self.draft())
                    && !self.field.composing
                    && self.field.comp.is_empty()
                    && !e.name.composing
                    && e.name.comp.is_empty()
            }
            Part::Profile(Control::Edit | Control::Remove) => d.preview.avatar_asset.is_some(),
            Part::Profile(Control::Regenerate) => d.preview.avatar_asset.is_none(),
            Part::Profile(Control::More | Control::Gallery) => e.history_loading.is_none(),
            _ => true,
        }
    }

    pub fn profile_field_origin(&self, p: Part) -> (f32, f32) {
        self.part_rect(p).map_or((0.0, 0.0), |r| {
            (r.0 + 10.0, r.1 + 9.0 - self.field_for(p).scroll())
        })
    }

    pub fn profile_hit(&self, x: f32, y: f32) -> Option<Part> {
        self.parts
            .iter()
            .find(|(p, r)| {
                focusable(*p) && contains(*r, x, y) && (!body_part(*p) || contains(self.body, x, y))
            })
            .map(|(p, _)| *p)
    }

    pub fn profile_bounds(&self, p: Part) -> Option<Rect> {
        let (x, y, w, h) = self.part_rect(p)?;
        if !body_part(p) {
            return Some((x, y, w, h));
        }
        let top = y.max(self.body.1);
        let bottom = (y + h).min(self.body.1 + self.body.3);
        Some((x, top, w, (bottom - top).max(0.0)))
    }

    pub fn profile_focus(&mut self, p: Part) {
        if !self.profile_enabled(p) {
            return;
        }
        self.field.focused = p == Part::Field;
        if let Some(e) = &mut self.profile_edit {
            e.focus = p;
            e.name.focused = p == Part::Number(0);
            e.name.caret_on = true;
        }
        if body_part(p)
            && let Some(r) = self.part_rect(p)
        {
            if r.1 < self.body.1 {
                self.scroll -= self.body.1 - r.1;
            } else if r.1 + r.3 > self.body.1 + self.body.3 {
                self.scroll += r.1 + r.3 - self.body.1 - self.body.3;
            }
        }
    }

    pub fn profile_changed(&mut self) {
        fn normalize(f: &mut Composer, username: bool) {
            // WM_CHAR delivers a supplementary character in two messages.
            // Keep the first UTF-16 unit until its partner arrives.
            if f.composing || char::decode_utf16(f.text.iter().copied()).any(|c| c.is_err()) {
                return;
            }
            let clean = |s: &[u16]| -> Vec<u16> {
                let text = String::from_utf16_lossy(s);
                if username {
                    names::normalize_username(&text).encode_utf16().collect()
                } else {
                    text.chars()
                        .filter(|c| !c.is_control())
                        .take(64)
                        .collect::<String>()
                        .encode_utf16()
                        .collect()
                }
            };
            let next = clean(&f.text);
            if next != f.text {
                f.caret = clean(&f.text[..f.caret.min(f.text.len())]).len();
                f.anchor = clean(&f.text[..f.anchor.min(f.text.len())]).len();
                f.text = next;
            }
        }
        normalize(&mut self.field, true);
        if let Some(e) = &mut self.profile_edit {
            normalize(&mut e.name, false);
            e.name.caret_on = true;
        }
    }

    pub fn relayout_profile(&mut self, g: &Gfx, w: f32, h: f32) -> Result<()> {
        let cw = (w - 32.0).clamp(200.0, 500.0);
        let inner = cw - 40.0;
        let mut parts = Vec::new();
        let mut y = 0.0;
        let e = self.profile_edit.as_mut().unwrap();
        if let Some(draft) = &e.draft {
            parts.push((Part::Preview, (0.0, y, 96.0, 96.0)));
            y += 108.0;
            for control in [
                Control::Choose,
                Control::Edit,
                Control::Remove,
                Control::Regenerate,
                Control::Gallery,
            ] {
                if matches!(control, Control::Edit | Control::Remove)
                    && draft.preview.avatar_asset.is_none()
                {
                    continue;
                }
                if control == Control::Regenerate && draft.preview.avatar_asset.is_some() {
                    continue;
                }
                parts.push((Part::Profile(control), (0.0, y, inner, 34.0)));
                y += 40.0;
            }
            if e.history_open {
                parts.push((Part::Line(3), (0.0, y, inner, 24.0)));
                y += 28.0;
                let columns = ((inner + 8.0) / 64.0).floor().max(1.0) as usize;
                for k in 0..e.history.len() {
                    parts.push((
                        Part::Profile(Control::Previous(k)),
                        (
                            (k % columns) as f32 * 64.0,
                            y + (k / columns) as f32 * 64.0,
                            56.0,
                            56.0,
                        ),
                    ));
                }
                y += e.history.len().div_ceil(columns) as f32 * 64.0;
                if e.history_loading.is_some()
                    || (e.history.is_empty() && e.history_error.is_none())
                {
                    parts.push((Part::Line(4), (0.0, y, inner, 24.0)));
                    y += 28.0;
                }
                if let Some(error) = &e.history_error {
                    let eh = height(&layout(g, error, &g.formats.error, inner)?);
                    parts.push((Part::Line(5), (0.0, y, inner, eh)));
                    y += eh + 8.0;
                }
                if e.cursor.is_some() || e.history_error.is_some() {
                    parts.push((Part::Profile(Control::More), (0.0, y, inner, 34.0)));
                    y += 42.0;
                }
            }
            self.field
                .relayout_with(g, &g.formats.composer, 15.0, inner - 20.0)?;
            e.name
                .relayout_with(g, &g.formats.composer, 15.0, inner - 20.0)?;
            parts.push((Part::Line(0), (0.0, y, inner, 22.0)));
            y += 26.0;
            parts.push((Part::Field, (0.0, y, inner, 42.0)));
            y += 48.0;
            let hint = "3–32 lowercase letters, numbers, or underscores. Unique across Caper.";
            let hh = height(&layout(g, hint, &g.formats.dialog_note, inner)?);
            parts.push((Part::Line(1), (0.0, y, inner, hh)));
            y += hh + 14.0;
            parts.push((Part::Line(2), (0.0, y, inner, 22.0)));
            y += 26.0;
            parts.push((Part::Number(0), (0.0, y, inner, 42.0)));
            y += 50.0;
        } else if e.loading.is_none() {
            parts.push((Part::Retry, (0.0, y, inner, 36.0)));
            y += 44.0;
        }
        if e.loading.is_some() || e.draft.as_ref().is_some_and(|d| d.preparing.is_some()) {
            parts.push((Part::Loading, (0.0, y, inner, 24.0)));
            y += 32.0;
        }
        if e.confirm() {
            parts.push((Part::Hint, (0.0, y, inner, 24.0)));
            y += 32.0;
        }
        if let Some(error) = &self.error {
            let eh = height(&layout(g, error, &g.formats.error, inner)?);
            parts.push((Part::Error, (0.0, y, inner, eh)));
            y += eh + 8.0;
        }
        let ch = (y + 132.0).min(h - 32.0).max(160.0);
        let (cx, cy) = ((w - cw) / 2.0, (h - ch) / 2.0);
        self.card = (cx, cy, cw, ch);
        self.body = (cx + 20.0, cy + 60.0, inner, ch - 124.0);
        self.scroll = self.scroll.clamp(0.0, (y - self.body.3).max(0.0));
        for (_, r) in &mut parts {
            r.0 += self.body.0;
            r.1 += self.body.1 - self.scroll;
        }
        parts.insert(0, (Part::Title, (cx + 20.0, cy + 16.0, inner - 36.0, 28.0)));
        parts.push((Part::Close, (cx + cw - 48.0, cy + 12.0, 32.0, 32.0)));
        let bw = (inner - 8.0) / 2.0;
        parts.push((Part::Cancel, (cx + 20.0, cy + ch - 52.0, bw, 36.0)));
        parts.push((Part::Primary, (cx + 28.0 + bw, cy + ch - 52.0, bw, 36.0)));
        self.parts = parts;
        self.field_origin = self.profile_field_origin(Part::Field);
        Ok(())
    }

    pub fn draw_profile(&self, g: &Gfx, dc: &ID2D1DeviceContext) -> Result<()> {
        let e = self.profile_edit.as_ref().unwrap();
        for &(p, (x, y, w, h)) in &self.parts {
            let r = rect(x, y, x + w, y + h);
            unsafe {
                if body_part(p) {
                    dc.PushAxisAlignedClip(
                        &rect(
                            self.body.0,
                            self.body.1,
                            self.body.0 + self.body.2,
                            self.body.1 + self.body.3,
                        ),
                        D2D1_ANTIALIAS_MODE_ALIASED,
                    );
                }
                if matches!(p, Part::Field | Part::Number(0)) {
                    fill_round(dc, r, RADIUS, &g.brushes.composer);
                    stroke_round(
                        dc,
                        r,
                        RADIUS,
                        if e.focus == p {
                            &g.brushes.terracotta
                        } else {
                            &g.brushes.border
                        },
                    );
                    dc.PushAxisAlignedClip(
                        &rect(x + 1.0, y + 1.0, x + w - 1.0, y + h - 1.0),
                        D2D1_ANTIALIAS_MODE_ALIASED,
                    );
                    let (ox, oy) = self.profile_field_origin(p);
                    self.field_for(p).draw_text(g, dc, ox, oy, None);
                    dc.PopAxisAlignedClip();
                } else if p == Part::Preview {
                    if let Some(d) = &e.draft {
                        avatars::draw(dc, &d.preview.avatar(), r, RADIUS);
                    }
                } else if let Part::Profile(Control::Previous(k)) = p {
                    let photo = &e.history[k];
                    avatars::draw(
                        dc,
                        &avatars::Avatar::new(
                            None,
                            &Some(photo.avatar_asset.clone()),
                            &Some(photo.avatar_url.clone()),
                        ),
                        r,
                        RADIUS,
                    );
                    if e.focus == p
                        || e.draft.as_ref().is_some_and(|d| {
                            d.preview.avatar_asset.as_ref() == Some(&photo.avatar_asset)
                        })
                    {
                        stroke_round(dc, r, RADIUS, &g.brushes.terracotta);
                    }
                } else {
                    let enabled = self.profile_enabled(p);
                    let action = focusable(p);
                    let primary = p == Part::Primary && enabled;
                    if action {
                        fill_round(
                            dc,
                            r,
                            RADIUS,
                            if primary {
                                &g.brushes.eggshell
                            } else if self.hover == Some(p) {
                                &g.brushes.hover
                            } else {
                                &g.brushes.raised
                            },
                        );
                        stroke_round(
                            dc,
                            r,
                            RADIUS,
                            if e.focus == p {
                                &g.brushes.terracotta
                            } else {
                                &g.brushes.border
                            },
                        );
                    }
                    let label = if p == Part::Close {
                        "×".into()
                    } else {
                        self.profile_name(p)
                    };
                    let format = if p == Part::Title {
                        &g.formats.dialog_title
                    } else if action {
                        &g.formats.button
                    } else {
                        &g.formats.dialog_note
                    };
                    let l = layout(g, &label, format, if action { w - 16.0 } else { w })?;
                    let brush = if primary {
                        g.solid(BLACKOUT)
                    } else if matches!(p, Part::Error | Part::Line(5)) {
                        g.solid(ERROR)
                    } else if action && !enabled {
                        g.solid(TEXT_MUTED)
                    } else {
                        g.solid(TEXT)
                    };
                    crate::text::draw(
                        dc,
                        &l,
                        x + if action { 8.0 } else { 0.0 },
                        y + if action { (h - height(&l)) / 2.0 } else { 0.0 },
                        &brush,
                    );
                }
                if body_part(p) {
                    dc.PopAxisAlignedClip();
                }
            }
        }
        Ok(())
    }
}

impl App {
    pub fn open_profile_editor(&mut self) {
        let Some((epoch, api, token)) = self.live.as_ref().and_then(|l| l.profile_session()) else {
            return;
        };
        self.close_profile();
        self.close_settings();
        let (a, t) = (api.clone(), token.clone());
        let loading = Some(request(self.hwnd, move || a.me(&t).map_err(|e| e.message)));
        let mut d = Dialog::new(Kind::Profile, None, Composer::default());
        let mut editor = Editor::new(epoch, api, token);
        editor.loading = loading;
        d.profile_edit = Some(Box::new(editor));
        self.open_dialog(d);
    }

    pub fn close_profile_editor(&mut self) {
        let Some(d) = self.dialog.as_mut().filter(|d| d.kind == Kind::Profile) else {
            return;
        };
        let username = d.draft();
        let e = d.profile_edit.as_mut().unwrap();
        if e.protected() {
            return;
        }
        if e.dirty(&username) {
            if let Some(draft) = &mut e.draft {
                draft.confirm_discard = true;
            }
            d.profile_focus(Part::Cancel);
            d.scroll = f32::MAX / 2.0;
            self.dialog_changed();
        } else {
            self.close_dialog();
        }
    }

    pub fn profile_edit_activate(&mut self, p: Part) {
        let Some(d) = self.dialog.as_mut().filter(|d| d.kind == Kind::Profile) else {
            return;
        };
        if !d.profile_enabled(p) {
            return;
        }
        d.profile_focus(p);
        let username = d.draft();
        let e = d.profile_edit.as_mut().unwrap();
        let (api, token) = (e.api.clone(), e.token.clone());
        match p {
            Part::Close => return self.close_profile_editor(),
            Part::Cancel if e.confirm() => {
                e.draft.as_mut().unwrap().confirm_discard = false;
            }
            Part::Cancel => return self.close_profile_editor(),
            Part::Primary if e.confirm() => {
                self.close_dialog();
                return;
            }
            Part::Primary => {
                let draft = e.draft.as_ref().unwrap();
                let name = String::from_utf16_lossy(&e.name.text).trim().to_owned();
                let invalid = if username.len() < 3 {
                    Some("Username must be at least 3 characters.")
                } else if name.is_empty() {
                    Some("Enter a display name.")
                } else {
                    names::display_name_error(&name, draft.baseline.display_name.as_deref())
                };
                if let Some(error) = invalid {
                    d.error = Some(error.into());
                } else {
                    let avatar = draft.save();
                    e.saving = Some(request(self.hwnd, move || {
                        let payload = avatar.map(|v| v.payload(&api, &token)).transpose()?;
                        api.update_profile(&token, &username, &name, payload)
                            .map_err(|e| caper_client::worker::profile_error(&e))
                    }));
                    d.busy = true;
                    d.error = None;
                }
            }
            Part::Retry => {
                e.loading = Some(request(self.hwnd, move || {
                    api.me(&token).map_err(|e| e.message)
                }));
                d.error = None;
            }
            Part::Profile(Control::Choose) => {
                e.picking = Some(crate::uploads::pick_picture(self.hwnd));
            }
            Part::Profile(Control::Remove) => {
                e.draft.as_mut().unwrap().select(None, None);
            }
            Part::Profile(Control::Regenerate) => {
                e.draft.as_mut().unwrap().regenerate();
            }
            Part::Profile(Control::Gallery | Control::More) => {
                e.history_open = true;
                let cursor = if p == Part::Profile(Control::Gallery) {
                    e.history.clear();
                    e.cursor = None;
                    None
                } else {
                    e.cursor.clone()
                };
                e.history_error = None;
                e.history_loading = Some(request(self.hwnd, move || {
                    api.avatar_history(&token, cursor.as_deref())
                        .map_err(|e| e.message)
                }));
            }
            Part::Profile(Control::Previous(k)) => {
                if let Some(picture) = e.history.get(k) {
                    e.draft.as_mut().unwrap().select(
                        Some(picture.avatar_asset.clone()),
                        Some(picture.avatar_url.clone()),
                    );
                }
            }
            Part::Profile(Control::Edit) => {
                let draft = e.draft.as_ref().unwrap();
                let (picture, url) = (draft.picture.clone(), draft.preview.avatar_url.clone());
                e.editing = Some(request(self.hwnd, move || {
                    let bytes = if let Some(picture) = picture {
                        picture.bytes.clone()
                    } else {
                        api.fetch_media(
                            url.as_deref()
                                .ok_or("This picture is unavailable. Choose it again.")?,
                        )
                        .map_err(|_| "Could not load this picture. Try again.")?
                    };
                    let scratch = Arc::new(Scratch::new(if pictures::is_gif(&bytes) {
                        "gif"
                    } else {
                        "png"
                    }));
                    std::fs::write(scratch.path(), bytes)
                        .map_err(|_| "Could not prepare this picture for editing.")?;
                    Ok(scratch)
                }));
            }
            _ => {}
        }
        self.dialog_changed();
        self.events |= EV_FOCUS;
    }

    pub fn poll_profile_editor(&mut self) {
        let session = self
            .live
            .as_ref()
            .and_then(|l| l.profile_session())
            .map(|(e, _, _)| e);
        let Some(d) = self.dialog.as_mut().filter(|d| d.kind == Kind::Profile) else {
            return;
        };
        let e = d.profile_edit.as_mut().unwrap();
        if session != Some(e.epoch) {
            self.close_attachment_editor();
            self.close_dialog();
            return;
        }
        let mut changed = false;
        let mut loaded = false;
        if let Some(result) = take(&mut e.loading) {
            match result {
                Ok(account) => {
                    d.field = crate::custom_links::number(
                        account.username.as_deref().unwrap_or_default(),
                    );
                    e.name = crate::custom_links::number(
                        account.display_name.as_deref().unwrap_or_default(),
                    );
                    e.draft = Some(Draft::new(&account));
                    e.focus = Part::Field;
                    d.field.focused = true;
                    loaded = true;
                }
                Err(error) => d.error = Some(error),
            }
            changed = true;
        }
        if let Some(result) = take(&mut e.history_loading) {
            match result {
                Ok(page) => {
                    for picture in page.pictures {
                        if !e
                            .history
                            .iter()
                            .any(|p| p.avatar_asset == picture.avatar_asset)
                        {
                            e.history.push(picture);
                        }
                    }
                    e.cursor = page.next_cursor;
                    e.history_error = None;
                }
                Err(error) => e.history_error = Some(error),
            }
            changed = true;
        }
        if let Some(rx) = &e.picking
            && let Ok(paths) = rx.try_recv()
        {
            e.picking = None;
            if let (Some(path), Some(draft)) = (paths.into_iter().next(), &mut e.draft) {
                let window = self.hwnd.0 as isize;
                draft.prepare(path, None, None, false, move || wake(window));
                d.error = None;
            }
            changed = true;
        }
        if let Some(result) = e.draft.as_mut().and_then(Draft::poll) {
            match result {
                Ok((key, frames)) => {
                    if let Some(old) = e.local.replace(key.clone()) {
                        avatars::forget_local(&old);
                    }
                    if let Some((image, _)) = frames.into_iter().next() {
                        avatars::local(key, image);
                    }
                }
                Err(error) => d.error = Some(error),
            }
            changed = true;
        }
        let edit = take(&mut e.editing);
        if let Some(result) = take(&mut e.saving) {
            d.busy = false;
            match result {
                Ok(account) => {
                    if let (Some(key), Some(asset)) = (&e.local, &account.avatar_asset)
                        && e.draft.as_ref().is_some_and(|d| d.picture.is_some())
                    {
                        avatars::save_local(key, asset);
                    }
                    if let Some(live) = &mut self.live {
                        live.profile_saved(&account);
                    }
                    self.close_dialog();
                    self.refresh_saved_profile();
                    return;
                }
                Err(error) => d.error = Some(error),
            }
            changed = true;
        }
        if let Some(result) = edit {
            match result {
                Ok(scratch) => self.open_profile_picture_editor(scratch),
                Err(error) => {
                    self.dialog.as_mut().unwrap().error = Some(error);
                }
            }
            changed = true;
        }
        if changed {
            self.dialog_changed();
            if let Some(d) = &mut self.dialog {
                if loaded {
                    d.profile_focus(Part::Field);
                }
                if d.error.is_some() {
                    d.scroll = f32::MAX / 2.0;
                }
            }
            self.dialog_changed();
            self.events |= EV_SCREEN;
        }
    }

    pub fn profile_picture_edited(
        &mut self,
        scratch: Arc<Scratch>,
        edit: Option<caper_client::attachment_edit::Edit>,
    ) {
        if let Some(d) = &mut self.dialog
            && let Some(e) = &mut d.profile_edit
            && let Some(draft) = &mut e.draft
        {
            let window = self.hwnd.0 as isize;
            draft.prepare(
                scratch.path().to_owned(),
                edit,
                Some(scratch),
                false,
                move || wake(window),
            );
            d.error = None;
        }
        self.dialog_changed();
    }

    fn refresh_saved_profile(&mut self) {
        let Some(live) = &self.live else { return };
        // Rebuild displayed identities without navigating, resetting drafts or
        // replacing the timeline's cursors. Preserve message-local reveal state.
        if let Some((mut c, ids)) = live.profile_snapshot() {
            for message in &mut c.messages {
                if let Some(old) = self.c.messages.iter().find(|m| m.id == message.id) {
                    message.revealed.clone_from(&old.revealed);
                }
            }
            if let Err(error) = self.prepend_history(c, ids) {
                eprintln!("profile refresh failed: {error:?}");
            }
            for i in 0..self.c.messages.len() {
                if let Err(error) = self.rows.replace(&self.g, &self.c, i) {
                    eprintln!("profile row refresh failed: {error:?}");
                }
            }
            if let Ok((channels, members)) = crate::side_layouts(&self.g, &self.c) {
                self.channel_layouts = channels;
                self.member_layouts = members;
            }
        }
        if let Err(error) = self.thread_refresh() {
            eprintln!("profile thread refresh failed: {error:?}");
        }
        self.avatars_changed();
        self.direct_head_changed();
        self.profile_changed();
    }

    pub fn profile_edit_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    ) -> Option<Option<LRESULT>> {
        let d = self.dialog.as_mut()?;
        let done = Some(Some(LRESULT(0)));
        match msg {
            WM_MOUSEMOVE => {
                self.pointer = Some((x, y));
                if let Some(p @ (Part::Field | Part::Number(0))) = d.pressed {
                    let (ox, oy) = d.profile_field_origin(p);
                    let f = d.field_for_mut(p);
                    f.move_to(f.hit(x - ox, y - oy), true);
                }
                d.hover = d.profile_hit(x, y);
            }
            WM_LBUTTONDOWN => {
                let hit = d.profile_hit(x, y);
                if let Some(p) = hit.filter(|p| d.profile_enabled(*p)) {
                    d.profile_focus(p);
                    d.pressed = Some(p);
                    unsafe {
                        SetCapture(self.hwnd);
                    }
                    if matches!(p, Part::Field | Part::Number(0)) {
                        let (ox, oy) = d.profile_field_origin(p);
                        let f = d.field_for_mut(p);
                        f.move_to(f.hit(x - ox, y - oy), shift);
                    }
                } else if !d.on_card(x, y) {
                    self.close_profile_editor();
                }
            }
            WM_LBUTTONUP => {
                unsafe {
                    let _ = ReleaseCapture();
                }
                let hit = d.profile_hit(x, y);
                if let Some(p) = d
                    .pressed
                    .take()
                    .filter(|p| hit == Some(*p) && !matches!(p, Part::Field | Part::Number(0)))
                {
                    self.profile_edit_activate(p);
                }
            }
            WM_MOUSEWHEEL => d.scroll -= (wp.0 >> 16) as u16 as i16 as f32 / 120.0 * 60.0,
            WM_SETCURSOR => {
                let cursor = match self.pointer.and_then(|(x, y)| d.profile_hit(x, y)) {
                    Some(Part::Field | Part::Number(0)) => IDC_IBEAM,
                    Some(p) if d.profile_enabled(p) => IDC_HAND,
                    _ => IDC_ARROW,
                };
                unsafe {
                    SetCursor(LoadCursorW(None, cursor).ok());
                }
                return Some(Some(LRESULT(1)));
            }
            WM_KEYDOWN => {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                let focus = d.profile_edit.as_ref().unwrap().focus;
                if matches!(focus, Part::Field | Part::Number(0)) {
                    let field = d.field_for(focus);
                    // The IME owns Enter and Escape until its text is committed.
                    if field.composing || !field.comp.is_empty() {
                        return Some(None);
                    }
                }
                match vk {
                    VK_ESCAPE => self.close_profile_editor(),
                    VK_TAB => {
                        let choices: Vec<_> = d.parts().filter(|p| d.profile_enabled(*p)).collect();
                        if !choices.is_empty() {
                            let i = choices.iter().position(|p| *p == focus).unwrap_or(0);
                            d.profile_focus(
                                choices[(i + if shift { choices.len() - 1 } else { 1 })
                                    % choices.len()],
                            );
                        }
                    }
                    VK_RETURN => self.profile_edit_activate(
                        if ctrl || matches!(focus, Part::Field | Part::Number(0)) {
                            Part::Primary
                        } else {
                            focus
                        },
                    ),
                    VK_SPACE if !matches!(focus, Part::Field | Part::Number(0)) => {
                        self.profile_edit_activate(focus)
                    }
                    VK_PRIOR | VK_NEXT if !matches!(focus, Part::Field | Part::Number(0)) => {
                        d.scroll += if vk == VK_PRIOR { -d.body.3 } else { d.body.3 };
                    }
                    _ if matches!(focus, Part::Field | Part::Number(0))
                        && d.profile_enabled(focus) =>
                    {
                        let field = d.field_for_mut(focus);
                        if ctrl && vk == VIRTUAL_KEY(b'C' as u16) {
                            let (a, b) = field.selection();
                            if b > a {
                                crate::set_clipboard(self.hwnd, &field.text[a..b]);
                            }
                        } else {
                            field.edit_key(self.hwnd, vk, ctrl, shift, false);
                        }
                    }
                    _ => {}
                }
            }
            WM_CHAR => {
                let p = d.profile_edit.as_ref().unwrap().focus;
                let c = wp.0 as u16;
                if !ctrl
                    && c >= 0x20
                    && c != 0x7f
                    && matches!(p, Part::Field | Part::Number(0))
                    && d.profile_enabled(p)
                {
                    d.field_for_mut(p).insert(&[c]);
                }
            }
            _ => return None,
        }
        self.dialog_changed();
        self.events |= EV_FOCUS;
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_fields_stay_private_and_cancel_is_protected_only_during_save_or_picker() {
        let account: Account = serde_json::from_value(serde_json::json!({
            "id":"self", "username":"alice", "displayName":"Alice", "avatarId":17
        }))
        .unwrap();
        let mut e = Editor::new(1, Api::new("https://example.test").unwrap(), "token".into());
        e.draft = Some(Draft::new(&account));
        e.name = crate::custom_links::number("Alice");
        let mut d = Dialog::new(Kind::Profile, None, crate::custom_links::number("alice"));
        d.profile_edit = Some(Box::new(e));
        assert!(!d.enabled(Part::Primary));
        d.profile_focus(Part::Number(0));
        assert_eq!(d.number_focus(), Some(0));
        d.field_for_mut(Part::Number(0))
            .set_text("José 🌱\nValerio".encode_utf16().collect());
        d.profile_changed();
        assert_eq!(d.draft(), "alice");
        assert_eq!(
            String::from_utf16_lossy(&d.field_for(Part::Number(0)).text),
            "José 🌱Valerio"
        );
        assert!(d.enabled(Part::Primary));
        assert!(d.enabled(Part::Cancel));
        d.field_for_mut(Part::Number(0)).composing = true;
        assert!(!d.enabled(Part::Primary));
        d.field_for_mut(Part::Number(0)).composing = false;
        assert!(d.enabled(Part::Primary));
        assert_eq!(
            d.profile_edit
                .as_ref()
                .unwrap()
                .draft
                .as_ref()
                .unwrap()
                .baseline
                .display_name
                .as_deref(),
            Some("Alice")
        );
        let (tx, rx) = mpsc::channel();
        d.profile_edit.as_mut().unwrap().saving = Some(rx);
        assert!(!d.enabled(Part::Cancel));
        assert!(!d.enabled(Part::Number(0)));
        tx.send(Err("offline".into())).unwrap();
        assert!(
            take(&mut d.profile_edit.as_mut().unwrap().saving)
                .unwrap()
                .is_err()
        );
        assert!(d.enabled(Part::Primary));
        assert_eq!(
            String::from_utf16_lossy(&d.field_for(Part::Number(0)).text),
            "José 🌱Valerio"
        );
        d.profile_edit
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap()
            .confirm_discard = true;
        assert_eq!(d.profile_name(Part::Cancel), "Keep editing");
        assert_eq!(d.profile_name(Part::Primary), "Discard changes");
        assert!(!d.enabled(Part::Field));
        assert!(d.enabled(Part::Primary));
        assert!(d.enabled(Part::Cancel));
        d.profile_edit
            .as_mut()
            .unwrap()
            .draft
            .as_mut()
            .unwrap()
            .confirm_discard = false;
        let (picker, rx) = mpsc::channel();
        d.profile_edit.as_mut().unwrap().picking = Some(rx);
        assert!(!d.enabled(Part::Cancel));
        assert!(!d.enabled(Part::Primary));
        drop(picker);
        d.profile_edit.as_mut().unwrap().picking = None;

        d.field_for_mut(Part::Number(0)).set_text(vec![b'A' as u16]);
        d.field_for_mut(Part::Number(0)).insert(&[0xd83c]);
        d.profile_changed();
        assert_eq!(d.field_for(Part::Number(0)).text, [b'A' as u16, 0xd83c]);
        d.field_for_mut(Part::Number(0)).insert(&[0xdf31]);
        d.profile_changed();
        assert_eq!(
            String::from_utf16_lossy(&d.field_for(Part::Number(0)).text),
            "A🌱"
        );
    }
}
