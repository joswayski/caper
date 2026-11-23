//! Explorer file drags: OLE supplies enter/over/leave feedback; the legacy
//! WM_DROPFILES route uses the same pane hit test. The overlay takes no input.
mod motion;

use crate::{App, gfx, theme::*, uploads, with_app};
use motion::{Plan, Pose};
use std::{cell::Cell, path::PathBuf, time::Instant};
use windows::Win32::{
    Foundation::*,
    Graphics::Direct2D::*,
    Graphics::Gdi::ScreenToClient,
    System::{Com::*, Ole::*, SystemServices::MODIFIERKEYS_FLAGS},
    UI::{Shell::HDROP, WindowsAndMessaging::*},
};
use windows::core::*;
use windows_numerics::Matrix3x2;

pub const TIMER_DROP: usize = 42;
type Rect = crate::overlay::Rect;

macro_rules! card {
    ($glyph:literal) => { concat!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="54" height="66" viewBox="0 0 54 66"><path d="M8 .5h31.5l14 14V58a7.5 7.5 0 0 1-7.5 7.5H8A7.5 7.5 0 0 1 .5 58V8A7.5 7.5 0 0 1 8 .5Z" fill="#1b1b1f" stroke="#29292f"/><path d="M39.5 .5v8a6 6 0 0 0 6 6h8" fill="#121214" stroke="#29292f"/><g transform="translate(17 23) scale(0.8333)" fill="none" stroke="#9a99a2" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">"##,
        $glyph, "</g></svg>"
    ) };
}
const CARDS: [&str; 3] = [
    card!(
        r#"<rect width="18" height="18" x="3" y="3" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>"#
    ),
    card!(
        r#"<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4M10 9H8M16 13H8M16 17H8"/>"#
    ),
    card!(
        r#"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M7 3v18M3 7.5h4M3 12h18M3 16.5h4M17 3v18M17 7.5h4M17 16.5h4"/>"#
    ),
];

pub struct Open {
    point: POINT,
    root: Option<String>,
    since: Instant,
    plan: Plan,
    cards: [Option<ID2D1Bitmap1>; 3],
    character: Option<ID2D1Bitmap1>,
}

fn bitmap(dc: &ID2D1DeviceContext, svg: &[u8], w: u32, h: u32) -> Option<ID2D1Bitmap1> {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w * 2, h * 2)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(
            (w * 2) as f32 / tree.size().width(),
            (h * 2) as f32 / tree.size().height(),
        ),
        &mut pixmap.as_mut(),
    );
    let mut pixels = pixmap.take();
    for p in pixels.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
    }
    gfx::bitmap_on(dc, w * 2, h * 2, &pixels).ok()
}

impl App {
    /// Only an uncovered, writable conversation pane is a destination.
    fn drop_target_at(&self, point: POINT) -> Option<(Option<String>, Rect)> {
        if self.browsing()
            || self.narrow.menu
            || (self.is_narrow() && self.members_w() > 0.0)
            || self.signin.is_some()
            || self.settings.is_some()
            || self.dialog.is_some()
            || self.viewer.is_some()
            || self.attachment_editor.is_some()
            || self.pins.is_some()
            || self.profile.is_some()
            || self.menu.is_some()
            || self.picker.is_some()
            || self.gifs.is_some()
            || self.dm_menu.is_some()
            || self.account_menu.is_some()
            || !self.live.as_ref().is_some_and(|l| l.can_attach())
            || self.dm_bar().is_some()
        {
            return None;
        }
        let (x, y) = (point.x as f32 / self.scale, point.y as f32 / self.scale);
        if y < HEADER_H || y >= self.h || self.on_member_overlay(x, y) {
            return None;
        }
        if let Some(t) = &self.thread
            && self.thread_shown()
            && x >= self.thread_x()
            && x < self.thread_x() + self.thread_w()
        {
            return Some((
                Some(t.root.clone()),
                (
                    self.thread_x(),
                    HEADER_H,
                    self.thread_w(),
                    self.h - HEADER_H,
                ),
            ));
        }
        if self.chat_shown()
            && y >= self.history_y()
            && x >= self.list_x()
            && x < self.list_x() + self.chat_width()
            && self.rec().is_none()
        {
            return Some((
                None,
                (
                    self.list_x(),
                    self.history_y(),
                    self.chat_width(),
                    self.h - self.history_y(),
                ),
            ));
        }
        None
    }

