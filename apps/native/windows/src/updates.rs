//! Self-updates (`caper_client::updates`), as in the egui client: a release
//! build checks for a newer signed release in the background; when one
//! exists, "Update available" sits above the account footer, and it opens
//! the review ("A fresh Caper is ready"), whose Restart to update starts
//! `caper-updater` and quits so it can swap the new build in and reopen.
use crate::gfx::{Gfx, fill_round, rect, stroke_round};
use crate::overlay::{Dialog, Kind, Part, Rect};
use crate::settings::FOOT_H;
use crate::theme::*;
use crate::{App, EV_SCREEN, text};
use caper_client::state::Phase;
use caper_client::updates::{Available, DOWNLOAD_URL};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsIconic, KillTimer, SetTimer};
use windows::core::{Interface, Result};
use windows_numerics::{Matrix3x2, Vector2};

/// The shine's timer, and its rhythm: one sweep of `--motion-slow` × 6
/// every eight seconds.
pub const TIMER_SHINE: usize = 40;
const SHINE_EVERY: f32 = 8.0;
const SHINE_SWEEP: f32 = 0.26 * 6.0;
/// The prismatic edge, left to right (UpdateButtonShine's colours).
const SHINE_COLORS: [u32; 5] = [0xA780F2, 0x68AFEC, 0x63D2C1, 0xD7CD82, 0xE58BB7];

/// egui's dock: a 36-DIP secondary button with 8 DIPs under it.
pub const DOCK_H: f32 = 36.0;
const DOCK_GAP: f32 = 8.0;

/// A line of the review's body.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Line {
    /// "The last update didn't finish", in the error colour.
    Failed,
    /// "WHAT'S NEW".
    Heading,
    /// A highlight's title.
    Title,
    /// Everything else, muted.
    Body,
}

/// How a body line is set: the error's red, the small heading, a
/// highlight's bold title, or muted text.
pub fn line_format(g: &Gfx, line: Line) -> &IDWriteTextFormat {
    match line {
        Line::Failed => &g.formats.error,
        Line::Heading => &g.formats.section,
        Line::Title => &g.formats.direct_name,
        Line::Body => &g.formats.dialog_note,
    }
}

pub fn line_brush(g: &Gfx, line: Line) -> &ID2D1SolidColorBrush {
    match line {
        Line::Failed => &g.brushes.error,
        Line::Title => &g.brushes.text,
        Line::Heading | Line::Body => &g.brushes.muted,
    }
}

impl Dialog {
    pub fn update(available: Available, error: Option<String>, in_call: bool) -> Self {
        let mut d = Self::new(Kind::Update, None, crate::composer::Composer::default());
        d.error = error.or_else(|| available.last_error.clone());
        d.in_call = in_call;
        d.update = Some(Box::new(available));
        d
    }

    pub(crate) fn update_title() -> &'static str {
        "A fresh Caper is ready"
    }

    /// "Version 0.1.82 · October 18, 2026".
    pub(crate) fn update_note(&self) -> String {
        let Some(u) = &self.update else {
            return String::new();
        };
        let date = u
            .released
            .as_deref()
            .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .map(|d| d.format("%B %-d, %Y").to_string());
        match date {
            Some(date) => format!("Version {} \u{b7} {date}", u.version),
            None => format!("Version {}", u.version),
        }
    }

    pub(crate) fn update_primary(&self) -> &'static str {
        match self.update.as_ref().map(|u| u.can_apply) {
            Some(false) => "Download",
            _ if self.in_call => "Restart and leave call",
            _ => "Restart to update",
        }
    }

    /// The body's lines in reading order, as egui shows them: why the last
    /// update failed, then what's new (the stacked highlights, else the
    /// updater's plain notes), then the download size.
    pub(crate) fn update_lines(&self) -> Vec<(Line, String)> {
        let Some(u) = &self.update else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Some(e) = &self.error {
            out.push((Line::Failed, "The last update didn\u{2019}t finish".into()));
            out.push((
                Line::Body,
                if u.can_apply {
                    "Try again using the button below, or use the installer.".into()
                } else {
                    "Download the installer with the button below.".into()
                },
            ));
            out.push((Line::Body, e.clone()));
        }
        out.push((Line::Heading, "WHAT\u{2019}S NEW".into()));
        if !u.history_complete {
            out.push((
                Line::Body,
                "Earlier release notes aren\u{2019}t available.".into(),
            ));
        }
        match &u.highlights {
            Some(items) if items.is_empty() => {
                out.push((Line::Body, "Behind-the-scenes fixes this time.".into()));
            }
            Some(items) => {
                for item in items {
                    out.push((Line::Title, item.title.clone()));
                    out.push((Line::Body, item.body.clone()));
                }
            }
            None => {
                out.push((Line::Body, u.notes.clone()));
                out.extend(
                    u.changelog
                        .iter()
                        .flat_map(|entry| entry.notes.lines())
                        .filter(|line| !line.trim().is_empty())
                        .map(|line| (Line::Body, line.to_owned())),
                );
            }
        }
        out
    }

    /// The footer under the notes: the manual-update note or the download size.
    pub(crate) fn update_footer(&self) -> Option<String> {
        let u = self.update.as_ref()?;
        if !u.can_apply {
            return Some("Download the installer to update this copy manually.".into());
        }
        u.download_label()
    }

    /// Line `k` of the body as laid out (the notes are cut to the window).
    pub fn update_line(&self, k: u8) -> String {
        self.update_shown
            .get(k as usize)
            .map(|(_, s)| s.clone())
            .unwrap_or_default()
    }
}

