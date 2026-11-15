//! Forwarding uses the shared picker and request contract. An uncertain send
//! keeps its destinations, note and idempotency keys locked until Retry.
use crate::composer::{Composer, Style};
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::list::{self, Rows};
use crate::model::Conversation;
use crate::overlay::{Dialog, Kind, Part, Rect, height, layout};
use crate::{App, EV_FOCUS, EV_SCREEN, theme::*};
use caper_client::api::Api;
use caper_client::forwarding::{Operation, Picker, Response};
use caper_client::model::{ForwardConversation, Message};
use caper_client::worker::SendFailure;
use std::cell::RefCell;
use std::sync::mpsc;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Result;

const NOTE: &str = "Shares this conversation, including future edits, reactions and replies. People in the destination can read and forward it.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Search,
    Note,
    Format(Style),
    Destination(usize),
    Message(usize),
    Link(usize, usize),
    Spoiler(usize, usize),
}

impl Control {
    pub fn identity(self) -> (usize, usize, usize) {
        match self {
            Self::Search => (0, 0, 0),
            Self::Note => (1, 0, 0),
            Self::Destination(k) => (2, k, 0),
            Self::Message(k) => (3, k, 0),
            Self::Link(i, k) => (4, i, k),
            Self::Spoiler(i, k) => (5, i, k),
            Self::Format(s) => (6, 0, s.bit() as usize),
        }
    }
}

pub fn field(p: Part) -> bool {
    matches!(p, Part::Forward(Control::Search | Control::Note))
}

pub fn focusable(p: Part) -> bool {
    matches!(p, Part::Close | Part::Primary | Part::Retry | Part::Older)
        || matches!(p, Part::Forward(c) if !matches!(c, Control::Message(_)))
}

fn body_part(p: Part) -> bool {
    !matches!(
        p,
        Part::Title
            | Part::Close
            | Part::Primary
            | Part::Forward(Control::Note | Control::Format(_))
    )
}