    pub fn drag_over(&mut self, point: POINT, files: bool) -> bool {
        let target = files.then(|| self.drop_target_at(point)).flatten();
        let Some((root, _)) = target else {
            self.close_drop();
            return false;
        };
        if self.drop_overlay.as_ref().is_none_or(|o| o.root != root) {
            let random = uuid::Uuid::new_v4().as_u128();
            let art = caper_client::avatar_images::BRANDING;
            self.drop_overlay = Some(Open {
                point,
                root: root.clone(),
                since: Instant::now(),
                plan: Plan::new((random >> 64) as u32),
                cards: CARDS.map(|svg| bitmap(&self.g.dc, svg.as_bytes(), 54, 66)),
                character: bitmap(
                    &self.g.dc,
                    art[(random % art.len() as u128) as usize],
                    76,
                    76,
                ),
            });
        } else if let Some(open) = &mut self.drop_overlay {
            open.point = point;
        }
        if crate::files::motion() {
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_DROP, 16, None);
            }
        }
        self.need = true;
        self.live.as_ref().is_some_and(|l| {
            l.composer_drafts(root.as_deref()).len() < caper_client::uploads::MAX_ATTACHMENTS
        })
    }

    pub fn close_drop(&mut self) {
        if self.drop_overlay.take().is_some() {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_DROP);
            }
            self.need = true;
        }
    }

    pub fn drop_paths(&mut self, point: POINT, paths: Vec<PathBuf>) -> bool {
        self.close_drop();
        let Some((root, _)) = self.drop_target_at(point) else {
            return false;
        };
        if paths.is_empty() {
            return false;
        }
        let Some(live) = &mut self.live else {
            return false;
        };
        if live.composer_drafts(root.as_deref()).len() >= caper_client::uploads::MAX_ATTACHMENTS {
            return false;
        }
        let error = live.add_files(
            paths.into_iter().map(|p| (p, None)).collect(),
            root.as_deref(),
        );
        if root.is_some() {
            if let Some(t) = &mut self.thread {
                t.attach_error = error;
            }
            self.set_thread_focus(true);
        } else {
            self.attach_error = error;
            self.set_thread_focus(false);
            self.set_composer_focus(true);
        }
        self.drafts_changed();
        true
    }

    pub fn draw_drop(&mut self) -> Result<()> {
        let target = self
            .drop_overlay
            .as_ref()
            .and_then(|o| self.drop_target_at(o.point));
        let Some((root, (x, y, w, h))) = target else {
            self.close_drop();
            return self.drop_p.offset(-10_000.0, 0.0);
        };
        let Some(open) = &self.drop_overlay else {
            return Ok(());
        };
        let reduced = !crate::files::motion();
        if reduced {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_DROP);
            }
        }
        let pose = Pose::at(&open.plan, open.since.elapsed().as_secs_f64(), reduced);
        let count = self
            .live
            .as_ref()
            .map_or(0, |l| l.composer_drafts(root.as_deref()).len());
        let (mut title, detail) =
            caper_client::uploads::drop_text(&self.c.title(), self.c.direct.is_some(), count);
        if root.is_some() {
            title = "Attach to thread".into();
        }
        let g = &self.g;
        let cw = (w - 32.0).clamp(120.0, 360.0);
        let heading = g.layout(&gfx::wide(&title), &g.formats.dialog_title, cw - 48.0)?;
        let detail = g.layout(&gfx::wide(&detail), &g.formats.note, cw - 48.0)?;
        unsafe {
            heading.SetTextAlignment(
                windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_CENTER,
            )?;
            detail.SetTextAlignment(
                windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_CENTER,
            )?;
        }
        let hh = gfx::metrics(&heading).height;
        let ch = 20.0 + 132.0 + 10.0 + hh + 6.0 + gfx::metrics(&detail).height + 26.0;
        let (cx, cy) = (
            (w - cw) / 2.0,
            (h - ch) / 2.0 + 10.0 * (1.0 - pose.card_rise as f32),
        );
        let scale = self.scale;
        self.drop_p.ensure(
            g,
            (w * scale).ceil().max(1.0) as u32,
            (h * scale).ceil().max(1.0) as u32,
        )?;
        self.drop_p
            .offset((x * scale).round(), (y * scale).round())?;
        self.drop_p.draw(scale, |dc| unsafe {
            let (scrim, _) = crate::theme::scrim();
            let fade = if crate::theme::is_light() { 0.48 } else { 0.82 };
            dc.Clear(Some(&gfx::color(scrim, fade * pose.fade as f32)));
            let opacity = pose.card_opacity as f32;
            let brush =
                dc.CreateSolidColorBrush(&gfx::color(crate::theme::surface(), opacity), None)?;
            gfx::fill_round(dc, gfx::rect(cx, cy, cx + cw, cy + ch), 8.0, &brush);
            brush.SetColor(&gfx::color(crate::theme::border(), opacity));
            gfx::stroke_round(dc, gfx::rect(cx, cy, cx + cw, cy + ch), 8.0, &brush);
            brush.SetColor(&gfx::color(crate::theme::terracotta(), opacity));
            let dashed = dc.GetFactory()?.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    dashStyle: D2D1_DASH_STYLE_DASH,
                    ..Default::default()
                },
                None,
            )?;
            dc.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: gfx::rect(cx + 9.75, cy + 9.75, cx + cw - 9.75, cy + ch - 9.75),
                    radiusX: 4.0,
                    radiusY: 4.0,
                },
                &brush,
                1.5,
                &dashed,
            );
            let (sx, sy) = (cx + cw / 2.0, cy + 20.0);
            let place = |image: &Option<ID2D1Bitmap1>,
                         px: f32,
                         py: f32,
                         iw: f32,
                         ih: f32,
                         turn: f64,
                         alpha: f64| {
                if let Some(image) = image {
                    let mut old = Matrix3x2::identity();
                    dc.GetTransform(&mut old);
                    dc.SetTransform(
                        &(Matrix3x2::rotation(turn as f32) * Matrix3x2::translation(px, py) * old),
                    );
                    dc.DrawBitmap(
                        image,
                        Some(&gfx::rect(-iw / 2.0, -ih, iw / 2.0, 0.0)),
                        opacity * alpha as f32,
                        D2D1_INTERPOLATION_MODE_LINEAR,
                        None,
                        None,
                    );
                    dc.SetTransform(&old);
                }
            };
            for i in [1, 0, 2] {
                let p = &pose.cards[i];
                place(
                    &open.cards[i],
                    sx + p.x as f32,
                    sy + 98.0 + p.y as f32,
                    54.0,
                    66.0,
                    p.turn,
                    p.opacity,
                );
            }
            let p = &pose.character;
            place(
                &open.character,
                sx,
                sy + 132.0 + p.y as f32,
                76.0 * p.scale_x as f32,
                76.0 * p.scale_y as f32,
                p.turn,
                p.opacity,
            );
            brush.SetColor(&gfx::color(crate::theme::text(), opacity));
            crate::text::draw(dc, &heading, cx + 24.0, sy + 142.0, &brush);
            brush.SetColor(&gfx::color(crate::theme::text_muted(), opacity));
            crate::text::draw(dc, &detail, cx + 24.0, sy + 142.0 + hh + 6.0, &brush);
            Ok(())
        })
    }
}

