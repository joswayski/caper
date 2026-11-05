//! The signed-out screens, as the egui client draws them: "Loading your
//! spaces…" while a stored session is restored, then the email step
//! ("Welcome to Caper") and the emailed-code step ("Check your email."),
//! with the API's resend limits and attempt counts.
use crate::composer::Composer;
use crate::gfx::{Gfx, fill_round, rect, stroke_round, wide};
use crate::theme::*;
use caper_client::sign_in::{MAX_RESENDS, clean_sign_in_code, resend_countdown, resend_wait};
use std::time::{Duration, Instant};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::*;
use windows_numerics::{Matrix3x2, Vector2};

const MAX_W: f32 = 440.0;
const MARGIN: f32 = 20.0;
const TOP: f32 = 100.0;
pub const WORDMARK: (f32, f32) = (132.0, 35.0);
const FIELD_H: f32 = 52.0;
const FIELD_INSET: f32 = 14.0;
const ACTION_H: f32 = 58.0;
const SPINNER: f32 = 16.0;

/// The interactive parts of the form.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    /// The heading (not interactive).
    Title,
    Field,
    /// The error or status line under the field (not interactive).
    Message,
    Primary,
    OtherEmail,
    Resend,
}

/// What the form asks the worker for.
pub enum Request {
    Code(String),
    Verify { challenge: String, code: String },
}

pub type Rect = (f32, f32, f32, f32);

pub struct SignIn {
    /// Restoring a stored session: only the wordmark and a spinner.
    pub restoring: bool,
    pub email: Composer,
    pub code: Composer,
    pub challenge: Option<String>,
    pub loading: bool,
    resending: bool,
    pub error: Option<String>,
    status: Option<&'static str>,
    attempts_remaining: Option<u64>,
    resends: u8,
    sent_at: Option<Instant>,
    pub hover: Option<Part>,
    pub pressed: Option<Part>,
    /// Hit rectangles of the last draw, in client DIPs.
    parts: Vec<(Part, Rect, bool)>,
    /// Client DIP origin of the field's text layout (after horizontal scroll).
    pub field_origin: (f32, f32),
    field_scroll: f32,
    pub spin: f32,
    /// Signed in, but the account could not be opened (shown while restoring).
    pub failure: Option<String>,
    wordmark: Option<ID2D1SvgDocument>,
}