fn contains(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

pub fn forwardable(m: &Message) -> bool {
    !m.is_deleted() && m.forward.as_ref().is_none_or(|f| f.message.is_some())
}

enum View {
    Picker(Box<Picker>),
    Conversation {
        wrapper: Box<Message>,
        page: Option<Box<ForwardConversation>>,
        pages: usize,
        oldest: Option<String>,
    },
}

pub struct Forward {
    epoch: u64,
    api: Api,
    token: String,
    view: View,
    pending: Option<mpsc::Receiver<std::result::Result<Response, SendFailure>>>,
    pub note: Composer,
    pub focus: Part,
    originals: Vec<Original>,
}

/// egui's read-only original: rich text, author, edited marker and reaction
/// counts. Reuse the ordinary text layout, without mutable message actions.
struct Original {
    c: Conversation,
    rows: RefCell<Rows>,
    summary: String,
    /// Its attachments named for Narrator; the preview draws their cards.
    files: String,
}

impl Original {
    fn new(m: &Message) -> Self {
        let person = crate::live::person(
            &m.author.id,
            &m.author.name,
            crate::avatars::Avatar::new(
                m.author.avatar_id,
                &m.author.avatar_asset,
                &m.author.avatar_url,
            ),
        );
        let mut shown = crate::live::message(m, 0, false, None);
        shown.time = if m.edited_at.is_some() && !m.is_deleted() {
            "(edited)".into()
        } else {
            String::new()
        };
        shown.reply = None;
        shown.pinned_by = None;
        shown.thread_replies = None;
        shown.thread_context = false;
        shown.forward = None;
        shown.reactions.clear();
        // As on web and GTK, the original shows its pictures and files.
        shown.embeds.clear();
        let mut summary = String::new();
        let mut files = Vec::new();
        if !m.is_deleted() {
            if m.content.text.is_empty() && m.content.attachments.iter().any(|a| a.voice) {
                files.push("Voice message".to_owned());
            }
            for a in m.content.attachments.iter().filter(|a| !a.voice) {
                files.push(crate::files::spoken(a, &Default::default()));
            }
            summary.push_str(
                &m.reactions
                    .iter()
                    .filter(|r| !r.author_ids.is_empty())
                    .map(|r| format!("{} {}", r.emoji, r.author_ids.len()))
                    .collect::<Vec<_>>()
                    .join("  "),
            );
        }
        Self {
            c: Conversation::live(Vec::new(), 0, Vec::new(), 0, vec![person], vec![shown]),
            rows: RefCell::new(Rows::new(1)),
            summary,
            files: files.join(". "),
        }
    }

    fn measure(&self, g: &Gfx, w: f32) -> Result<f32> {
        let mut rows = self.rows.borrow_mut();
        rows.set_width(Rows::body_width(w));
        rows.ensure(g, &self.c, 0)?;
        let summary = if self.summary.is_empty() {
            0.0
        } else {
            height(&layout(
                g,
                &self.summary,
                &g.formats.dialog_note,
                w - list::BODY_X,
            )?) + 4.0
        };
        Ok(rows.height(0) + summary + 8.0)
    }

    fn name(&self) -> String {
        let text = self
            .rows
            .borrow()
            .row(0)
            .map_or_else(String::new, |r| r.spoken());
        let parts = [&text, &self.c.messages[0].time, &self.files, &self.summary];
        let rest: Vec<&str> = parts
            .iter()
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .collect();
        format!("{}. {}", self.c.members[0].name, rest.join(" "))
    }

    fn controls(&self, i: usize, r: Rect) -> Vec<(Part, Rect)> {
        let rows = self.rows.borrow();
        let Some(row) = rows.row(0) else {
            return vec![];
        };
        let (x, y) = (
            r.0 + list::BODY_X,
            r.1 + list::content_top(false) + row.text_y,
        );
        let mut parts = Vec::new();
        for (k, &(s, n, hidden)) in row.spoilers.iter().enumerate() {
            if hidden {
                parts.extend(gfx::range_rects(&row.text, s, n).into_iter().map(|r| {
                    (
                        Part::Forward(Control::Spoiler(i, k)),
                        (x + r.left, y + r.top, r.width, r.height),
                    )
                }));
            }
        }
        for (k, (s, n, _)) in row.links.iter().enumerate() {
            parts.extend(gfx::range_rects(&row.text, *s, *n).into_iter().map(|r| {
                (
                    Part::Forward(Control::Link(i, k)),
                    (x + r.left, y + r.top, r.width, r.height),
                )
            }));
        }
        parts
    }

    fn draw(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        r: Rect,
        files: Option<&mut crate::files::Ctx>,
    ) -> Result<()> {
        let mut rows = self.rows.borrow_mut();
        let mut transform = windows_numerics::Matrix3x2::default();
        unsafe {
            dc.GetTransform(&mut transform);
        }
        let mut inset = transform;
        inset.M31 += r.0 * transform.M11;
        inset.M32 += r.0 * transform.M12;
        unsafe {
            dc.SetTransform(&inset);
        }
        let result = list::draw_row(g, dc, &self.c, &mut rows, 0, r.1, None, files);
        unsafe {
            dc.SetTransform(&transform);
        }
        result?;
        if !self.summary.is_empty() {
            let l = layout(g, &self.summary, &g.formats.dialog_note, r.2 - list::BODY_X)?;
            crate::text::draw(
                dc,
                &l,
                r.0 + list::BODY_X,
                r.1 + rows.height(0),
                &g.brushes.muted,
            );
        }
        Ok(())
    }
}

impl Forward {
    fn new(epoch: u64, api: Api, token: String, view: View) -> Self {
        let focus = if matches!(view, View::Picker(_)) {
            Part::Forward(Control::Search)
        } else {
            Part::Close
        };
        let mut f = Self {
            epoch,
            api,
            token,
            view,
            pending: None,
            note: Composer::formatted(),
            focus,
            originals: Vec::new(),
        };
        f.rebuild();
        f
    }

    fn rebuild(&mut self) {
        self.originals = match &self.view {
            View::Picker(p) => vec![Original::new(
                p.source
                    .forward
                    .as_ref()
                    .and_then(|f| f.message.as_ref())
                    .unwrap_or(&p.source),
            )],
            View::Conversation {
                page: Some(page), ..
            } => page
                .root
                .iter()
                .chain(page.messages.iter())
                .map(Original::new)
                .collect(),
            _ => Vec::new(),
        };
    }

    fn operation(&self) -> Operation {
        match &self.view {
            View::Picker(_) => Operation::Destinations {
                token: self.token.clone(),
            },
            View::Conversation {
                wrapper,
                pages,
                oldest,
                ..
            } => Operation::Conversation {
                token: self.token.clone(),
                wrapper: wrapper.clone(),
                pages: *pages,
                oldest: oldest.clone(),
            },
        }
    }

    fn dispatch(&mut self, hwnd: HWND, op: Operation) {
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        let (api, window) = (self.api.clone(), hwnd.0 as isize);
        std::thread::spawn(move || {
            let _ = tx.send(op.run(&api));
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(window as _)),
                    crate::live::WM_LIVE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        });
    }
}

