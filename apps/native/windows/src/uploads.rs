//! Files attached in the composer, as in the egui client: picked with the
//! system dialog (Attach files), dropped on the window or pasted, each
//! uploading at once and shown as a 104-point tile above the text with
//! Remove (its ring shows the upload's progress), a spoiler toggle and,
//! after a failure, Retry. The upload state is the shared
//! `caper_client::drafts::Upload`; sending waits for every file.
use crate::composer::{self, PAD};
use crate::files::{D2d, alpha, cover, line};
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::live::Sent;
use crate::theme::*;
use crate::{App, overlay};
use caper_client::Rgba;
use caper_client::drafts::{self, Progress, Upload};
use caper_client::media::Textures;
use caper_client::model::AttachmentKind;
use caper_client::uploads::Scratch;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::{CF_DIB, CF_HDROP};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows::core::*;

/// A staged file's tile, as in egui.
pub const TILE: f32 = 104.0;
const GAP: f32 = 8.0;
/// The text's room after the Attach files button.
pub const ATTACH_W: f32 = 30.0;
/// The text's room before the GIF button, while GIFs are available.
pub const GIF_W: f32 = 36.0;
const NOTICE_H: f32 = 18.0;
/// A corner button's circle, its inset from the tile's edges and its target.
const CORNER_R: f32 = 11.0;
const CORNER_INSET: f32 = 4.0;
const CORNER_HIT: f32 = 28.0;

/// A composer file: the shared upload state and its tile's bitmaps.
pub struct Draft {
    pub upload: Upload,
    /// The sender's own decoded copy, once preparing produced one.
    pub thumb: Option<Rgba>,
    /// A thumbnail arrived since the last upload event was handled.
    pub thumb_changed: bool,
    bitmap: Option<ID2D1Bitmap1>,
    cover: Option<ID2D1Bitmap1>,
}

impl Draft {
    pub fn new(upload: Upload) -> Self {
        Self {
            upload,
            thumb: None,
            thumb_changed: false,
            bitmap: None,
            cover: None,
        }
    }

    pub fn set_thumbnail(&mut self, image: Rgba) {
        self.thumb = Some(image);
        self.thumb_changed = true;
        self.bitmap = None;
        self.cover = None;
    }

    /// Creates the tile's bitmaps (the picture and its blurred spoiler
    /// cover) once its thumbnail is known.
    pub fn prepare(&mut self, tex: &mut D2d) {
        if self.bitmap.is_none()
            && let Some(image) = &self.thumb
        {
            self.bitmap = Some(tex.create("draft", image));
            self.cover = Some(tex.create("draft-cover", &caper_client::media::blur(image)));
        }
    }
}

/// A control in the composer box.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Attach,
    /// Send a GIF: opens the picker.
    Gif,
    /// The open channel's file `k`: its tile (preview), spoiler toggle,
    /// Remove and Retry.
    Open(usize),
    Spoiler(usize),
    Remove(usize),
    Retry(usize),
}

/// Tiles for `n` files in a box `w` wide, wrapping into rows, and the
/// height they and an optional notice line take above the text (0 for
/// neither, so the box is unchanged without files).
pub fn strip(n: usize, w: f32, notice: bool) -> (Vec<D2D_RECT_F>, f32) {
    let per = (((w - 2.0 * PAD + GAP) / (TILE + GAP)).floor() as usize).max(1);
    let tiles = (0..n)
        .map(|k| {
            let x = PAD + (k % per) as f32 * (TILE + GAP);
            let y = PAD + (k / per) as f32 * (TILE + GAP);
            rect(x, y, x + TILE, y + TILE)
        })
        .collect();
    let mut h = if n == 0 {
        0.0
    } else {
        PAD + n.div_ceil(per) as f32 * (TILE + GAP) - GAP
    };
    if notice {
        h += if n == 0 { PAD } else { 6.0 } + NOTICE_H;
    }
    (tiles, h)
}