impl App {
    /// "Update available" (client DIPs), above the account footer while an
    /// update is known, as egui's dock.
    pub fn update_dock_rect(&self) -> Option<Rect> {
        (self.update_dock.is_some() && self.foot_shown()).then_some((
            RAIL_W + 12.0,
            self.h - FOOT_H - DOCK_GAP - DOCK_H,
            SIDEBAR_W - 24.0,
            DOCK_H,
        ))
    }

    /// The dock's height with its gap, for what stacks above it.
    pub fn update_dock_h(&self) -> f32 {
        if self.update_dock_rect().is_some() {
            DOCK_H + DOCK_GAP
        } else {
            0.0
        }
    }

    pub fn on_update_dock(&self, x: f32, y: f32) -> bool {
        self.update_dock_rect()
            .is_some_and(|(rx, ry, rw, rh)| x >= rx && x < rx + rw && y >= ry && y < ry + rh)
    }

    /// The dock's tooltip text in egui, as its UI Automation help text.
    pub fn update_dock_help(&self) -> String {
        self.update_dock.as_deref().map_or_else(String::new, |v| {
            format!("Caper {v} is available. Review the update before restarting.")
        })
    }

    /// Draws the dock in the sidebar (sidebar-local DIPs).
    pub fn draw_update_dock(&self, g: &Gfx, dc: &ID2D1DeviceContext) -> Result<()> {
        let Some((x, y, w, h)) = self.update_dock_rect() else {
            return Ok(());
        };
        let x = x - RAIL_W;
        let r = rect(x, y, x + w, y + h);
        fill_round(dc, r, RADIUS, &g.solid(SURFACE_RAISED));
        if self.update_dock_hover {
            fill_round(dc, r, RADIUS, &g.brushes.hover);
        }
        stroke_round(dc, r, RADIUS, &g.brushes.border);
        self.draw_update_shine(dc, r)?;
        let l = crate::overlay::layout(g, "Update available", &g.formats.button, 1e6)?;
        let m = crate::gfx::metrics(&l);
        text::draw(
            dc,
            &l,
            x + (w - m.widthIncludingTrailingWhitespace) / 2.0,
            y + (h - m.height) / 2.0,
            &g.brushes.text,
        );
        Ok(())
    }

    /// Whether the shine's sweep may move: Windows animates (reduced
    /// motion is off) and Caper is the window in front, as egui and macOS.
    fn shine_running(&self) -> bool {
        crate::files::motion()
            && unsafe { GetForegroundWindow() } == self.hwnd
            && !unsafe { IsIconic(self.hwnd) }.as_bool()
    }

    /// The sweep's eased progress now, while one is passing.
    fn shine_progress(&self) -> Option<f32> {
        let start = self.update_shine?;
        let phase = start.elapsed().as_secs_f32() % SHINE_EVERY;
        (phase < SHINE_SWEEP && self.shine_running()).then(|| {
            let t = phase / SHINE_SWEEP;
            if t >= 1.0 {
                1.0
            } else {
                1.0 - 2f32.powf(-10.0 * t)
            }
        })
    }