impl Dialog {
    pub fn forward_name(&self, p: Part) -> String {
        let Some(f) = &self.forwarding else {
            return String::new();
        };
        match p {
            Part::Title => self.title(),
            Part::Close => format!("Close {}", self.title()),
            Part::Note => NOTE.into(),
            Part::Error => self.error.clone().unwrap_or_default(),
            Part::Loading => "Loading…".into(),
            Part::Primary => match &f.view {
                View::Picker(p) => p.label(),
                _ => String::new(),
            },
            Part::Retry => if matches!(f.view, View::Picker(_)) {
                "Retry loading"
            } else {
                "Retry"
            }
            .into(),
            Part::Older => "Load older replies".into(),
            Part::Hint => match &f.view {
                View::Picker(p) if p.destinations.is_empty() => {
                    "Join a channel or start a DM to forward here."
                }
                View::Picker(_) => "No matching destinations.",
                View::Conversation { page: Some(p), .. } if p.root.is_some() => "No replies yet.",
                _ => "Original conversation unavailable.",
            }
            .into(),
            Part::Forward(Control::Search) => "Find a space, channel or DM".into(),
            Part::Forward(Control::Note) => "Add a note (optional)".into(),
            Part::Forward(Control::Format(style)) => style.name().into(),
            Part::Forward(Control::Destination(k)) => match &f.view {
                View::Picker(p) => p.destinations.get(k).map_or_else(String::new, |d| {
                    format!(
                        "{}{} · {}",
                        if d.direct { "" } else { "# " },
                        d.name,
                        d.space_name
                    )
                }),
                _ => String::new(),
            },
            Part::Forward(Control::Message(i)) => {
                f.originals.get(i).map_or_else(String::new, Original::name)
            }
            Part::Forward(Control::Link(i, k)) => f
                .originals
                .get(i)
                .and_then(|o| {
                    o.rows
                        .borrow()
                        .row(0)
                        .and_then(|r| r.links.get(k))
                        .map(|(_, _, href)| href.clone())
                })
                .unwrap_or_default(),
            Part::Forward(Control::Spoiler(_, _)) => "Reveal spoiler".into(),
            _ => String::new(),
        }
    }

    pub fn forward_checked(&self, p: Part) -> bool {
        if let (Some(f), Part::Forward(Control::Format(style))) = (&self.forwarding, p) {
            return f.note.pressed(style);
        }
        if let (Some(f), Part::Forward(Control::Destination(k))) = (&self.forwarding, p)
            && let View::Picker(p) = &f.view
        {
            return p
                .destinations
                .get(k)
                .is_some_and(|d| p.selected.contains(&d.id));
        }
        false
    }

    pub fn forward_enabled(&self, p: Part) -> bool {
        let Some(f) = &self.forwarding else {
            return false;
        };
        match p {
            Part::Close => true,
            Part::Forward(Control::Search | Control::Note | Control::Destination(_)) => {
                matches!(&f.view, View::Picker(p) if p.pending.is_none())
            }
            Part::Forward(Control::Format(_)) => {
                matches!(&f.view, View::Picker(p) if p.pending.is_none())
                    && f.note.format_height() > 0.0
            }
            Part::Primary => {
                matches!(&f.view, View::Picker(p) if p.ready(self.busy, &String::from_utf16_lossy(&f.note.text)))
            }
            Part::Retry | Part::Older => !self.busy && self.part_rect(p).is_some(),
            Part::Forward(Control::Link(_, _) | Control::Spoiler(_, _)) => true,
            _ => false,
        }
    }

    pub fn forward_bounds(&self, p: Part) -> Option<Rect> {
        self.parts
            .iter()
            .filter(|(q, _)| *q == p)
            .find_map(|(_, r)| {
                if !body_part(p) {
                    return Some(*r);
                }
                let top = r.1.max(self.body.1);
                let bottom = (r.1 + r.3).min(self.body.1 + self.body.3);
                (bottom > top).then_some((r.0, top, r.2, bottom - top))
            })
    }

    pub fn forward_hit(&self, x: f32, y: f32) -> Option<Part> {
        self.parts
            .iter()
            .find(|(p, r)| {
                focusable(*p) && contains(*r, x, y) && (!body_part(*p) || contains(self.body, x, y))
            })
            .map(|(p, _)| *p)
    }

    pub fn forward_origin(&self, p: Part) -> (f32, f32) {
        let (x, y, _, h) = self.part_rect(p).unwrap_or_default();
        let f = self.field_for(p);
        let (_, cy, ch) = f.caret_pos(f.display_caret());
        let visible = (h - 16.0).max(1.0);
        let scroll = (cy + ch - visible).clamp(0.0, (f.text_h - visible).max(0.0));
        (x + 10.0, y + 8.0 - scroll)
    }