fn contains(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

fn layout(
    g: &Gfx,
    text: &str,
    format: &IDWriteTextFormat,
    width: f32,
) -> Result<IDWriteTextLayout> {
    g.layout(&wide(text), format, width)
}

fn height(l: &IDWriteTextLayout) -> f32 {
    crate::gfx::metrics(l).height
}

fn text_width(l: &IDWriteTextLayout) -> f32 {
    crate::gfx::metrics(l).widthIncludingTrailingWhitespace
}

impl SignIn {
    pub fn new() -> Self {
        Self {
            restoring: true,
            email: Composer::default(),
            code: Composer::default(),
            challenge: None,
            loading: false,
            resending: false,
            error: None,
            status: None,
            attempts_remaining: None,
            resends: 0,
            sent_at: None,
            hover: None,
            pressed: None,
            parts: Vec::new(),
            field_origin: (0.0, 0.0),
            field_scroll: 0.0,
            spin: 0.0,
            failure: None,
            wordmark: None,
        }
    }

    pub fn field(&self) -> &Composer {
        if self.challenge.is_some() {
            &self.code
        } else {
            &self.email
        }
    }

    pub fn field_mut(&mut self) -> &mut Composer {
        if self.challenge.is_some() {
            &mut self.code
        } else {
            &mut self.email
        }
    }

    fn email_text(&self) -> String {
        String::from_utf16_lossy(&self.email.text).trim().to_owned()
    }

    fn code_text(&self) -> String {
        String::from_utf16_lossy(&self.code.text)
    }

    fn exhausted(&self) -> bool {
        self.attempts_remaining == Some(0)
    }

    /// The field takes edits: not while a request runs (it stays focused,
    /// read-only, as on web) nor once the code's attempts are used up.
    pub fn editable(&self) -> bool {
        !self.restoring && !self.loading && !(self.challenge.is_some() && self.exhausted())
    }

    fn primary_enabled(&self) -> bool {
        if self.loading {
            return false;
        }
        match &self.challenge {
            None => self.email_text().contains('@'),
            Some(_) if self.exhausted() => true,
            Some(_) => self.code_text().len() == 6,
        }
    }

    fn resends_left(&self) -> bool {
        self.resends < MAX_RESENDS
    }

    /// Time until "Resend code" is available, while it counts down.
    pub fn resend_wait(&self) -> Option<Duration> {
        if self.challenge.is_none() || self.exhausted() || !self.resends_left() || self.loading {
            return None;
        }
        self.sent_at.and_then(|sent| resend_wait(sent.elapsed()))
    }

    /// When the countdown's label next changes.
    pub fn next_tick(&self) -> Option<Duration> {
        self.resend_wait().map(|wait| resend_countdown(wait).1)
    }

    /// After the field's text changed: clean a code, clear the error.
    pub fn edited(&mut self) {
        if self.challenge.is_some() {
            let mut code = self.code_text();
            let before = code.clone();
            clean_sign_in_code(&mut code);
            if code != before {
                self.code.text = wide(&code);
                self.code.caret = self.code.caret.min(self.code.text.len());
                self.code.anchor = self.code.caret;
            }
        }
        if !self.loading {
            self.error = None;
        }
    }

    /// Enter in the field: the primary action when it is enabled.
    pub fn submit(&mut self) -> Option<Request> {
        if self.primary_enabled() && !self.exhausted() {
            self.activate(Part::Primary)
        } else {
            None
        }
    }

    pub fn activate(&mut self, part: Part) -> Option<Request> {
        if self.restoring || self.loading {
            return None;
        }
        match part {
            Part::Title | Part::Message | Part::Field => None,
            Part::Primary if !self.primary_enabled() => None,
            Part::Primary => match self.challenge.clone() {
                None => {
                    self.loading = true;
                    self.error = None;
                    Some(Request::Code(self.email_text()))
                }
                Some(_) if self.exhausted() => {
                    if !self.resends_left() {
                        return None;
                    }
                    self.code = Composer::default();
                    self.resend()
                }
                Some(challenge) => {
                    self.loading = true;
                    self.error = None;
                    self.status = None;
                    Some(Request::Verify {
                        challenge,
                        code: self.code_text(),
                    })
                }
            },
            Part::OtherEmail => {
                self.challenge = None;
                self.code = Composer::default();
                self.error = None;
                self.attempts_remaining = None;
                self.sent_at = None;
                self.resends = 0;
                self.resending = false;
                self.status = None;
                None
            }
            Part::Resend if self.resend_wait().is_some() || !self.resends_left() => None,
            Part::Resend => self.resend(),
        }
    }

    /// Requests another code for the same email; `code_requested` counts it.
    fn resend(&mut self) -> Option<Request> {
        self.loading = true;
        self.resending = true;
        self.error = None;
        self.status = None;
        Some(Request::Code(self.email_text()))
    }

    pub fn code_requested(&mut self, result: std::result::Result<String, String>) {
        self.loading = false;
        let resend = std::mem::take(&mut self.resending);
        match result {
            Ok(challenge) => {
                self.challenge = Some(challenge);
                self.code = Composer::default();
                self.attempts_remaining = None;
                self.sent_at = Some(Instant::now());
                if resend {
                    self.resends = self.resends.saturating_add(1);
                    self.status = Some("We sent a new code. Earlier codes no longer work.");
                }
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub fn verify_failed(&mut self, error: String, attempts_remaining: Option<u64>) {
        self.loading = false;
        self.attempts_remaining = attempts_remaining;
        self.error = Some(error);
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<Part> {
        self.parts
            .iter()
            .find(|(_, r, enabled)| *enabled && contains(*r, x, y))
            .map(|(p, _, _)| *p)
    }

    /// The parts on screen, in reading order.
    pub fn parts(&self) -> impl Iterator<Item = Part> + '_ {
        self.parts.iter().map(|(p, _, _)| *p)
    }

    pub fn part_rect(&self, part: Part) -> Option<(Rect, bool)> {
        self.parts
            .iter()
            .find(|(p, _, _)| *p == part)
            .map(|(_, r, e)| (*r, *e))
    }

    /// The screen's text for UI Automation names.
    pub fn title(&self) -> &'static str {
        if self.challenge.is_some() {
            "Check your email."
        } else {
            "Welcome to Caper"
        }
    }

    pub fn field_name(&self) -> &'static str {
        if self.challenge.is_some() {
            "Sign-in code"
        } else {
            "Email address"
        }
    }

    pub fn primary_label(&self) -> &'static str {
        match (&self.challenge, self.loading) {
            (None, true) => "Sending…",
            (None, false) => "Email me a code",
            (Some(_), true) if self.resending || self.exhausted() => "Sending…",
            (Some(_), true) => "Checking…",
            (Some(_), false) if self.exhausted() => "Email me a new code",
            (Some(_), false) => "Continue",
        }
    }

    pub fn resend_label(&self) -> String {
        if self.loading && self.resending {
            "Sending…".to_owned()
        } else if let Some(wait) = self.resend_wait() {
            resend_countdown(wait).0
        } else {
            "Resend code".to_owned()
        }
    }

    pub fn message(&self) -> Option<&str> {
        if self.restoring {
            return Some(self.failure.as_deref().unwrap_or("Loading your spaces…"));
        }
        self.error.as_deref().or(self.status)
    }

    /// Lays out the field's text (single line, 16px).
    pub fn relayout_field(&mut self, g: &Gfx, window_w: f32) -> Result<()> {
        let w = self.column(window_w).1;
        let inner = w - 2.0 * FIELD_INSET;
        self.field_mut()
            .relayout_with(g, &g.formats.field, 16.0, inner)?;
        Ok(())
    }

    /// Column x and width for a window `w` DIPs wide.
    fn column(&self, w: f32) -> (f32, f32) {
        let width = (w - 2.0 * MARGIN).clamp(1.0, MAX_W);
        ((w - width) / 2.0, width)
    }

    /// Draws the whole screen into a `w`×`h` DIP surface and records the
    /// hit rectangles.
    pub fn draw(&mut self, g: &Gfx, dc: &ID2D1DeviceContext, w: f32, h: f32) -> Result<()> {
        unsafe {
            dc.Clear(Some(&crate::gfx::color(BLACKOUT, 1.0)));
        }
        if self.wordmark.is_none() {
            self.wordmark = Some(g.wordmark(WORDMARK.0, WORDMARK.1)?);
        }
        self.parts.clear();
        if self.restoring {
            return self.draw_restoring(g, dc, w, h);
        }
        self.relayout_field(g, w)?;
        let (x0, width) = self.column(w);
        let mut y = TOP;
        self.draw_wordmark(dc, x0, y)?;
        y += WORDMARK.1 + 58.0;
        let code_step = self.challenge.is_some();
        let title = layout(g, self.title(), &g.formats.title, width)?;
        self.parts
            .push((Part::Title, (x0, y, width, height(&title)), false));
        y = self.text(dc, &title, x0, y, &g.brushes.text) + 4.0;
        let subtitle = if code_step {
            format!(
                "Enter the six-character code sent to {}. It expires in 10 minutes.",
                self.email_text()
            )
        } else {
            "Use your email to create an account or log back in.".to_owned()
        };
        let subtitle = layout(g, &subtitle, &g.formats.body, width)?;
        y = self.text(dc, &subtitle, x0, y, &g.brushes.muted) + 20.0;
        let label = layout(g, self.field_name(), &g.formats.label, width)?;
        y = self.text(dc, &label, x0, y, &g.brushes.text) + 8.0;
        let field = (x0, y, width, FIELD_H);
        let editable_field = !(code_step && self.exhausted());
        self.draw_field(g, dc, field, editable_field)?;
        self.parts.push((Part::Field, field, editable_field));
        y += FIELD_H;
        let before = y + 12.0;
        if let Some(error) = &self.error {
            y = self.error_frame(g, dc, error, x0, y + 12.0, width)?;
        } else if let Some(status) = self.status.filter(|_| code_step) {
            let l = layout(g, status, &g.formats.note, width)?;
            y = self.text(dc, &l, x0, y + 12.0, &g.brushes.muted);
        }
        if y > before {
            self.parts
                .push((Part::Message, (x0, before, width, y - before), false));
        }
        if code_step && self.attempts_remaining == Some(1) {
            let l = layout(
                g,
                "One attempt left. Check the code carefully.",
                &g.formats.label,
                width,
            )?;
            y = self.text(dc, &l, x0, y + 8.0, &g.brushes.text);
        }
        y += 12.0;
        let show_primary = !(code_step && self.exhausted() && !self.resends_left());
        if show_primary {
            let idle = match (&self.challenge, self.exhausted()) {
                (None, _) => "Email me a code",
                (Some(_), true) => "Email me a new code",
                (Some(_), false) => "Continue",
            };
            let idle_w = width_of(g, idle)? + 84.0;
            let bw = if code_step {
                idle_w.max(200.0).min(width)
            } else {
                idle_w.min(width)
            };
            let r = (x0 + width - bw, y, bw, ACTION_H);
            let enabled = self.primary_enabled();
            self.draw_action(g, dc, r, enabled)?;
            self.parts.push((Part::Primary, r, enabled));
            y += ACTION_H;
        }
        if code_step {
            y += 10.0;
            let mut x = x0;
            let other = layout(g, "Use a different email", &g.formats.link_small, width)?;
            let r = (x, y, text_width(&other), height(&other));
            self.link(g, dc, &other, r, !self.loading, Part::OtherEmail)?;
            self.parts.push((Part::OtherEmail, r, !self.loading));
            x += r.2 + 16.0;
            if !self.exhausted() && self.resends_left() {
                let label = self.resend_label();
                let l = layout(g, &label, &g.formats.link_small, width)?;
                let enabled = !self.loading && self.resend_wait().is_none();
                let r = (x, y, text_width(&l), height(&l));
                self.link(g, dc, &l, r, enabled, Part::Resend)?;
                self.parts.push((Part::Resend, r, enabled));
            }
            y += r.3;
            if !self.resends_left() {
                let l = layout(
                    g,
                    "Still nothing? Check your spam folder, or try again in 15 minutes.",
                    &g.formats.link_small,
                    width,
                )?;
                unsafe { l.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)? };
                self.text(dc, &l, x0, y + 8.0, &g.brushes.muted);
            }
        }
        Ok(())
    }

    fn draw_restoring(&mut self, g: &Gfx, dc: &ID2D1DeviceContext, w: f32, h: f32) -> Result<()> {
        let text = self.failure.as_deref().unwrap_or("Loading your spaces…");
        let note = layout(g, text, &g.formats.link_small, (w - 2.0 * MARGIN).max(1.0))?;
        let all = DWRITE_TEXT_RANGE {
            startPosition: 0,
            length: u32::MAX,
        };
        unsafe { note.SetFontSize(12.0, all)? };
        let note_h = height(&note);
        let total = WORDMARK.1 + 14.0 + SPINNER + 14.0 + note_h;
        let mut y = ((h - total) / 2.0).max(0.0);
        self.draw_wordmark(dc, (w - WORDMARK.0) / 2.0, y)?;
        y += WORDMARK.1 + 14.0;
        if self.failure.is_none() {
            self.draw_spinner(g, dc, w / 2.0, y + SPINNER / 2.0)?;
        }
        y += SPINNER + 14.0;
        let (nw, nh) = (text_width(&note), height(&note));
        let x = (w - nw) / 2.0;
        self.parts.push((Part::Message, (x, y, nw, nh), false));
        let brush = if self.failure.is_some() {
            &g.brushes.error
        } else {
            &g.brushes.muted
        };
        self.text(dc, &note, x, y, brush);
        Ok(())
    }

    /// egui's spinner: a 3/4 arc turning once a second or so.
    fn draw_spinner(&self, g: &Gfx, dc: &ID2D1DeviceContext, cx: f32, cy: f32) -> Result<()> {
        let r = SPINNER / 2.0 - 1.0;
        unsafe {
            let factory = g.d2d.GetFactory()?;
            let path = factory.CreatePathGeometry()?;
            let sink = path.Open()?;
            let start = self.spin;
            let point = |a: f32| Vector2 {
                X: cx + r * a.cos(),
                Y: cy + r * a.sin(),
            };
            sink.BeginFigure(point(start), D2D1_FIGURE_BEGIN_HOLLOW);
            let end = start + std::f32::consts::PI * 1.5;
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: point(end),
                size: D2D_SIZE_F {
                    width: r,
                    height: r,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: D2D1_ARC_SIZE_LARGE,
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            sink.Close()?;
            dc.DrawGeometry(&path, &g.brushes.muted, 1.5, None);
        }
        Ok(())
    }

    fn draw_wordmark(&self, dc: &ID2D1DeviceContext, x: f32, y: f32) -> Result<()> {
        let Some(doc) = &self.wordmark else {
            return Ok(());
        };
        unsafe {
            let dc5: ID2D1DeviceContext5 = dc.cast()?;
            let mut before = Matrix3x2::default();
            dc.GetTransform(&mut before);
            dc.SetTransform(&(Matrix3x2::translation(x, y) * before));
            dc5.DrawSvgDocument(doc);
            dc.SetTransform(&before);
        }
        Ok(())
    }

    /// Draws `l` at (x, y); returns the y below it.
    fn text(
        &self,
        dc: &ID2D1DeviceContext,
        l: &IDWriteTextLayout,
        x: f32,
        y: f32,
        brush: &ID2D1SolidColorBrush,
    ) -> f32 {
        crate::text::draw(dc, l, x, y, brush);
        y + height(l)
    }

    fn draw_field(
        &mut self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        r: Rect,
        enabled: bool,
    ) -> Result<()> {
        let focused = self.field().focused;
        let hovered = self.hover == Some(Part::Field);
        let stroke = if focused {
            g.solid(TERRACOTTA)
        } else if hovered && enabled {
            g.solid(TERRACOTTA_BRIGHT)
        } else {
            g.brushes.border.clone()
        };
        let (x, y, w, h) = r;
        let inner = w - 2.0 * FIELD_INSET;
        let field = self.field();
        let (cx, _, _) = field.caret_pos(field.display_caret());
        let mut scroll = self.field_scroll;
        if cx - scroll > inner - 2.0 {
            scroll = cx - inner + 2.0;
        } else if cx < scroll {
            scroll = cx;
        }
        let text_w = field.layout.as_ref().map_or(0.0, text_width);
        scroll = scroll.clamp(0.0, (text_w - inner + 2.0).max(0.0));
        self.field_scroll = scroll;
        let oy = y + (h - self.field().text_h) / 2.0;
        let ox = x + FIELD_INSET - scroll;
        self.field_origin = (ox, oy);
        let field = self.field();
        let placeholder = if self.challenge.is_some() {
            "e.g. ABC234"
        } else {
            "you@example.com"
        };
        let hint = if field.text.is_empty() && field.comp.is_empty() {
            Some(layout(g, placeholder, &g.formats.field, inner)?)
        } else {
            None
        };
        unsafe {
            let body = rect(x, y, x + w, y + h);
            fill_round(dc, body, 7.0, &g.brushes.composer);
            stroke_round(dc, body, 7.0, &stroke);
            dc.PushAxisAlignedClip(
                &rect(
                    x + FIELD_INSET - 1.0,
                    y + 1.0,
                    x + w - FIELD_INSET + 2.0,
                    y + h - 1.0,
                ),
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
            field.draw_text(g, dc, ox, oy, hint.as_ref().map(|l| (l, &g.brushes.hint)));
            dc.PopAxisAlignedClip();
        }
        Ok(())
    }

    fn error_frame(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        error: &str,
        x: f32,
        y: f32,
        width: f32,
    ) -> Result<f32> {
        let fw = width.min(414.0);
        let l = layout(g, error, &g.formats.error, fw - 24.0)?;
        let fh = height(&l) + 28.0;
        stroke_round(dc, rect(x, y, x + fw, y + fh), 6.0, &g.brushes.terracotta);
        crate::text::draw(dc, &l, x + 12.0, y + 14.0, &g.brushes.error);
        Ok(y + fh)
    }

    fn draw_action(&self, g: &Gfx, dc: &ID2D1DeviceContext, r: Rect, enabled: bool) -> Result<()> {
        let (fill, border, label) = if !enabled {
            (SURFACE_RAISED, BORDER, TEXT_MUTED)
        } else if self.pressed == Some(Part::Primary) && self.hover == Some(Part::Primary) {
            (ACTION_PRESS, ACTION_PRESS, BLACKOUT)
        } else if self.hover == Some(Part::Primary) {
            (ACTION_HOVER, ACTION_HOVER, BLACKOUT)
        } else {
            (ACTION, ACTION, BLACKOUT)
        };
        let (x, y, w, h) = r;
        let label_brush = g.solid(label);
        unsafe {
            let body = rect(x, y, x + w, y + h);
            fill_round(dc, body, 7.0, &g.solid(fill));
            stroke_round(dc, body, 7.0, &g.solid(border));
            let l = layout(g, self.primary_label(), &g.formats.action, w)?;
            crate::text::draw(dc, &l, x + 21.0, y + (h - height(&l)) / 2.0, &label_brush);
            let (cx, cy) = (x + w - 22.0, y + h / 2.0);
            let p = |px: f32, py: f32| Vector2 { X: px, Y: py };
            for (a, b) in [
                (p(cx - 9.0, cy), p(cx + 7.0, cy)),
                (p(cx + 2.0, cy - 5.0), p(cx + 7.0, cy)),
                (p(cx + 2.0, cy + 5.0), p(cx + 7.0, cy)),
            ] {
                dc.DrawLine(a, b, &label_brush, 1.5, None);
            }
        }
        Ok(())
    }

    fn link(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        l: &IDWriteTextLayout,
        r: Rect,
        enabled: bool,
        part: Part,
    ) -> Result<()> {
        let brush = if !enabled {
            unsafe { dc.CreateSolidColorBrush(&crate::gfx::color(TEXT_MUTED, 0.5), None)? }
        } else if self.hover == Some(part) {
            g.brushes.text.clone()
        } else {
            g.brushes.muted.clone()
        };
        crate::text::draw(dc, l, r.0, r.1, &brush);
        Ok(())
    }
}

fn width_of(g: &Gfx, text: &str) -> Result<f32> {
    Ok(text_width(&layout(g, text, &g.formats.action, 1e6)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use caper_client::sign_in::RESEND_COOLDOWN;

    fn challenged() -> SignIn {
        let mut s = SignIn::new();
        s.restoring = false;
        s.email.text = wide("me@example.com");
        assert!(
            matches!(s.activate(Part::Primary), Some(Request::Code(e)) if e == "me@example.com")
        );
        s.code_requested(Ok("challenge".into()));
        s
    }

    #[test]
    fn resends_are_limited_and_counted_only_when_a_code_arrives() {
        let mut s = challenged();
        // The cooldown blocks the first resend.
        assert!(s.activate(Part::Resend).is_none());
        for sent in 1..=MAX_RESENDS {
            s.sent_at = Some(Instant::now() - RESEND_COOLDOWN);
            assert!(matches!(s.activate(Part::Resend), Some(Request::Code(_))));
            assert_eq!(s.primary_label(), "Sending…");
            s.code_requested(Ok(format!("challenge-{sent}")));
            assert_eq!(s.resends, sent);
            assert_eq!(
                s.challenge.as_deref(),
                Some(format!("challenge-{sent}").as_str())
            );
        }
        s.sent_at = Some(Instant::now() - RESEND_COOLDOWN);
        assert!(s.activate(Part::Resend).is_none(), "no third resend");
    }

    #[test]
    fn exhausted_code_offers_a_new_code_and_blocks_verify() {
        let mut s = challenged();
        s.code.text = wide("ABC234");
        assert!(matches!(s.submit(), Some(Request::Verify { code, .. }) if code == "ABC234"));
        s.verify_failed("Wrong code".into(), Some(0));
        assert!(!s.editable());
        assert!(
            s.submit().is_none(),
            "Enter cannot verify an exhausted code"
        );
        assert_eq!(s.primary_label(), "Email me a new code");
        assert!(matches!(s.activate(Part::Primary), Some(Request::Code(_))));
        assert!(s.code.text.is_empty());
    }

    #[test]
    fn edits_clean_the_code_and_clear_the_error() {
        let mut s = challenged();
        s.error = Some("Wrong code".into());
        s.code.text = wide(" abc-23i4o5");
        s.edited();
        assert_eq!(s.code_text(), "ABC234");
        assert!(s.error.is_none());
    }
}