fn format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
        ..Default::default()
    }
}

#[implement(IDropTarget)]
struct Target {
    hwnd: HWND,
    files: Cell<bool>,
}

impl Target_Impl {
    fn point(&self, pt: &POINTL) -> POINT {
        let mut point = POINT { x: pt.x, y: pt.y };
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut point);
        }
        point
    }
    fn over(&self, pt: &POINTL, effect: *mut DROPEFFECT) {
        let allowed =
            unsafe { !effect.is_null() && (*effect & DROPEFFECT_COPY) != DROPEFFECT_NONE };
        let accepted =
            with_app(|a| a.drag_over(self.point(pt), self.files.get() && allowed)).unwrap_or(false);
        if !effect.is_null() {
            unsafe {
                *effect = if accepted {
                    DROPEFFECT_COPY
                } else {
                    DROPEFFECT_NONE
                };
            }
        }
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        self.files.set(
            data.as_ref()
                .is_some_and(|d| unsafe { d.QueryGetData(&format()).is_ok() }),
        );
        self.over(pt, effect);
        Ok(())
    }
    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        self.over(pt, effect);
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        self.files.set(false);
        with_app(App::close_drop);
        Ok(())
    }
    fn Drop(
        &self,
        data: Ref<IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        let allowed =
            unsafe { !effect.is_null() && (*effect & DROPEFFECT_COPY) != DROPEFFECT_NONE };
        let point = self.point(pt);
        // GetData can pump COM messages. Never hold the app's RefCell borrow
        // across a call into Explorer. OLE owns the HDROP, so ReleaseStgMedium
        // (not DragFinish) releases it, including its pUnkForRelease owner.
        let paths = if allowed && let Some(data) = data.as_ref() {
            unsafe {
                match data.GetData(&format()) {
                    Ok(mut medium) => {
                        let paths = if medium.tymed == TYMED_HGLOBAL.0 as u32 {
                            uploads::drop_files(HDROP(medium.u.hGlobal.0))
                        } else {
                            Vec::new()
                        };
                        ReleaseStgMedium(&mut medium);
                        paths
                    }
                    Err(_) => Vec::new(),
                }
            }
        } else {
            Vec::new()
        };
        let accepted = with_app(|a| a.drop_paths(point, paths)).unwrap_or(false);
        self.files.set(false);
        if !effect.is_null() {
            unsafe {
                *effect = if accepted {
                    DROPEFFECT_COPY
                } else {
                    DROPEFFECT_NONE
                };
            }
        }
        Ok(())
    }
}

pub fn register(hwnd: HWND) -> Result<()> {
    let target: IDropTarget = Target {
        hwnd,
        files: Cell::new(false),
    }
    .into();
    unsafe { RegisterDragDrop(hwnd, &target) }
}