    pub fn forward_focus(&mut self, p: Part) {
        if !self.forward_enabled(p) {
            return;
        }
        let Some(f) = &mut self.forwarding else {
            return;
        };
        f.focus = p;
        self.field.focused = p == Part::Forward(Control::Search);
        f.note.focused = matches!(p, Part::Forward(Control::Note | Control::Format(_)));
        self.field.caret_on = true;
        f.note.caret_on = true;
        if body_part(p)
            && let Some((_, y, _, h)) = self.part_rect(p)
        {
            if y < self.body.1 {
                self.scroll -= self.body.1 - y;
            } else if y + h > self.body.1 + self.body.3 {
                self.scroll += y + h - self.body.1 - self.body.3;
            }
        }
    }

    pub fn relayout_forward(&mut self, g: &Gfx, w: f32, h: f32) -> Result<()> {
        let cw = (w - 32.0).clamp(240.0, 500.0);
        let inner = cw - 32.0;
        let f = self.forwarding.as_mut().unwrap();
        let picker = matches!(f.view, View::Picker(_));
        let mut parts = Vec::new();
        let mut y = 0.0;
        if picker {
            let nh = height(&layout(g, NOTE, &g.formats.dialog_note, inner)?);
            parts.push((Part::Note, (0.0, y, inner, nh)));
            y += nh + 8.0;
        }
        for (i, o) in f.originals.iter().enumerate() {
            let oh = o.measure(g, inner)?;
            parts.push((Part::Forward(Control::Message(i)), (0.0, y, inner, oh)));
            y += oh;
            if i == 0 && matches!(&f.view, View::Conversation { page: Some(p), .. } if p.has_more) {
                parts.push((Part::Older, (0.0, y, inner, 36.0)));
                y += 44.0;
            }
        }
        if let View::Picker(p) = &f.view {
            self.field
                .relayout_with(g, &g.formats.composer, 15.0, inner - 20.0)?;
            f.note
                .relayout_with(g, &g.formats.composer, 15.0, inner - 20.0)?;
            parts.push((Part::Forward(Control::Search), (0.0, y, inner, 40.0)));
            y += 48.0;
            let query = String::from_utf16_lossy(&self.field.text);
            for d in p.visible(&query) {
                let k = p
                    .destinations
                    .iter()
                    .position(|item| item.id == d.id)
                    .unwrap();
                let label = format!(
                    "{}{} · {}",
                    if d.direct { "" } else { "# " },
                    d.name,
                    d.space_name
                );
                let rh =
                    (height(&layout(g, &label, &g.formats.button, inner - 44.0)?) + 16.0).max(44.0);
                parts.push((Part::Forward(Control::Destination(k)), (0.0, y, inner, rh)));
                y += rh + 2.0;
            }
            if !self.busy && self.error.is_none() && p.visible(&query).next().is_none() {
                parts.push((Part::Hint, (0.0, y, inner, 44.0)));
                y += 52.0;
            }
        } else if matches!(&f.view, View::Conversation { page: Some(p), .. } if p.root.is_none() || p.messages.is_empty())
        {
            parts.push((Part::Hint, (0.0, y, inner, 28.0)));
            y += 36.0;
        }
        if self.busy {
            parts.push((Part::Loading, (0.0, y, inner, 28.0)));
            y += 36.0;
        }
        if let Some(error) = &self.error {
            let eh = height(&layout(g, error, &g.formats.error, inner)?);
            parts.push((Part::Error, (0.0, y, inner, eh)));
            y += eh + 8.0;
            if !picker || matches!(&f.view, View::Picker(p) if p.session.is_none()) {
                parts.push((Part::Retry, (0.0, y, inner, 36.0)));
                y += 44.0;
            }
        }
        // Search/preview/destinations scroll; the note and send never fall below
        // a tall original or a large destination list.
        let formatting = if picker { f.note.format_height() } else { 0.0 };
        let footer = if picker { 124.0 + formatting } else { 16.0 };
        let bh = y.clamp(48.0, 420.0).min((h - 92.0 - footer).max(40.0));
        let ch = bh + 60.0 + footer;
        let (cx, cy) = ((w - cw) / 2.0, (h - ch) / 2.0);
        self.card = (cx, cy, cw, ch);
        self.body = (cx + 16.0, cy + 60.0, inner, bh);
        self.scroll = self.scroll.clamp(0.0, (y - bh).max(0.0));
        for (_, r) in &mut parts {
            r.0 += self.body.0;
            r.1 += self.body.1 - self.scroll;
        }
        let mut controls = Vec::new();
        for (p, r) in &parts {
            if let Part::Forward(Control::Message(i)) = p {
                controls.extend(f.originals[*i].controls(*i, *r));
            }
        }
        parts.extend(controls);
        parts.insert(0, (Part::Title, (cx + 16.0, cy + 16.0, inner - 40.0, 28.0)));
        if picker {
            if formatting > 0.0 {
                for (k, style) in Style::ALL.into_iter().enumerate() {
                    // Keep the toolbar within even the narrowest dialog.
                    let bw = ((inner - 12.0) / 6.0).min(36.0);
                    parts.push((
                        Part::Forward(Control::Format(style)),
                        (
                            cx + 16.0 + k as f32 * bw + if k >= 4 { 12.0 } else { 0.0 },
                            cy + ch - 116.0 - formatting,
                            bw,
                            36.0,
                        ),
                    ));
                }
            }
            parts.push((
                Part::Forward(Control::Note),
                (cx + 16.0, cy + ch - 116.0, inner, 64.0),
            ));
            parts.push((Part::Primary, (cx + 16.0, cy + ch - 44.0, inner, 36.0)));
        }
        // Last in Tab order: from a destination, Tab reaches the note and
        // Forward before the button that discards them.
        parts.push((Part::Close, (cx + cw - 44.0, cy + 12.0, 32.0, 32.0)));
        self.parts = parts;
        self.field_origin = self.forward_origin(Part::Forward(Control::Search));
        Ok(())
    }