/// Attach files, beside the first line of text below the files.
pub fn attach_rect(top: f32) -> D2D_RECT_F {
    let cy = top + PAD + composer::LINE / 2.0;
    rect(8.0, cy - 14.0, 36.0, cy + 14.0)
}

/// Send a GIF, at the end of the first line in a box `bw` wide.
pub fn gif_rect(top: f32, bw: f32) -> D2D_RECT_F {
    let cy = top + PAD + composer::LINE / 2.0;
    rect(bw - 36.0, cy - 14.0, bw - 8.0, cy + 14.0)
}

/// egui's GIF button: "GIF" in a small rounded badge.
fn gif_button(g: &Gfx, dc: &ID2D1DeviceContext, r: D2D_RECT_F, color: u32) -> Result<()> {
    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    let badge = rect(cx - 10.0, cy - 7.0, cx + 10.0, cy + 7.0);
    let brush = dc_brush(dc, color)?;
    unsafe {
        dc.DrawRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: rect(
                    badge.left + 0.7,
                    badge.top + 0.7,
                    badge.right - 0.7,
                    badge.bottom - 0.7,
                ),
                radiusX: 3.0,
                radiusY: 3.0,
            },
            &brush,
            1.4,
            None,
        );
    }
    let l = line(g, "GIF", &g.formats.gif_badge, 1e6)?;
    let m = gfx::metrics(&l);
    crate::text::draw(
        dc,
        &l,
        cx - m.widthIncludingTrailingWhitespace / 2.0,
        cy - m.height / 2.0,
        &brush,
    );
    Ok(())
}

fn remove_rect(t: D2D_RECT_F) -> D2D_RECT_F {
    rect(t.right - CORNER_HIT, t.top, t.right, t.top + CORNER_HIT)
}

fn spoiler_rect(t: D2D_RECT_F) -> D2D_RECT_F {
    rect(t.left, t.bottom - CORNER_HIT, t.left + CORNER_HIT, t.bottom)
}

fn retry_rect(t: D2D_RECT_F) -> D2D_RECT_F {
    let (cx, cy) = ((t.left + t.right) / 2.0, (t.top + t.bottom) / 2.0);
    rect(cx - 34.0, cy - 14.0, cx + 34.0, cy + 14.0)
}