    /// egui's and macOS's UpdateButtonShine: a still prismatic edge (a soft
    /// 4-DIP glow under a 1-DIP line, drawn inside the button) and one
    /// short light sweep every eight seconds, clipped to the corners. The
    /// label and hit target never move.
    fn draw_update_shine(&self, dc: &ID2D1DeviceContext, r: D2D_RECT_F) -> Result<()> {
        unsafe {
            let stops: Vec<D2D1_GRADIENT_STOP> = SHINE_COLORS
                .iter()
                .enumerate()
                .map(|(k, c)| D2D1_GRADIENT_STOP {
                    position: k as f32 / (SHINE_COLORS.len() - 1) as f32,
                    color: crate::gfx::color(*c, 1.0),
                })
                .collect();
            let collection = ID2D1RenderTarget::CreateGradientStopCollection(
                dc,
                &stops,
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            )?;
            let edge = dc.CreateLinearGradientBrush(
                &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: Vector2 { X: r.left, Y: 0.0 },
                    endPoint: Vector2 { X: r.right, Y: 0.0 },
                },
                None,
                &collection,
            )?;
            for (width, alpha) in [(4.0_f32, 0.12), (1.0, 0.75)] {
                edge.SetOpacity(alpha);
                let i = width / 2.0;
                dc.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: rect(r.left + i, r.top + i, r.right - i, r.bottom - i),
                        radiusX: RADIUS - i,
                        radiusY: RADIUS - i,
                    },
                    &edge,
                    width,
                    None,
                );
            }
            let Some(progress) = self.shine_progress() else {
                return Ok(());
            };
            let mask = dc
                .GetFactory()?
                .CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT {
                    rect: r,
                    radiusX: RADIUS,
                    radiusY: RADIUS,
                })?;
            dc.PushLayer(
                &D2D1_LAYER_PARAMETERS1 {
                    contentBounds: r,
                    geometricMask: std::mem::ManuallyDrop::new(Some(mask.cast()?)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: Matrix3x2::identity(),
                    opacity: 1.0,
                    opacityBrush: std::mem::ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS1_NONE,
                },
                None,
            );
            // A 56-DIP band of white (16% at its middle), tilted 18°,
            // crossing from 80 DIPs before the button to 80 after it.
            let h = r.bottom - r.top;
            let x = r.left - 80.0 + (r.right - r.left + 160.0) * progress;
            let band = [
                D2D1_GRADIENT_STOP {
                    position: 0.0,
                    color: crate::gfx::color(0xFFFFFF, 0.0),
                },
                D2D1_GRADIENT_STOP {
                    position: 0.5,
                    color: crate::gfx::color(0xFFFFFF, 0.16),
                },
                D2D1_GRADIENT_STOP {
                    position: 1.0,
                    color: crate::gfx::color(0xFFFFFF, 0.0),
                },
            ];
            let collection = ID2D1RenderTarget::CreateGradientStopCollection(
                dc,
                &band,
                D2D1_GAMMA_2_2,
                D2D1_EXTEND_MODE_CLAMP,
            )?;
            let light = dc.CreateLinearGradientBrush(
                &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                    startPoint: Vector2 { X: 0.0, Y: 0.0 },
                    endPoint: Vector2 { X: 56.0, Y: 0.0 },
                },
                None,
                &collection,
            )?;
            let mut base = Matrix3x2::default();
            dc.GetTransform(&mut base);
            let tilt = Matrix3x2::rotation(18.0) * Matrix3x2::translation(x, r.top - h) * base;
            dc.SetTransform(&tilt);
            dc.FillRectangle(&rect(0.0, 0.0, 56.0, h * 3.0), &light);
            dc.SetTransform(&base);
            dc.PopLayer();
        }
        Ok(())
    }

    /// Paces the shine: frames while a sweep passes, then a timer for the
    /// next one; while it can't move, a slow check for when it can again.
    pub(crate) fn shine_tick(&mut self) {
        let Some(start) = self
            .update_shine
            .filter(|_| self.update_dock_rect().is_some())
        else {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_SHINE);
            }
            return;
        };
        let running = self.shine_running();
        let phase = start.elapsed().as_secs_f32() % SHINE_EVERY;
        let sweeping = running && phase < SHINE_SWEEP;
        if sweeping || self.shine_drawn {
            // Each frame of the sweep, and once after it to clear it.
            self.dirty.sidebar = true;
            self.need = true;
        }
        self.shine_drawn = sweeping;
        let next = if !running {
            1.0
        } else if sweeping {
            0.033
        } else {
            SHINE_EVERY - phase
        };
        unsafe {
            SetTimer(
                Some(self.hwnd),
                TIMER_SHINE,
                (next * 1000.0).max(1.0) as u32,
                None,
            );
        }
    }

    /// The dock was clicked: the review opens.
    pub fn open_update_review(&mut self) {
        self.updates.show_available();
        self.poll_updates();
    }

    /// After each wake: the dock follows what the checks found, and the
    /// review opens when one was asked for and nothing else is open.
    pub(crate) fn poll_updates(&mut self) {
        let known = self.updates.known_available().map(|u| u.version);
        if known != self.update_dock {
            self.update_shine = known.as_ref().map(|_| std::time::Instant::now());
            self.update_dock = known;
            self.shine_tick();
            self.dirty.sidebar = true;
            self.events |= EV_SCREEN;
            self.need = true;
        }
        if self.dialog.is_some() || self.signin.is_some() || self.viewer.is_some() {
            return;
        }
        if let Some(available) = self.updates.available() {
            let in_call = self
                .live
                .as_ref()
                .is_some_and(|l| !matches!(l.voice.state.phase, Phase::Idle));
            let d = Dialog::update(available, self.updates.error.clone(), in_call);
            self.open_dialog(d);
        }
    }

    /// Restart to update (or Download, for a copy the updater can't
    /// replace), and Later.
    pub(crate) fn update_activate(&mut self, part: Part) {
        match part {
            Part::Primary => {
                let can_apply = self
                    .dialog
                    .as_ref()
                    .and_then(|d| d.update.as_ref())
                    .is_some_and(|u| u.can_apply);
                if !can_apply {
                    crate::open_link(DOWNLOAD_URL);
                } else if self.updates.apply() {
                    // The updater waits for this process to exit.
                    self.quit = true;
                    self.need = true;
                } else if let Some(d) = &mut self.dialog {
                    d.error = self.updates.error.clone();
                    self.dialog_changed();
                }
            }
            Part::Cancel | Part::Close => self.close_dialog(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use caper_client::updates::Highlight;

    fn release() -> Available {
        Available {
            version: "0.1.90".into(),
            notes: "Release notes".into(),
            changelog: Vec::new(),
            history_complete: true,
            highlights: Some(vec![Highlight {
                title: "Faster scrolling".into(),
                body: "Long channels scroll smoothly.".into(),
            }]),
            released: Some("2026-11-09".into()),
            revision: None,
            sequence: None,
            last_error: None,
            can_apply: true,
            download_size: Some(1_500_000),
            patch: true,
        }
    }

    #[test]
    fn the_review_reads_as_egui_s() {
        let d = Dialog::update(release(), None, false);
        assert_eq!(d.title(), "A fresh Caper is ready");
        assert_eq!(d.note(), "Version 0.1.90 \u{b7} November 9, 2026");
        assert_eq!(d.primary_label(), "Restart to update");
        assert_eq!(d.cancel_label(), "Later");
        assert_eq!(
            d.update_lines(),
            vec![
                (Line::Heading, "WHAT\u{2019}S NEW".into()),
                (Line::Title, "Faster scrolling".into()),
                (Line::Body, "Long channels scroll smoothly.".into()),
            ]
        );
        assert_eq!(
            d.update_footer().as_deref(),
            Some("1.5 MB download, only what changed")
        );
        // In a call it says it will leave; a copy it can't replace downloads.
        assert_eq!(
            Dialog::update(release(), None, true).primary_label(),
            "Restart and leave call"
        );
        let manual = Available {
            can_apply: false,
            ..release()
        };
        let d = Dialog::update(manual, Some("Access denied".into()), false);
        assert_eq!(d.primary_label(), "Download");
        assert_eq!(
            d.update_lines()[..3],
            [
                (Line::Failed, "The last update didn\u{2019}t finish".into()),
                (
                    Line::Body,
                    "Download the installer with the button below.".into()
                ),
                (Line::Body, "Access denied".into()),
            ]
        );
        assert_eq!(
            d.update_footer().as_deref(),
            Some("Download the installer to update this copy manually.")
        );
    }
}
