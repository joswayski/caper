//! A message's attachments, as the egui client shows them: images and
//! videos as tiles (one framed tile, or a gallery of two or more), then
//! audio and other files as cards. Spoilers stay covered by a blurred,
//! striped cover until clicked, and a revealed one can be hidden again.
//! Images load through the shared media cache (`caper_client::media`),
//! which also refreshes expiring signed URLs and plays GIFs.
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::theme::*;
use caper_client::attachments::{frame_size, gallery, inline_source};
use caper_client::media::{ImageState, Media, Textures};
use caper_client::model::{Attachment, AttachmentKind, AttachmentStatus};
use std::collections::HashSet;
use std::time::{Duration, Instant};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::*;
use windows_numerics::Matrix3x2;

pub const CARD_W: f32 = 280.0;
pub const CARD_H: f32 = 52.0;
const GAP: f32 = 8.0;
/// A covered spoiler file card's size, as in egui.
const COVER: (f32, f32) = (180.0, 100.0);

/// Direct2D bitmaps for the shared media cache.
pub struct D2d(pub ID2D1DeviceContext);

/// Unmultiplied RGBA to premultiplied BGRA, Direct2D's bitmap format.
fn bgra(image: &caper_client::Rgba) -> Vec<u8> {
    let mut out = Vec::with_capacity(image.pixels.len());
    for p in image.pixels.as_chunks::<4>().0 {
        let a = p[3] as u32;
        let m = |c: u8| ((c as u32 * a + 127) / 255) as u8;
        out.extend_from_slice(&[m(p[2]), m(p[1]), m(p[0]), p[3]]);
    }
    out
}

impl Textures for D2d {
    type Texture = ID2D1Bitmap1;
    type Id = usize;

    fn create(&mut self, _key: &str, image: &caper_client::Rgba) -> ID2D1Bitmap1 {
        let (w, h) = (image.size[0].max(1) as u32, image.size[1].max(1) as u32);
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        let pixels = bgra(image);
        let size = D2D_SIZE_U {
            width: w,
            height: h,
        };
        unsafe {
            self.0
                .CreateBitmap(size, Some(pixels.as_ptr().cast()), w * 4, &props)
                .or_else(|_| {
                    // An image too large for the device: a 1×1 stand-in keeps
                    // the cache's bookkeeping simple.
                    self.0.CreateBitmap(
                        D2D_SIZE_U {
                            width: 1,
                            height: 1,
                        },
                        Some([0u8; 4].as_ptr().cast()),
                        4,
                        &props,
                    )
                })
                .expect("a 1×1 bitmap")
        }
    }

    fn update(&mut self, texture: &mut ID2D1Bitmap1, image: &caper_client::Rgba) {
        let pixels = bgra(image);
        unsafe {
            let _ = texture.CopyFromMemory(None, pixels.as_ptr().cast(), image.size[0] as u32 * 4);
        }
    }

    fn id(texture: &ID2D1Bitmap1) -> usize {
        texture.as_raw() as usize
    }
}

/// One attachment's place in its row, relative to the files' top-left.
pub struct File {
    /// Its index in the message's attachments.
    pub index: usize,
    pub tile: bool,
    pub rect: D2D_RECT_F,
    /// A card's name and detail lines.
    pub name: Option<IDWriteTextLayout>,
    pub detail: Option<IDWriteTextLayout>,
}

/// A row's attachments: their places, the gallery's bounds (whose outer
/// corners are rounded) and the height they take.
#[derive(Default)]
pub struct Files {
    pub list: Vec<File>,
    pub gallery: Option<D2D_RECT_F>,
    pub height: f32,
}

pub fn media_kind(a: &Attachment) -> bool {
    matches!(a.kind, AttachmentKind::Image | AttachmentKind::Video)
}

/// Covered until revealed.
pub fn hidden(a: &Attachment, revealed: &HashSet<String>) -> bool {
    a.spoiler && !revealed.contains(&a.id)
}

pub fn processing_label() -> &'static str {
    "Processing\u{2026}"
}