    pub fn draw_forward(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        mut media: Option<&mut crate::files::Ctx>,
    ) -> Result<()> {
        let f = self.forwarding.as_ref().unwrap();
        for &(p, r @ (x, y, w, h)) in &self.parts {
            if matches!(
                p,
                Part::Forward(Control::Link(_, _) | Control::Spoiler(_, _))
            ) {
                continue;
            }
            if self.forward_bounds(p).is_none() {
                continue;
            }
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
                let result = (|| -> Result<()> {
                    if let Part::Forward(Control::Message(i)) = p {
                        return f.originals[i].draw(g, dc, r, media.as_deref_mut());
                    }
                    let bounds = rect(x, y, x + w, y + h);
                    let enabled = self.forward_enabled(p);
                    if let Part::Forward(Control::Format(style)) = p {
                        fill_round(
                            dc,
                            bounds,
                            RADIUS,
                            if f.note.pressed(style) {
                                &g.brushes.border
                            } else {
                                &g.brushes.raised
                            },
                        );
                        if f.focus == p {
                            stroke_round(dc, bounds, RADIUS, &g.brushes.terracotta);
                        }
                        crate::icons::draw(
                            dc,
                            style.icon(),
                            rect(
                                x + (w - 17.0) / 2.0,
                                y + 9.5,
                                x + (w + 17.0) / 2.0,
                                y + 26.5,
                            ),
                            TEXT,
                            1.0,
                        );
                        return Ok(());
                    }
                    if field(p) {
                        fill_round(dc, bounds, RADIUS, &g.brushes.composer);
                        stroke_round(
                            dc,
                            bounds,
                            RADIUS,
                            if f.focus == p && enabled {
                                &g.brushes.terracotta
                            } else {
                                &g.brushes.border
                            },
                        );
                        dc.PushAxisAlignedClip(
                            &rect(x + 1.0, y + 1.0, x + w - 1.0, y + h - 1.0),
                            D2D1_ANTIALIAS_MODE_ALIASED,
                        );
                        let (ox, oy) = self.forward_origin(p);
                        let placeholder =
                            layout(g, &self.forward_name(p), &g.formats.composer, w - 20.0)?;
                        self.field_for(p).draw_text(
                            g,
                            dc,
                            ox,
                            oy,
                            Some((&placeholder, &g.brushes.muted)),
                        );
                        dc.PopAxisAlignedClip();
                        return Ok(());
                    }
                    let action = focusable(p);
                    let primary = p == Part::Primary && enabled;
                    if action {
                        fill_round(
                            dc,
                            bounds,
                            RADIUS,
                            if primary {
                                &g.brushes.eggshell
                            } else if self.hover == Some(p) && enabled {
                                &g.brushes.hover
                            } else {
                                &g.brushes.raised
                            },
                        );
                        stroke_round(
                            dc,
                            bounds,
                            RADIUS,
                            if f.focus == p {
                                &g.brushes.terracotta
                            } else {
                                &g.brushes.border
                            },
                        );
                    }
                    let check = matches!(p, Part::Forward(Control::Destination(_)));
                    let label = if p == Part::Close {
                        "×".into()
                    } else {
                        self.forward_name(p)
                    };
                    let format = if p == Part::Title {
                        &g.formats.dialog_title
                    } else if action {
                        &g.formats.button
                    } else {
                        &g.formats.dialog_note
                    };
                    let l = layout(
                        g,
                        &label,
                        format,
                        w - if check {
                            44.0
                        } else if action {
                            16.0
                        } else {
                            0.0
                        },
                    )?;
                    let brush = if primary {
                        g.solid(BLACKOUT)
                    } else if p == Part::Error {
                        g.solid(ERROR)
                    } else if action && enabled {
                        g.solid(TEXT)
                    } else {
                        g.solid(TEXT_MUTED)
                    };
                    crate::text::draw(
                        dc,
                        &l,
                        x + if action { 8.0 } else { 0.0 },
                        y + if action { (h - height(&l)) / 2.0 } else { 0.0 },
                        &brush,
                    );
                    if check {
                        let cb = rect(
                            x + w - 28.0,
                            y + (h - 18.0) / 2.0,
                            x + w - 10.0,
                            y + (h + 18.0) / 2.0,
                        );
                        stroke_round(dc, cb, 4.0, &g.brushes.border);
                        if self.forward_checked(p) {
                            crate::icons::draw(dc, crate::icons::Icon::Check, cb, TEXT, 1.0);
                        }
                    }
                    Ok(())
                })();
                if body_part(p) {
                    dc.PopAxisAlignedClip();
                }
                result?;
            }
        }
        Ok(())
    }
}