fn inside(r: D2D_RECT_F, x: f32, y: f32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Whether a tile opens a preview: its local original, or the upload.
fn previewable(d: &Draft) -> bool {
    d.upload.local_path.is_some()
        || d.upload
            .attachment
            .as_ref()
            .is_some_and(|a| a.url.is_some())
}

/// The control at box-local (x, y).
pub fn part_at(tiles: &[D2D_RECT_F], drafts: &[&Draft], top: f32, x: f32, y: f32) -> Option<Part> {
    if inside(attach_rect(top), x, y) {
        return Some(Part::Attach);
    }
    let k = tiles.iter().position(|t| inside(*t, x, y))?;
    let (t, d) = (tiles[k], drafts.get(k)?);
    Some(if inside(remove_rect(t), x, y) {
        Part::Remove(k)
    } else if inside(spoiler_rect(t), x, y) {
        Part::Spoiler(k)
    } else if d.upload.error.is_some() && inside(retry_rect(t), x, y) {
        Part::Retry(k)
    } else if previewable(d) {
        Part::Open(k)
    } else {
        return None;
    })
}

/// A part's box-local bounds.
pub fn part_rect(tiles: &[D2D_RECT_F], top: f32, part: Part) -> Option<D2D_RECT_F> {
    Some(match part {
        Part::Attach => attach_rect(top),
        // Placed by the box's width (`App::box_part_rect`).
        Part::Gif => return None,
        Part::Open(k) => *tiles.get(k)?,
        Part::Spoiler(k) => spoiler_rect(*tiles.get(k)?),
        Part::Remove(k) => remove_rect(*tiles.get(k)?),
        Part::Retry(k) => retry_rect(*tiles.get(k)?),
    })
}

/// A file's name, size and status, as egui's tile describes it.
pub fn describe(d: &Draft) -> String {
    let (status, _) = d.upload.status();
    let size = caper_client::uploads::size_label(d.upload.source_size, d.upload.stored_size);
    [d.upload.name.as_str(), size.as_str(), status.as_str()]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(", ")
}

/// A round corner button: blackout at 78% with a hairline edge.
fn corner(g: &Gfx, dc: &ID2D1DeviceContext, cx: f32, cy: f32) -> Result<()> {
    let e = D2D1_ELLIPSE {
        point: windows_numerics::Vector2 { X: cx, Y: cy },
        radiusX: CORNER_R,
        radiusY: CORNER_R,
    };
    unsafe {
        dc.FillEllipse(&e, &alpha(dc, BLACKOUT, 199.0 / 255.0)?);
        dc.DrawEllipse(&e, &g.brushes.border, 1.0, None);
    }
    Ok(())
}

/// The upload's progress around Remove: a fraction clockwise from the
/// top, or a quarter while there is no fraction to show.
fn ring(dc: &ID2D1DeviceContext, cx: f32, cy: f32, progress: Progress) -> Result<()> {
    let fraction = match progress {
        Progress::Known(f) => f.clamp(0.0, 1.0),
        Progress::Unknown => 0.25,
    };
    let brush = dc_brush(dc, ACTION)?;
    let steps = (48.0 * fraction).ceil() as usize;
    let point = |t: f32| {
        let a = t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        windows_numerics::Vector2 {
            X: cx + CORNER_R * a.cos(),
            Y: cy + CORNER_R * a.sin(),
        }
    };
    for i in 0..steps {
        let (a, b) = (i as f32 / 48.0, ((i + 1) as f32 / 48.0).min(fraction));
        unsafe { dc.DrawLine(point(a), point(b), &brush, 2.0, None) };
    }
    Ok(())
}

fn dc_brush(dc: &ID2D1DeviceContext, c: u32) -> Result<ID2D1SolidColorBrush> {
    alpha(dc, c, 1.0)
}

fn centered_icon(dc: &ID2D1DeviceContext, icon: Icon, cx: f32, cy: f32, size: f32, color: u32) {
    let h = size / 2.0;
    icons::draw(dc, icon, rect(cx - h, cy - h, cx + h, cy + h), color, 1.0);
}

fn tile(g: &Gfx, dc: &ID2D1DeviceContext, d: &Draft, t: D2D_RECT_F) -> Result<()> {
    let u = &d.upload;
    let (cx, cy) = ((t.left + t.right) / 2.0, (t.top + t.bottom) / 2.0);
    fill_round(dc, t, RADIUS, &g.brushes.surface);
    if u.spoiler {
        if let Some(c) = &d.cover {
            cover(dc, c, t, RADIUS)?;
            fill_round(dc, t, RADIUS, &alpha(dc, 0, 70.0 / 255.0)?);
        }
    } else if let Some(b) = &d.bitmap {
        cover(dc, b, t, RADIUS)?;
    } else if u.kind != AttachmentKind::Video {
        centered_icon(dc, Icon::FileText, cx, cy - 8.0, 24.0, TEXT_MUTED);
        let l = line(
            g,
            &drafts::kind_label(&u.name),
            &g.formats.file_detail,
            TILE,
        )?;
        let m = gfx::metrics(&l);
        crate::text::draw(
            dc,
            &l,
            cx - m.widthIncludingTrailingWhitespace / 2.0,
            cy + 12.0 - m.height / 2.0,
            &g.brushes.muted,
        );
    }
    // The centre holds Retry after a failure, else the spoiler eye or a
    // video's play mark.
    if u.error.is_none() && (u.spoiler || u.kind == AttachmentKind::Video) {
        corner(g, dc, cx, cy)?;
        let icon = if u.spoiler { Icon::EyeOff } else { Icon::Play };
        centered_icon(dc, icon, cx, cy, 12.0, TEXT);
    }
    let edge = if u.error.is_some() {
        &g.brushes.error
    } else {
        &g.brushes.border
    };
    stroke_round(dc, t, RADIUS, edge);
    if u.error.is_some() {
        let r = retry_rect(t);
        fill_round(dc, r, RADIUS, &g.brushes.blackout);
        stroke_round(dc, r, RADIUS, &g.brushes.border);
        let l = line(g, "Retry", &g.formats.file_detail, 1e6)?;
        let m = gfx::metrics(&l);
        let w = 14.0 + 5.0 + m.widthIncludingTrailingWhitespace;
        let x = (r.left + r.right - w) / 2.0;
        centered_icon(dc, Icon::Retry, x + 7.0, cy, 14.0, TEXT);
        crate::text::draw(dc, &l, x + 19.0, cy - m.height / 2.0, &g.brushes.text);
    }
    // Remove top-right with the upload's ring; the spoiler toggle
    // diagonally opposite, so a slip meant for one never lands on the other.
    let (rx, ry) = (
        t.right - CORNER_INSET - CORNER_R,
        t.top + CORNER_INSET + CORNER_R,
    );
    corner(g, dc, rx, ry)?;
    centered_icon(dc, Icon::Close, rx, ry, 12.0, TEXT);
    if u.attachment.is_none() && u.error.is_none() {
        ring(dc, rx, ry, u.status().1)?;
    }
    let (sx, sy) = (
        t.left + CORNER_INSET + CORNER_R,
        t.bottom - CORNER_INSET - CORNER_R,
    );
    corner(g, dc, sx, sy)?;
    let icon = if u.spoiler { Icon::Eye } else { Icon::EyeOff };
    centered_icon(dc, icon, sx, sy, 12.0, TEXT);
    Ok(())
}

/// Draws the box's files, notice, Attach files and, in a box `gif` wide
/// while GIFs are available, Send a GIF (bright while its picker is open).
#[allow(clippy::too_many_arguments)]
pub fn draw(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    drafts: &[&Draft],
    tiles: &[D2D_RECT_F],
    top: f32,
    notice: Option<&str>,
    can_attach: bool,
    gif: Option<(f32, bool)>,
) -> Result<()> {
    if let Some((bw, open)) = gif {
        gif_button(
            g,
            dc,
            gif_rect(top, bw),
            if open { TEXT } else { TEXT_MUTED },
        )?;
    }
    for (d, t) in drafts.iter().zip(tiles) {
        tile(g, dc, d, *t)?;
    }
    if let Some(text) = notice {
        let l = line(g, text, &g.formats.file_detail, 1e6)?;
        crate::text::draw(
            dc,
            &l,
            PAD,
            top - NOTICE_H + (NOTICE_H - gfx::metrics(&l).height) / 2.0,
            &g.brushes.error,
        );
    }
    let a = attach_rect(top);
    let (cx, cy) = ((a.left + a.right) / 2.0, (a.top + a.bottom) / 2.0);
    let color = if can_attach { TEXT_MUTED } else { BORDER };
    centered_icon(dc, Icon::Plus, cx, cy, 18.0, color);
    Ok(())
}

/// The system file dialog on its own thread (it blocks while open), its
/// choice posted back to the window.
pub fn pick(hwnd: HWND) -> Receiver<Vec<PathBuf>> {
    let (tx, rx) = channel();
    let raw = hwnd.0 as isize;
    std::thread::spawn(move || {
        let owner = HWND(raw as _);
        let files = unsafe {
            if CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_err() {
                Vec::new()
            } else {
                let files = dialog(owner).unwrap_or_default();
                CoUninitialize();
                files
            }
        };
        let _ = tx.send(files);
        unsafe {
            let _ = PostMessageW(Some(owner), crate::live::WM_LIVE, WPARAM(0), LPARAM(0));
        }
    });
    rx
}

/// "Upload a file": any files, several at once. Cancel chooses none.
unsafe fn dialog(owner: HWND) -> Result<Vec<PathBuf>> {
    unsafe {
        let d: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        d.SetOptions(
            d.GetOptions()? | FOS_ALLOWMULTISELECT | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM,
        )?;
        d.SetTitle(w!("Upload a file"))?;
        if d.Show(Some(owner)).is_err() {
            return Ok(Vec::new());
        }
        let items = d.GetResults()?;
        let mut out = Vec::new();
        for i in 0..items.GetCount()? {
            let path = items.GetItemAt(i)?.GetDisplayName(SIGDN_FILESYSPATH)?;
            out.push(PathBuf::from(path.to_string()?));
            CoTaskMemFree(Some(path.0 as _));
        }
        Ok(out)
    }
}

/// The files in a drop or a copied file list (folders are skipped).
fn drop_files(drop: HDROP) -> Vec<PathBuf> {
    unsafe {
        let n = DragQueryFileW(drop, u32::MAX, None);
        (0..n)
            .filter_map(|i| {
                let len = DragQueryFileW(drop, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                DragQueryFileW(drop, i, Some(&mut buf));
                let path = PathBuf::from(String::from_utf16_lossy(&buf[..len]));
                path.is_file().then_some(path)
            })
            .collect()
    }
}

/// The clipboard's bytes in `format`, if it holds that format.
unsafe fn clipboard_bytes(format: u32) -> Option<Vec<u8>> {
    unsafe {
        if IsClipboardFormatAvailable(format).is_err() {
            return None;
        }
        let h = HGLOBAL(GetClipboardData(format).ok()?.0);
        let p = GlobalLock(h).cast::<u8>();
        if p.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(p, GlobalSize(h)).to_vec();
        let _ = GlobalUnlock(h);
        Some(bytes)
    }
}

/// A clipboard DIB as a BMP file: the file header the format leaves out.
fn bmp(dib: &[u8]) -> Option<Vec<u8>> {
    let u32_at = |i: usize| Some(u32::from_le_bytes(dib.get(i..i + 4)?.try_into().ok()?));
    let header = u32_at(0)?;
    let bits = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?);
    let compression = u32_at(16)?;
    let used = u32_at(32)?;
    let masks = if header == 40 && compression == 3 {
        12
    } else {
        0
    };
    let palette = if used > 0 {
        used
    } else if bits <= 8 {
        1 << bits
    } else {
        0
    };
    let offset = 14 + header + masks + palette * 4;
    let mut out = Vec::with_capacity(14 + dib.len());
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(14 + dib.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

/// What a paste attaches: copied files, else a copied picture saved as a
/// PNG the draft keeps (egui's "Pasted image.png"). Empty for text.
fn clipboard_files(hwnd: HWND) -> Vec<(PathBuf, Option<Arc<Scratch>>)> {
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return Vec::new();
        }
        let files = if IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_ok() {
            GetClipboardData(CF_HDROP.0 as u32)
                .map(|h| drop_files(HDROP(h.0)))
                .unwrap_or_default()
                .into_iter()
                .map(|p| (p, None))
                .collect()
        } else {
            let png = RegisterClipboardFormatW(w!("PNG"));
            let picture = clipboard_bytes(png).or_else(|| {
                let image = image::load_from_memory_with_format(
                    &bmp(&clipboard_bytes(CF_DIB.0 as u32)?)?,
                    image::ImageFormat::Bmp,
                )
                .ok()?;
                let mut out = std::io::Cursor::new(Vec::new());
                image.write_to(&mut out, image::ImageFormat::Png).ok()?;
                Some(out.into_inner())
            });
            picture
                .and_then(|bytes| {
                    let scratch = Arc::new(Scratch::new("png"));
                    std::fs::write(scratch.path(), bytes).ok()?;
                    Some(vec![(scratch.path().to_path_buf(), Some(scratch))])
                })
                .unwrap_or_default()
        };
        let _ = CloseClipboard();
        files
    }
}

impl App {
    /// The open channel's composer files, their tiles in a box `bw` wide,
    /// the height above the text, and the notice line.
    pub fn box_files(&self, bw: f32) -> (Vec<&Draft>, Vec<D2D_RECT_F>, f32, Option<String>) {
        let Some(live) = &self.live else {
            return (Vec::new(), Vec::new(), 0.0, None);
        };
        let drafts = live.open_drafts();
        let failed = drafts.iter().filter(|d| d.upload.error.is_some()).count();
        let notice = drafts::failed_notice(failed).or_else(|| self.attach_error.clone());
        let (tiles, top) = strip(drafts.len(), bw, notice.is_some());
        (drafts, tiles, top, notice)
    }

    /// The composer box's height for width `bw`, its text laid out after
    /// Attach files and below the files.
    pub fn box_height(&mut self, bw: f32) -> Result<f32> {
        if let Some(bar) = self.dm_bar() {
            return Ok(bar.layout(&self.g, bw)?.1);
        }
        let top = if self.live.is_some() {
            self.box_files(bw).2
        } else {
            0.0
        };
        let left = if self.live.is_some() { ATTACH_W } else { 0.0 };
        let right = if self.gif_shown() { GIF_W } else { 0.0 };
        self.composer.inset = (left, top);
        Ok(top
            + self
                .composer
                .relayout(&self.g, bw - 2.0 * PAD - left - right)?)
    }

    /// Whether the composer offers GIFs (KLIPY answered for this account).
    pub fn gif_shown(&self) -> bool {
        self.live.as_ref().is_some_and(|l| l.can_gif())
    }

    /// The composer box's control under client point (x, y).
    pub fn box_part_at(&self, x: f32, y: f32) -> Option<Part> {
        if self.live.is_none() || !self.in_box(x, y) || self.dm_bar().is_some() {
            return None;
        }
        let (bx, by, bw, _) = self.box_rect();
        let (drafts, tiles, top, _) = self.box_files(bw);
        if self.gif_shown() && inside(gif_rect(top, bw), x - bx, y - by) {
            return Some(Part::Gif);
        }
        part_at(&tiles, &drafts, top, x - bx, y - by)
    }

    /// The composer box's controls, in reading order (for UI Automation).
    pub fn box_parts(&self) -> Vec<Part> {
        if self.live.is_none() || self.dm_bar().is_some() {
            return Vec::new();
        }
        let (_, _, bw, _) = self.box_rect();
        let (drafts, ..) = self.box_files(bw);
        let mut out = Vec::new();
        for (k, d) in drafts.iter().enumerate() {
            if previewable(d) {
                out.push(Part::Open(k));
            }
            if d.upload.error.is_some() {
                out.push(Part::Retry(k));
            }
            out.extend([Part::Spoiler(k), Part::Remove(k)]);
        }
        out.push(Part::Attach);
        if self.gif_shown() {
            out.push(Part::Gif);
        }
        out
    }

    pub fn box_part_rect(&self, part: Part) -> Option<overlay::Rect> {
        let (bx, by, bw, _) = self.box_rect();
        let (_, tiles, top, _) = self.box_files(bw);
        let r = match part {
            Part::Gif => gif_rect(top, bw),
            _ => part_rect(&tiles, top, part)?,
        };
        Some((bx + r.left, by + r.top, r.right - r.left, r.bottom - r.top))
    }

    pub fn box_part_label(&self, part: Part) -> String {
        let (_, _, bw, _) = self.box_rect();
        let (drafts, ..) = self.box_files(bw);
        let file = |k: usize| drafts.get(k).map(|d| (describe(d), d));
        match part {
            Part::Attach => match &self.live {
                Some(live) if live.can_attach() => "Attach files".into(),
                Some(live) => live.attach_unavailable().into(),
                None => String::new(),
            },
            Part::Gif => "Send a GIF".into(),
            Part::Open(k) => file(k).map_or_else(String::new, |(s, _)| format!("Preview {s}")),
            Part::Spoiler(k) => file(k).map_or_else(String::new, |(_, d)| {
                if d.upload.spoiler {
                    format!("Remove spoiler from {}", d.upload.name)
                } else {
                    format!("Mark {} as spoiler", d.upload.name)
                }
            }),
            Part::Remove(k) => file(k).map_or_else(String::new, |(s, _)| format!("Remove {s}")),
            Part::Retry(k) => file(k).map_or_else(String::new, |(_, d)| {
                format!("Retry uploading {}", d.upload.name)
            }),
        }
    }

    /// A click (or UIA Invoke) on a composer box control.
    pub fn box_activate(&mut self, part: Part) {
        match part {
            Part::Attach => return self.pick_files(),
            Part::Gif => return self.toggle_gifs(),
            _ => {}
        }
        let (_, _, bw, _) = self.box_rect();
        let (drafts, ..) = self.box_files(bw);
        let k = match part {
            Part::Open(k) | Part::Spoiler(k) | Part::Remove(k) | Part::Retry(k) => k,
            Part::Attach | Part::Gif => return,
        };
        let Some(d) = drafts.get(k) else { return };
        let key = d.upload.key;
        // The local original opens as it is; an uploaded file by its URL.
        let open = d
            .upload
            .local_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .or_else(|| d.upload.attachment.as_ref().and_then(|a| a.url.clone()));
        let Some(live) = &mut self.live else { return };
        match part {
            Part::Open(_) => {
                if let Some(target) = open {
                    crate::open_link(&target);
                }
                return;
            }
            Part::Spoiler(_) => live.toggle_spoiler(key),
            Part::Remove(_) => {
                live.remove_draft(key);
                self.attach_error = None;
            }
            Part::Retry(_) => live.retry_draft(key),
            Part::Attach | Part::Gif => {}
        }
        self.drafts_changed();
    }

    /// Attach files: the system dialog, unless one is open or uploads are
    /// unavailable.
    pub fn pick_files(&mut self) {
        if self.file_dialog.is_some() || !self.live.as_ref().is_some_and(|l| l.can_attach()) {
            return;
        }
        self.file_dialog = Some(pick(self.hwnd));
    }

    /// The dialog's choice, once made.
    pub fn poll_file_dialog(&mut self) {
        let Some(rx) = &self.file_dialog else { return };
        if let Ok(paths) = rx.try_recv() {
            self.file_dialog = None;
            self.attach(paths.into_iter().map(|p| (p, None)).collect());
        }
    }

    /// Files dropped on the window.
    pub fn dropped(&mut self, drop: HDROP) {
        let files = drop_files(drop);
        unsafe { DragFinish(drop) };
        self.attach(files.into_iter().map(|p| (p, None)).collect());
    }

    /// Ctrl+V in the composer: copied files or a picture attach; text
    /// pastes as usual (false).
    pub fn paste_files(&mut self) -> bool {
        if !self.live.as_ref().is_some_and(|l| l.can_attach()) {
            return false;
        }
        let files = clipboard_files(self.hwnd);
        if files.is_empty() {
            return false;
        }
        self.attach(files);
        true
    }

    fn attach(&mut self, files: Vec<(PathBuf, Option<Arc<Scratch>>)>) {
        if files.is_empty() {
            return;
        }
        let Some(live) = &mut self.live else { return };
        self.attach_error = live.add_files(files);
        self.drafts_changed();
    }

    /// The composer's files changed: the box relayouts and redraws, and a
    /// send waiting for them goes once they are all uploaded (or stops when
    /// one failed or the draft changed).
    pub fn drafts_changed(&mut self) {
        if let Some((text, keys)) = &self.send_waiting
            && let Some(live) = &self.live
        {
            let drafts = live.open_drafts();
            let now: Vec<u64> = drafts.iter().map(|d| d.upload.key).collect();
            if *text != self.composer.text
                || now != *keys
                || drafts.iter().any(|d| d.upload.error.is_some())
            {
                self.send_waiting = None;
            } else if drafts
                .iter()
                .all(|d| !d.upload.uploading && !d.upload.needs_upload())
            {
                self.send_waiting = None;
                if let Err(e) = self.send() {
                    eprintln!("send failed: {e:?}");
                }
                return;
            }
        }
        self.composer_changed();
    }

    /// Sends the composer's text with its files: at once when they are
    /// uploaded, else when they finish. Returns whether it went now.
    pub fn send_files(&mut self, text: String) -> bool {
        let reply = self.reply.as_ref().map(|r| r.target.id.clone());
        let Some(live) = &mut self.live else {
            return false;
        };
        let keys: Vec<u64> = live.open_drafts().iter().map(|d| d.upload.key).collect();
        if live.send_with_files(text, reply) == Sent::Waiting {
            self.send_waiting = Some((self.composer.text.clone(), keys));
            self.drafts_changed();
            return false;
        }
        self.send_waiting = None;
        self.attach_error = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_is_empty_without_files_and_wraps_tiles_into_rows() {
        assert_eq!(strip(0, 600.0, false).1, 0.0);
        // Only the notice: the line and the box's top padding.
        assert_eq!(strip(0, 600.0, true).1, PAD + NOTICE_H);
        // 600 wide holds (600 - 24 + 8) / 112 = 5 tiles a row.
        let (tiles, h) = strip(7, 600.0, false);
        assert_eq!(tiles[4].left, PAD + 4.0 * (TILE + GAP));
        assert_eq!((tiles[5].left, tiles[5].top), (PAD, PAD + TILE + GAP));
        assert_eq!(h, PAD + 2.0 * TILE + GAP);
        // A box narrower than one tile still shows them, one a row.
        let (tiles, _) = strip(2, 50.0, false);
        assert_eq!(tiles[1].top, PAD + TILE + GAP);
    }

    #[test]
    fn corners_beat_the_tile_and_retry_only_after_a_failure() {
        let mut upload = Upload::picked(1, "c".into(), None, "a.png".into(), Some("a.png".into()));
        let tiles = strip(1, 600.0, false).0;
        let t = tiles[0];
        let at = |d: &Draft, x: f32, y: f32| part_at(&tiles, &[d], 0.0, x, y);
        let d = Draft::new(Upload::picked(2, "c".into(), None, "b".into(), None));
        // Nothing to preview yet: the tile's middle is inert.
        assert_eq!(at(&d, t.left + 50.0, t.top + 50.0), None);
        upload.error = Some("Failed to upload.".into());
        let d = Draft::new(upload);
        assert_eq!(at(&d, t.right - 2.0, t.top + 2.0), Some(Part::Remove(0)));
        assert_eq!(at(&d, t.left + 2.0, t.bottom - 2.0), Some(Part::Spoiler(0)));
        assert_eq!(at(&d, t.left + 52.0, t.top + 52.0), Some(Part::Retry(0)));
        assert_eq!(at(&d, t.left + 52.0, t.top + 20.0), Some(Part::Open(0)));
        assert_eq!(at(&d, 20.0, attach_rect(0.0).top + 2.0), Some(Part::Attach));
    }

    #[test]
    fn a_clipboard_dib_gains_the_bmp_file_header() {
        // A 1×1 32-bit BI_BITFIELDS DIB: 40-byte header, 12 bytes of masks.
        let mut dib = vec![0u8; 40 + 12 + 4];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1i32.to_le_bytes());
        dib[8..12].copy_from_slice(&1i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib[16..20].copy_from_slice(&3u32.to_le_bytes());
        dib[40..44].copy_from_slice(&0x00FF_0000u32.to_le_bytes());
        dib[44..48].copy_from_slice(&0x0000_FF00u32.to_le_bytes());
        dib[48..52].copy_from_slice(&0x0000_00FFu32.to_le_bytes());
        dib[52..56].copy_from_slice(&[0x30, 0x20, 0x10, 0xFF]);
        let file = bmp(&dib).unwrap();
        assert_eq!(&file[0..2], b"BM");
        assert_eq!(
            u32::from_le_bytes(file[10..14].try_into().unwrap()),
            14 + 40 + 12
        );
        let image = image::load_from_memory_with_format(&file, image::ImageFormat::Bmp)
            .unwrap()
            .to_rgba8();
        assert_eq!(image.get_pixel(0, 0).0[..3], [0x10, 0x20, 0x30]);
    }
}