/// A file card's second line, as egui words it.
pub fn detail(a: &Attachment) -> String {
    let failed = !a.unavailable && a.status == AttachmentStatus::Failed;
    let processing = !a.unavailable && a.status == AttachmentStatus::Processing;
    if a.unavailable {
        "File removed".into()
    } else if failed {
        "Couldn't process this file".into()
    } else if processing {
        processing_label().into()
    } else if a.kind == AttachmentKind::Audio {
        format!(
            "Audio \u{b7} {}",
            caper_client::uploads::format_bytes(a.size)
        )
    } else {
        caper_client::uploads::format_bytes(a.size)
    }
}

/// Whether clicking opens the attachment (a viewer, the browser or player).
pub fn clickable(a: &Attachment) -> bool {
    !a.unavailable && a.status == AttachmentStatus::Ready && a.url.is_some()
}

/// What screen readers hear for an attachment, as egui labels it.
pub fn label(a: &Attachment, revealed: &HashSet<String>) -> String {
    if hidden(a, revealed) {
        return "Reveal spoiler attachment".into();
    }
    let failed = a.status == AttachmentStatus::Failed;
    let processing = a.status == AttachmentStatus::Processing;
    if !media_kind(a) {
        return if clickable(a) {
            format!("Open {}", a.name)
        } else {
            format!("{}, {}", a.name, detail(a))
        };
    }
    if a.unavailable {
        format!("{}, File removed", a.name)
    } else if failed {
        format!("{}, Couldn't process this file", a.name)
    } else if processing {
        format!("{}, {}", a.name, processing_label())
    } else if a.kind == AttachmentKind::Video && a.animated {
        format!("Play animation {}", a.name)
    } else if a.kind == AttachmentKind::Video {
        format!("Play {}", a.name)
    } else {
        format!("View {}", a.name)
    }
}

fn line(g: &Gfx, text: &str, format: &IDWriteTextFormat, w: f32) -> Result<IDWriteTextLayout> {
    let l = g.layout(&gfx::wide(text), format, w.max(1.0))?;
    crate::emoji::apply(&l, text, 12.0, 1.0)?;
    Ok(l)
}

/// The cards' text, laid out once per row build.
pub fn build(g: &Gfx, attachments: &[Attachment]) -> Result<Files> {
    let mut list = Vec::new();
    for (index, a) in attachments.iter().enumerate() {
        let tile = media_kind(a);
        let (name, detail) = if tile {
            (None, None)
        } else {
            (
                Some(line(g, &a.name, &g.formats.file_name, CARD_W - 56.0)?),
                Some(line(g, &detail(a), &g.formats.file_detail, CARD_W - 56.0)?),
            )
        };
        list.push(File {
            index,
            tile,
            rect: rect(0.0, 0.0, 0.0, 0.0),
            name,
            detail,
        });
    }
    // Media tiles first, then cards, as egui orders them.
    list.sort_by_key(|f| !f.tile);
    Ok(Files {
        list,
        gallery: None,
        height: 0.0,
    })
}