impl App {
    pub fn open_forward(&mut self, id: &str, conversation: bool) {
        let Some(live) = &self.live else { return };
        let (Some(source), Some((epoch, api, token))) = (live.message(id), live.forward_session())
        else {
            return;
        };
        if !forwardable(&source) || (conversation && source.forward.is_none()) {
            return;
        }
        let kind = if conversation {
            Kind::ForwardConversation
        } else {
            Kind::Forward
        };
        let view = if conversation {
            View::Conversation {
                wrapper: Box::new(source),
                page: None,
                pages: 1,
                oldest: None,
            }
        } else {
            View::Picker(Box::new(Picker::new(source)))
        };
        let mut f = Forward::new(epoch, api, token, view);
        f.dispatch(self.hwnd, f.operation());
        let mut d = Dialog::new(kind, None, Composer::default());
        d.field.focused = !conversation;
        d.forwarding = Some(Box::new(f));
        d.busy = true;
        self.open_dialog(d);
        if !conversation && let Some(d) = &mut self.dialog {
            d.forward_focus(Part::Forward(Control::Search));
            self.dialog_changed();
        }
    }

    pub fn forward_dialog_activate(&mut self, p: Part) {
        let Some(d) = &mut self.dialog else { return };
        if !d.forward_enabled(p) {
            return;
        }
        d.forward_focus(p);
        let f = d.forwarding.as_mut().unwrap();
        let mut operation = None;
        match p {
            Part::Close => return self.dismiss_dialog(),
            Part::Primary => {
                if let View::Picker(p) = &mut f.view {
                    operation = p.next(&f.token, &String::from_utf16_lossy(&f.note.text), true);
                }
            }
            Part::Retry => operation = Some(f.operation()),
            Part::Older => {
                if let View::Conversation { pages, .. } = &mut f.view {
                    *pages += 1;
                }
                operation = Some(f.operation());
            }
            Part::Forward(Control::Format(style)) => {
                f.note.toggle(style);
                f.focus = Part::Forward(Control::Note);
            }
            Part::Forward(Control::Destination(k)) => {
                if let View::Picker(p) = &mut f.view
                    && let Some(item) = p.destinations.get(k)
                {
                    let id = item.id.clone();
                    p.set(&id, !p.selected.contains(&id));
                }
            }
            Part::Forward(Control::Link(i, k)) => {
                if let Some(o) = f.originals.get(i)
                    && let Some(row) = o.rows.borrow().row(0)
                    && let Some((_, _, href)) = row.links.get(k)
                {
                    crate::open_link(href);
                }
            }
            Part::Forward(Control::Spoiler(i, k)) => {
                if let Some(o) = f.originals.get_mut(i) {
                    let mut rows = o.rows.borrow_mut();
                    if let Some((s, _, true)) = rows.row(0).and_then(|r| r.spoilers.get(k)).copied()
                    {
                        o.c.messages[0].revealed.insert(list::spoiler_key(s));
                        if let Err(e) = rows.replace(&self.g, &o.c, 0) {
                            eprintln!("forward spoiler layout failed: {e:?}");
                        }
                    }
                }
            }
            _ => {}
        }
        if let Some(op) = operation {
            d.busy = true;
            d.error = None;
            f.dispatch(self.hwnd, op);
        }
        self.dialog_changed();
        self.events |= EV_FOCUS | EV_SCREEN;
    }