/// Places the attachments in a body `width` wide; returns their height.
pub fn place(
    files: &mut Files,
    attachments: &[Attachment],
    revealed: &HashSet<String>,
    width: f32,
) -> f32 {
    let tiles = files.list.iter().filter(|f| f.tile).count();
    let slots = gallery(tiles, width);
    files.gallery = slots
        .last()
        .map(|last| rect(0.0, 0.0, width.min(480.0), last[1] + last[3]));
    let mut y = 0.0;
    for (k, f) in files.list.iter_mut().filter(|f| f.tile).enumerate() {
        f.rect = match slots.get(k) {
            Some(&[x, sy, w, h]) => rect(x, sy, x + w, sy + h),
            None => {
                let [fw, fh] = frame_size(&attachments[f.index]);
                let s = (width / fw).clamp(0.1, 1.0);
                rect(0.0, 0.0, fw * s, fh * s)
            }
        };
        y = f32::max(y, f.rect.bottom);
    }
    if tiles > 0 {
        y += GAP;
    }
    // Cards wrap like egui's horizontal_wrapped row.
    let card_w = width.clamp(120.0, CARD_W).min(width.max(1.0));
    let (mut x, mut line_h, top) = (0.0_f32, 0.0_f32, y);
    let mut row_top = top;
    let mut any = false;
    for f in files.list.iter_mut().filter(|f| !f.tile) {
        any = true;
        let (w, h) = if hidden(&attachments[f.index], revealed) {
            (COVER.0.min(width), COVER.1)
        } else {
            (card_w, CARD_H)
        };
        if x > 0.0 && x + w > width {
            x = 0.0;
            row_top += line_h + GAP;
            line_h = 0.0;
        }
        f.rect = rect(x, row_top, x + w, row_top + h);
        for l in [&f.name, &f.detail].into_iter().flatten() {
            unsafe {
                let _ = l.SetMaxWidth((w - 56.0).max(1.0));
            }
        }
        x += w + GAP;
        line_h = line_h.max(h);
    }
    if any {
        y = row_top + line_h + GAP;
    }
    files.height = (y - GAP).max(0.0);
    files.height
}

/// The "Hide spoiler" button on a revealed spoiler, in the same
/// coordinates as `bounds`.
pub fn hide_rect(bounds: D2D_RECT_F) -> (D2D_RECT_F, bool) {
    let width = 126.0_f32.min((bounds.right - bounds.left - 8.0).max(0.0));
    let compact = width < 116.0;
    let x = bounds.right - width - 4.0;
    let y = bounds.top + 4.0;
    (
        rect(x, y, x + width, y + if compact { 42.0 } else { 30.0 }),
        compact,
    )
}

/// What drawing attachments needs beyond the row: the media cache and its
/// bitmaps, which spoilers are revealed, whether GIFs play, and when the
/// next GIF frame is due.
pub struct Ctx<'a> {
    pub media: &'a mut Media<D2d>,
    pub tex: &'a mut D2d,
    pub playing: bool,
    pub now: Instant,
    pub unix: i64,
    pub next: Option<Duration>,
    /// Rows whose GIFs play, to redraw when the next frame is due.
    pub animated: Vec<usize>,
}

impl<'a> Ctx<'a> {
    pub fn new(media: &'a mut Media<D2d>, tex: &'a mut D2d) -> Self {
        Self {
            media,
            tex,
            playing: motion(),
            now: Instant::now(),
            unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64),
            next: None,
            animated: Vec::new(),
        }
    }
}

/// Whether Windows animates (its reduced-motion setting is off), so GIFs play.
pub fn motion() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::*;
    let mut on = windows::core::BOOL(1);
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&mut on as *mut windows::core::BOOL).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    on.as_bool()
}

fn alpha(dc: &ID2D1DeviceContext, c: u32, a: f32) -> Result<ID2D1SolidColorBrush> {
    unsafe { dc.CreateSolidColorBrush(&gfx::color(c, a), None) }
}

/// Fills `r` with `bmp` scaled to cover it, centred (egui's uv crop).
fn cover(dc: &ID2D1DeviceContext, bmp: &ID2D1Bitmap1, r: D2D_RECT_F, radius: f32) -> Result<()> {
    unsafe {
        let size = bmp.GetPixelSize();
        let (bw, bh) = (size.width.max(1) as f32, size.height.max(1) as f32);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let k = (w / bw).max(h / bh);
        let brush = dc.CreateBitmapBrush(
            bmp,
            Some(&D2D1_BITMAP_BRUSH_PROPERTIES1 {
                extendModeX: D2D1_EXTEND_MODE_CLAMP,
                extendModeY: D2D1_EXTEND_MODE_CLAMP,
                interpolationMode: D2D1_INTERPOLATION_MODE_LINEAR,
            }),
            None,
        )?;
        brush.SetTransform(&Matrix3x2 {
            M11: k,
            M12: 0.0,
            M21: 0.0,
            M22: k,
            M31: r.left + (w - bw * k) / 2.0,
            M32: r.top + (h - bh * k) / 2.0,
        });
        dc.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: r,
                radiusX: radius,
                radiusY: radius,
            },
            &brush,
        );
    }
    Ok(())
}

fn centered(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    text: &str,
    r: D2D_RECT_F,
    brush: &ID2D1SolidColorBrush,
) -> Result<()> {
    let l = line(
        g,
        text,
        &g.formats.file_detail,
        (r.right - r.left - 12.0).max(1.0),
    )?;
    let m = gfx::metrics(&l);
    crate::text::draw(
        dc,
        &l,
        (r.left + r.right - m.widthIncludingTrailingWhitespace) / 2.0,
        (r.top + r.bottom - m.height) / 2.0,
        brush,
    );
    Ok(())
}

/// egui's spoiler badge: an eye-off icon and "Spoiler", with "Click to
/// reveal" under it on a cover.
fn badge(g: &Gfx, dc: &ID2D1DeviceContext, b: D2D_RECT_F) -> Result<()> {
    let (cx, cy) = ((b.left + b.right) / 2.0, (b.top + b.bottom) / 2.0);
    let w = 134.0_f32.min((b.right - b.left - 8.0).max(0.0));
    let r = rect(cx - w / 2.0, cy - 25.0, cx + w / 2.0, cy + 25.0);
    fill_round(dc, r, RADIUS, &g.brushes.blackout);
    stroke_round(dc, r, RADIUS, &g.brushes.border);
    let y = r.top + 14.0;
    icons::draw(
        dc,
        Icon::EyeOff,
        rect(cx - 33.0, y - 7.0, cx - 19.0, y + 7.0),
        TEXT,
        1.0,
    );
    let l = line(g, "Spoiler", &g.formats.file_name, 1e6)?;
    crate::text::draw(
        dc,
        &l,
        cx - 13.0,
        y - gfx::metrics(&l).height / 2.0,
        &g.brushes.text,
    );
    let l = line(g, "Click to reveal", &g.formats.file_detail, 1e6)?;
    let m = gfx::metrics(&l);
    crate::text::draw(
        dc,
        &l,
        cx - m.widthIncludingTrailingWhitespace / 2.0,
        r.bottom - 12.0 - m.height / 2.0,
        &g.brushes.muted,
    );
    Ok(())
}

fn hide_button(g: &Gfx, dc: &ID2D1DeviceContext, bounds: D2D_RECT_F) -> Result<()> {
    let (r, compact) = hide_rect(bounds);
    fill_round(dc, r, RADIUS, &g.brushes.blackout);
    stroke_round(dc, r, RADIUS, &g.brushes.border);
    let text = if compact {
        "Hide\nspoiler"
    } else {
        "Hide spoiler"
    };
    let l = line(g, text, &g.formats.file_detail, 1e6)?;
    unsafe { l.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)? };
    let m = gfx::metrics(&l);
    let content = 14.0 + 6.0 + m.width;
    let x = (r.left + r.right - content) / 2.0;
    let cy = (r.top + r.bottom) / 2.0;
    icons::draw(
        dc,
        Icon::EyeOff,
        rect(x, cy - 7.0, x + 14.0, cy + 7.0),
        TEXT,
        1.0,
    );
    crate::text::draw(
        dc,
        &l,
        x + 20.0 - m.left,
        cy - m.height / 2.0,
        &g.brushes.text,
    );
    Ok(())
}