    /// Runs after live updates, so a wrapper revision invalidates an old page
    /// before a request can publish it. The oldest boundary survives refresh.
    pub fn poll_forward(&mut self) {
        let Some(d) = &mut self.dialog else { return };
        let Some(f) = &mut d.forwarding else { return };
        if self
            .live
            .as_ref()
            .and_then(|l| l.forward_session())
            .is_none_or(|(epoch, _, _)| epoch != f.epoch)
        {
            return self.close_dialog();
        }
        let mut changed = false;
        if let View::Conversation { wrapper, page, .. } = &mut f.view {
            let current = self.live.as_ref().and_then(|l| l.message(&wrapper.id));
            let Some(current) = current.filter(|m| forwardable(m) && m.forward.is_some()) else {
                return self.close_dialog();
            };
            if current.forward.as_ref().map(|f| &f.seq) != wrapper.forward.as_ref().map(|f| &f.seq)
            {
                **wrapper = current;
                *page = None;
                f.rebuild();
                f.dispatch(self.hwnd, f.operation());
                d.busy = true;
                d.error = None;
                changed = true;
            }
        }
        let result = f.pending.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(_) => Some(Err(SendFailure {
                status: None,
                message: "The request stopped. Try again.".into(),
                code: None,
            })),
        });
        if let Some(result) = result {
            f.pending = None;
            d.busy = false;
            changed = true;
            match result {
                Ok(Response::Destinations(values, capability)) => {
                    if let View::Picker(p) = &mut f.view {
                        p.loaded(values, capability);
                    }
                }
                Ok(Response::Sent(message)) => {
                    if let Some(live) = &mut self.live {
                        live.forward_sent(*message.clone());
                    }
                    if let View::Picker(p) = &mut f.view {
                        if p.sent(&message) {
                            self.dismiss_dialog();
                            return;
                        }
                        if let Some(op) =
                            p.next(&f.token, &String::from_utf16_lossy(&f.note.text), false)
                        {
                            f.dispatch(self.hwnd, op);
                            d.busy = true;
                        }
                    }
                }
                Ok(Response::Conversation(page)) => {
                    if let Some(source) = page.source.clone() {
                        self.close_dialog();
                        if let Some(live) = &mut self.live {
                            live.open_forward_source(source);
                        }
                        let _ = self.drain_live();
                        return;
                    }
                    if let View::Conversation {
                        page: shown,
                        oldest,
                        ..
                    } = &mut f.view
                    {
                        *oldest = page.messages.first().map(|m| m.seq.clone());
                        *shown = Some(page);
                        f.rebuild();
                    }
                }
                Err(error) => {
                    d.error = Some(match &mut f.view {
                        View::Picker(p) => p.failed(error),
                        View::Conversation { page, .. } => {
                            *page = None;
                            error.message
                        }
                    });
                    f.rebuild();
                }
            }
        }
        if changed {
            self.dialog_changed();
            self.events |= EV_SCREEN;
        }
    }

    pub fn forward_dialog_input(
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
                if let Some(p) = d.pressed.filter(|p| field(*p)) {
                    let (ox, oy) = d.forward_origin(p);
                    let f = d.field_for_mut(p);
                    f.move_to(f.hit(x - ox, y - oy), true);
                }
                d.hover = d.forward_hit(x, y);
            }
            WM_LBUTTONDOWN => {
                if let Some(p) = d.forward_hit(x, y).filter(|p| d.forward_enabled(*p)) {
                    d.forward_focus(p);
                    d.pressed = Some(p);
                    unsafe {
                        SetCapture(self.hwnd);
                    }
                    if field(p) {
                        let (ox, oy) = d.forward_origin(p);
                        let f = d.field_for_mut(p);
                        f.move_to(f.hit(x - ox, y - oy), shift);
                    }
                } else if !d.on_card(x, y) {
                    self.dismiss_dialog();
                }
            }
            WM_LBUTTONUP => {
                unsafe {
                    let _ = ReleaseCapture();
                }
                let hit = d.forward_hit(x, y);
                if let Some(p) = d.pressed.take().filter(|p| hit == Some(*p) && !field(*p)) {
                    self.forward_dialog_activate(p);
                }
            }
            WM_MOUSEWHEEL => d.scroll -= (wp.0 >> 16) as u16 as i16 as f32 / 120.0 * 60.0,
            WM_SETCURSOR => {
                let cursor = match self.pointer.and_then(|(x, y)| d.forward_hit(x, y)) {
                    Some(p) if field(p) && d.forward_enabled(p) => IDC_IBEAM,
                    Some(p) if d.forward_enabled(p) => IDC_HAND,
                    _ => IDC_ARROW,
                };
                unsafe {
                    SetCursor(LoadCursorW(None, cursor).ok());
                }
                return Some(Some(LRESULT(1)));
            }
            WM_KEYDOWN => {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                let focus = d.forwarding.as_ref().unwrap().focus;
                if field(focus)
                    && (d.field_for(focus).composing || !d.field_for(focus).comp.is_empty())
                {
                    return Some(None);
                }
                match vk {
                    VK_ESCAPE => self.dismiss_dialog(),
                    VK_TAB => {
                        let mut choices: Vec<_> =
                            d.parts().filter(|p| d.forward_enabled(*p)).collect();
                        choices.dedup();
                        if !choices.is_empty() {
                            let k = choices.iter().position(|p| *p == focus).unwrap_or(0);
                            d.forward_focus(
                                choices[(k + if shift { choices.len() - 1 } else { 1 })
                                    % choices.len()],
                            );
                        }
                    }
                    VK_RETURN if ctrl => self.forward_dialog_activate(Part::Primary),
                    VK_RETURN
                        if focus == Part::Forward(Control::Note) && d.forward_enabled(focus) =>
                    {
                        d.field_for_mut(focus).insert(&['\n' as u16]);
                    }
                    VK_RETURN | VK_SPACE if !field(focus) => self.forward_dialog_activate(focus),
                    VK_PRIOR | VK_NEXT if !field(focus) => {
                        d.scroll += if vk == VK_PRIOR { -d.body.3 } else { d.body.3 }
                    }
                    _ if field(focus) && d.forward_enabled(focus) => {
                        let f = d.field_for_mut(focus);
                        if ctrl && vk == VIRTUAL_KEY(b'C' as u16) {
                            let copied = f.copied();
                            if !copied.is_empty() {
                                crate::set_clipboard(self.hwnd, &copied);
                            }
                        } else {
                            f.edit_key(
                                self.hwnd,
                                vk,
                                ctrl,
                                shift,
                                focus == Part::Forward(Control::Note),
                            );
                        }
                    }
                    _ => {}
                }
            }
            WM_CHAR => {
                let p = d.forwarding.as_ref().unwrap().focus;
                let c = wp.0 as u16;
                if !ctrl && c >= 0x20 && c != 0x7f && field(p) && d.forward_enabled(p) {
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
    fn an_unconfirmed_forward_locks_the_note_and_destinations_but_allows_retry() {
        let source = serde_json::from_value(serde_json::json!({
            "id": "source", "channelId": "private", "seq": "7",
            "clientMessageId": "source-key", "createdAt": "2026-10-06T12:00:00Z",
            "author": {"id": "author", "name": "Alex", "isGuest": false},
            "content": {"version": 1, "type": "text", "text": "original"}
        }))
        .unwrap();
        let mut picker = Picker::new(source);
        picker.loaded(
            vec![caper_client::model::ForwardDestination {
                id: "destination".into(),
                name: "general".into(),
                space_name: "Workshop".into(),
                direct: false,
            }],
            "capability".into(),
        );
        picker.set("destination", true);
        let mut d = Dialog::new(Kind::Forward, None, Composer::default());
        d.forwarding = Some(Box::new(Forward::new(
            1,
            Api::new("http://127.0.0.1:9").unwrap(),
            "token".into(),
            View::Picker(Box::new(picker)),
        )));
        let editable = [Control::Search, Control::Note, Control::Destination(0)];
        assert!(
            editable
                .iter()
                .all(|c| d.forward_enabled(Part::Forward(*c)))
        );
        assert!(d.forward_enabled(Part::Primary));
        let View::Picker(p) = &mut d.forwarding.as_mut().unwrap().view else {
            unreachable!()
        };
        assert!(p.next("token", "note", true).is_some());
        p.failed(SendFailure {
            status: None,
            code: None,
            message: "Not confirmed".into(),
        });
        assert!(
            editable
                .iter()
                .all(|c| !d.forward_enabled(Part::Forward(*c)))
        );
        assert!(
            d.forward_enabled(Part::Primary),
            "Retry stays available without unlocking intent"
        );
        assert_eq!(d.forward_name(Part::Primary), "Retry forwards (1)");
        d.busy = true;
        assert!(!d.forward_enabled(Part::Primary), "no parallel send");
        assert!(d.forward_enabled(Part::Close));
        d.busy = false;
        let View::Picker(p) = &mut d.forwarding.as_mut().unwrap().view else {
            unreachable!()
        };
        p.failed(SendFailure {
            status: Some(403),
            code: None,
            message: "No access".into(),
        });
        assert!(
            editable
                .iter()
                .all(|c| d.forward_enabled(Part::Forward(*c))),
            "a definitive rejection unlocks the remaining intent"
        );
    }
}