/// Draws row `row`'s attachments with their top-left at (x, y).
#[allow(clippy::too_many_arguments)]
pub fn draw(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    files: &Files,
    attachments: &[Attachment],
    revealed: &HashSet<String>,
    mut ctx: Option<&mut Ctx>,
    row: usize,
    x: f32,
    y: f32,
) -> Result<()> {
    let at = |r: D2D_RECT_F| rect(x + r.left, y + r.top, x + r.right, y + r.bottom);
    let gallery = files.gallery.map(at);
    if let Some(b) = gallery {
        // Rounds only the gallery's outer corners: its tiles are clipped to it.
        unsafe {
            let factory = dc.GetFactory()?;
            let mask = factory.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT {
                rect: b,
                radiusX: 12.0,
                radiusY: 12.0,
            })?;
            dc.PushLayer(
                &D2D1_LAYER_PARAMETERS1 {
                    contentBounds: b,
                    geometricMask: std::mem::ManuallyDrop::new(Some(mask.cast()?)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: Matrix3x2::identity(),
                    opacity: 1.0,
                    opacityBrush: std::mem::ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS1_NONE,
                },
                None,
            );
        }
    }
    let mut result = Ok(());
    for f in files.list.iter().filter(|f| f.tile) {
        let radius = if gallery.is_some() { 1.0 } else { RADIUS };
        if let Err(e) = tile(
            g,
            dc,
            &attachments[f.index],
            revealed,
            ctx.as_deref_mut(),
            row,
            at(f.rect),
            radius,
        ) {
            result = Err(e);
        }
    }
    if gallery.is_some() {
        unsafe { dc.PopLayer() };
    }
    result?;
    for f in files.list.iter().filter(|f| !f.tile) {
        card(
            g,
            dc,
            f,
            &attachments[f.index],
            revealed,
            ctx.as_deref_mut(),
            at(f.rect),
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn tile(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    a: &Attachment,
    revealed: &HashSet<String>,
    ctx: Option<&mut Ctx>,
    row: usize,
    r: D2D_RECT_F,
    radius: f32,
) -> Result<()> {
    fill_round(dc, r, radius, &g.brushes.surface);
    if hidden(a, revealed) {
        return covered(g, dc, a, ctx, r, radius);
    }
    let processing = a.status == AttachmentStatus::Processing;
    let removed = a.unavailable;
    let failed = a.status == AttachmentStatus::Failed;
    let video = a.kind == AttachmentKind::Video;
    let mut shown = false;
    let mut failed_load = false;
    if !removed
        && !failed
        && let Some(c) = ctx
    {
        let a = &c.media.resolve(a);
        c.media.visible(a, c.unix, c.now);
        // A ready GIF plays its original frames, not a still.
        let gif = !processing && a.content_type == "image/gif";
        let state = if gif {
            let (state, next) = c
                .media
                .gif(c.tex, &a.id, a.url.as_deref(), c.playing, c.now);
            if let Some(next) = next {
                c.next = Some(c.next.map_or(next, |n| n.min(next)));
                c.animated.push(row);
            }
            state
        } else {
            c.media.image(&a.id, inline_source(a))
        };
        failed_load = matches!(state, ImageState::Failed);
        if matches!(state, ImageState::Ready(_))
            && let Some(bmp) = c.media.texture(&a.id)
        {
            cover(dc, bmp, r, radius)?;
            shown = true;
        }
    }
    if removed || failed {
        fill_round(dc, r, radius, &g.brushes.surface);
        let text = if removed {
            "File removed"
        } else {
            "Processing failed"
        };
        centered(g, dc, text, r, &g.brushes.muted)?;
    } else if processing {
        if shown {
            fill_round(dc, r, radius, &alpha(dc, 0, 140.0 / 255.0)?);
        }
        centered(g, dc, processing_label(), r, &g.brushes.text)?;
    } else if !shown && failed_load && !video {
        let text = if a.preview_url.is_none() {
            "Open to view full size"
        } else {
            "Image unavailable"
        };
        centered(g, dc, text, r, &g.brushes.muted)?;
    }
    if !processing && !removed && !failed && video {
        if a.animated {
            // The looping MP4's poster: a GIF badge.
            let b = rect(r.left + 8.0, r.top + 8.0, r.left + 42.0, r.top + 28.0);
            fill_round(dc, b, 6.0, &alpha(dc, 0, 190.0 / 255.0)?);
            centered(g, dc, "GIF", b, &g.brushes.text)?;
        } else {
            let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
            unsafe {
                dc.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: windows_numerics::Vector2 { X: cx, Y: cy },
                        radiusX: 22.0,
                        radiusY: 22.0,
                    },
                    &alpha(dc, 0, 170.0 / 255.0)?,
                );
            }
            icons::draw(
                dc,
                Icon::Play,
                rect(cx - 8.5, cy - 10.0, cx + 11.5, cy + 10.0),
                TEXT,
                1.0,
            );
        }
    }
    stroke_round(dc, r, radius, &g.brushes.border);
    if a.spoiler {
        hide_button(g, dc, r)?;
    }
    Ok(())
}

/// A covered spoiler: its blurred cover dimmed, diagonal stripes and the
/// badge, as in egui.
fn covered(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    a: &Attachment,
    ctx: Option<&mut Ctx>,
    r: D2D_RECT_F,
    radius: f32,
) -> Result<()> {
    fill_round(dc, r, radius, &g.brushes.surface);
    if let Some(c) = ctx
        && let a = &c.media.resolve(a)
        && matches!(c.media.spoiler(a), ImageState::Ready(_))
        && let Some(bmp) = c.media.texture(&caper_client::media::spoiler_key(&a.id))
    {
        cover(dc, bmp, r, radius)?;
        fill_round(dc, r, radius, &alpha(dc, 0, 70.0 / 255.0)?);
    }
    unsafe {
        dc.PushAxisAlignedClip(
            &rect(r.left + 2.0, r.top + 2.0, r.right - 2.0, r.bottom - 2.0),
            D2D1_ANTIALIAS_MODE_ALIASED,
        );
        let stripe = alpha(dc, 0xFFFFFF, 12.0 / 255.0)?;
        let h = r.bottom - r.top;
        let mut x = r.left - h;
        while x < r.right {
            dc.DrawLine(
                windows_numerics::Vector2 { X: x, Y: r.bottom },
                windows_numerics::Vector2 { X: x + h, Y: r.top },
                &stripe,
                1.0,
                None,
            );
            x += 12.0;
        }
        dc.PopAxisAlignedClip();
    }
    badge(g, dc, r)
}

fn card(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    f: &File,
    a: &Attachment,
    revealed: &HashSet<String>,
    ctx: Option<&mut Ctx>,
    r: D2D_RECT_F,
) -> Result<()> {
    if hidden(a, revealed) {
        return covered(g, dc, a, ctx, r, RADIUS);
    }
    fill_round(dc, r, RADIUS, &g.brushes.surface);
    stroke_round(dc, r, RADIUS, &g.brushes.border);
    let failed = !a.unavailable && a.status == AttachmentStatus::Failed;
    let processing = !a.unavailable && a.status == AttachmentStatus::Processing;
    if !processing {
        icons::draw(
            dc,
            Icon::FileText,
            rect(r.left + 13.0, r.top + 16.0, r.left + 33.0, r.top + 36.0),
            if failed { ERROR } else { TEXT_MUTED },
            1.0,
        );
    }
    if let Some(l) = &f.name {
        crate::text::draw(dc, l, r.left + 44.0, r.top + 9.0, &g.brushes.text);
    }
    if let Some(l) = &f.detail {
        let brush = if failed {
            &g.brushes.error
        } else {
            &g.brushes.muted
        };
        crate::text::draw(dc, l, r.left + 44.0, r.top + 29.0, brush);
    }
    if a.spoiler {
        hide_button(g, dc, r)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The window's side: redraws, clicks and UI Automation.

use crate::App;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

impl App {
    /// Images arrived or URLs changed: rows showing attachments redraw.
    pub fn media_changed(&mut self) {
        if self.c.messages.iter().any(|m| !m.attachments.is_empty()) {
            self.drawn = None;
            self.events |= crate::EV_ROWS;
            self.need = true;
        }
    }

    /// After drawing rows: when the next GIF frame is due, and a quick poll
    /// while loads wait to start or URLs to refresh.
    pub fn media_drawn(&mut self, next: Option<Duration>, animated: Vec<usize>, busy: bool) {
        unsafe {
            if let Some(next) = next {
                self.gif_rows = animated;
                SetTimer(
                    Some(self.hwnd),
                    crate::TIMER_MEDIA,
                    next.as_millis().clamp(10, u32::MAX as u128) as u32,
                    None,
                );
            }
            if busy {
                SetTimer(Some(self.hwnd), crate::TIMER_LIVE, 100, None);
            }
        }
    }

    pub fn next_gif_frame(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), crate::TIMER_MEDIA);
        }
        let rows = std::mem::take(&mut self.gif_rows);
        if !rows.is_empty() {
            self.redraw.extend(rows);
            self.events |= crate::EV_ROWS;
            self.need = true;
        }
    }

    /// The channel row and attachment under client point (x, y), and
    /// whether it is on a revealed spoiler's "Hide spoiler".
    pub fn file_at(&self, x: f32, y: f32) -> Option<(usize, usize, bool)> {
        if self.live.is_none() || !self.in_history(x, y) {
            return None;
        }
        let ay = self.scroll_top + (y - HEADER_H) as f64;
        let i = self.rows.row_at(ay)?;
        let m = self.c.messages.get(i)?;
        let ry = (ay - self.rows.top[i]) as f32;
        let (k, hide) = self.rows.file_at(i, m.grouped, x - self.list_x(), ry)?;
        let a = &m.attachments[k];
        Some((i, k, hide && a.spoiler && !hidden(a, &m.revealed)))
    }

    /// Row `i`'s attachment `k` in client DIPs, while the row is laid out.
    pub fn file_rect(&self, i: usize, k: usize) -> Option<crate::overlay::Rect> {
        if self.live.is_none() || i < self.rows.lo || i >= self.rows.len() {
            return None;
        }
        let r = self.rows.file_rect(i, self.c.messages.get(i)?.grouped, k)?;
        let y = HEADER_H + (self.rows.top[i] - self.scroll_top) as f32;
        Some((
            self.list_x() + r.left,
            y + r.top,
            r.right - r.left,
            r.bottom - r.top,
        ))
    }

    /// Row `i`'s attachments in reading order, while laid out.
    pub fn file_ids(&self, i: usize) -> Vec<usize> {
        if self.live.is_none() || i < self.rows.lo {
            return Vec::new();
        }
        self.rows
            .row(i)
            .map(|r| r.files.list.iter().map(|f| f.index).collect())
            .unwrap_or_default()
    }

    pub fn file_label(&self, i: usize, k: usize) -> String {
        self.c
            .messages
            .get(i)
            .and_then(|m| Some(label(m.attachments.get(k)?, &m.revealed)))
            .unwrap_or_default()
    }

    /// A click (or UIA Invoke) on attachment `k` of row `i`: reveals a
    /// spoiler, hides it again from its button, or opens the file.
    pub fn file_activate(&mut self, i: usize, k: usize, hide: bool) {
        let Some(m) = self.c.messages.get_mut(i) else {
            return;
        };
        let Some(a) = m.attachments.get(k).cloned() else {
            return;
        };
        let changed = if hidden(&a, &m.revealed) {
            m.revealed.insert(a.id.clone())
        } else if hide && a.spoiler {
            m.revealed.remove(&a.id)
        } else {
            false
        };
        if changed {
            match self.rows.replace(&self.g, &self.c, i) {
                Ok(true) => self.drawn = None,
                Ok(false) => self.redraw.push(i),
                Err(e) => eprintln!("attachment layout failed: {e:?}"),
            }
            self.events |= crate::EV_ROWS;
            self.need = true;
            return;
        }
        if !clickable(&a) {
            return;
        }
        // No media viewer on Windows yet: images and videos open in the
        // browser like other files, after a refresh when the URL expires.
        let Some(live) = &mut self.live else { return };
        let a = live.media.resolve(&a);
        let unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        if let Some(url) = live.media.open(&a, unix) {
            crate::open_link(&url);
        }
        if let Err(e) = self.drain_live() {
            eprintln!("attachment open failed: {e:?}");
        }
    }
}
