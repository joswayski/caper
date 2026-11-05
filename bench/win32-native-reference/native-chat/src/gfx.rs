//! Devices and shared drawing resources: D3D11 -> DXGI -> D2D device, DirectComposition desktop
//! device, DirectWrite factory with the Satoshi fonts in an in-memory collection, solid brushes.
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::core::*;
use windows_numerics::Matrix3x2;

pub fn color(c: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((c >> 16) & 0xff) as f32 / 255.0,
        g: ((c >> 8) & 0xff) as f32 / 255.0,
        b: (c & 0xff) as f32 / 255.0,
        a,
    }
}

pub fn rect(l: f32, t: f32, r: f32, b: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: l,
        top: t,
        right: r,
        bottom: b,
    }
}

pub struct Brushes {
    pub blackout: ID2D1SolidColorBrush,
    pub surface: ID2D1SolidColorBrush,
    pub border: ID2D1SolidColorBrush,
    pub text: ID2D1SolidColorBrush,
    pub muted: ID2D1SolidColorBrush,
    pub terracotta: ID2D1SolidColorBrush,
    pub terracotta_30: ID2D1SolidColorBrush,
    pub mention_bg: ID2D1SolidColorBrush,
    pub green: ID2D1SolidColorBrush,
    pub link: ID2D1SolidColorBrush,
    pub selection: ID2D1SolidColorBrush,
    pub authors: Vec<ID2D1SolidColorBrush>,
}

pub struct Formats {
    pub message: IDWriteTextFormat,
    pub name: IDWriteTextFormat,
    pub time: IDWriteTextFormat,
    pub initials: IDWriteTextFormat,
    pub reaction: IDWriteTextFormat,
    pub channel: IDWriteTextFormat,
    pub title: IDWriteTextFormat,
    pub header: IDWriteTextFormat,
    pub members_title: IDWriteTextFormat,
    pub member: IDWriteTextFormat,
    pub rail: IDWriteTextFormat,
    pub composer: IDWriteTextFormat,
    pub toolbar: IDWriteTextFormat,
}

pub struct Gfx {
    pub d3d: ID3D11Device,
    pub d2d: ID2D1Device,
    pub dc: ID2D1DeviceContext,
    pub dcomp: IDCompositionDesktopDevice,
    pub dwrite: IDWriteFactory5,
    pub fonts: IDWriteFontCollection1,
    pub brushes: Brushes,
    pub formats: Formats,
}

const SATOSHI: [&[u8]; 3] = [
    include_bytes!("../../../../shared/fonts/cache/Satoshi-Regular.otf"),
    include_bytes!("../../../../shared/fonts/cache/Satoshi-Medium.otf"),
    include_bytes!("../../../../shared/fonts/cache/Satoshi-Bold.otf"),
];

impl Gfx {
    pub fn new() -> Result<Self> {
        unsafe {
            let mut d3d = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[
                    D3D_FEATURE_LEVEL_11_1,
                    D3D_FEATURE_LEVEL_11_0,
                    D3D_FEATURE_LEVEL_10_1,
                ]),
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                None,
            )?;
            let d3d: ID3D11Device = d3d.unwrap();
            let dxgi: IDXGIDevice = d3d.cast()?;
            let factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d = factory.CreateDevice(&dxgi)?;
            let dc = d2d.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let dcomp: IDCompositionDesktopDevice = DCompositionCreateDevice2(&d2d)?;
            let dwrite: IDWriteFactory5 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let loader = dwrite.CreateInMemoryFontFileLoader()?;
            dwrite.RegisterFontFileLoader(&loader)?;
            let builder = dwrite.CreateFontSetBuilder()?;
            for data in SATOSHI {
                let file = loader.CreateInMemoryFontFileReference(
                    &dwrite,
                    data.as_ptr() as _,
                    data.len() as u32,
                    None,
                )?;
                builder.AddFontFile(&file)?;
            }
            let set = builder.CreateFontSet()?;
            let fonts = dwrite.CreateFontCollectionFromFontSet(&set)?;
            let brush = |c: u32, a: f32| dc.CreateSolidColorBrush(&color(c, a), None);
            let brushes = Brushes {
                blackout: brush(fixture::BLACKOUT, 1.0)?,
                surface: brush(fixture::SURFACE, 1.0)?,
                border: brush(fixture::BORDER, 1.0)?,
                text: brush(fixture::TEXT, 1.0)?,
                muted: brush(fixture::MUTED, 1.0)?,
                terracotta: brush(fixture::TERRACOTTA, 1.0)?,
                terracotta_30: brush(fixture::TERRACOTTA, 0.3)?,
                mention_bg: brush(fixture::TERRACOTTA, 0.24)?,
                green: brush(fixture::GREEN, 1.0)?,
                link: brush(fixture::LINK, 1.0)?,
                selection: brush(0x3A6EA5, 0.55)?,
                authors: [0x637A43, 0xB64D32, 0x4D6FB6, 0x8A5BB0, 0xC29A3A, 0x3A9C9A]
                    .iter()
                    .map(|c| brush(*c, 1.0))
                    .collect::<Result<_>>()?,
            };
            let fmt = |size: f32, weight: DWRITE_FONT_WEIGHT| -> Result<IDWriteTextFormat> {
                let f = dwrite.CreateTextFormat(
                    w!("Satoshi"),
                    &fonts,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("en-us"),
                )?;
                Ok(f)
            };
            let initials = fmt(13.0, DWRITE_FONT_WEIGHT_BOLD)?;
            initials.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            initials.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            let rail = fmt(13.0, DWRITE_FONT_WEIGHT_BOLD)?;
            rail.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            rail.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            let toolbar = fmt(14.0, DWRITE_FONT_WEIGHT_NORMAL)?;
            toolbar.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            toolbar.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            let formats = Formats {
                message: fmt(14.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                name: fmt(13.0, DWRITE_FONT_WEIGHT_BOLD)?,
                time: fmt(10.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                initials,
                reaction: fmt(12.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                channel: fmt(14.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                title: fmt(15.0, DWRITE_FONT_WEIGHT_BOLD)?,
                header: fmt(16.0, DWRITE_FONT_WEIGHT_BOLD)?,
                members_title: fmt(12.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                member: fmt(13.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                rail,
                composer: fmt(14.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                toolbar,
            };
            for f in [
                &formats.name,
                &formats.time,
                &formats.reaction,
                &formats.channel,
                &formats.title,
                &formats.header,
                &formats.members_title,
                &formats.member,
            ] {
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            }
            Ok(Self {
                d3d,
                d2d,
                dc,
                dcomp,
                dwrite,
                fonts,
                brushes,
                formats,
            })
        }
    }

    pub fn layout(
        &self,
        text: &[u16],
        format: &IDWriteTextFormat,
        max_width: f32,
    ) -> Result<IDWriteTextLayout> {
        unsafe {
            self.dwrite
                .CreateTextLayout(text, format, max_width, 100_000.0)
        }
    }

    pub fn surface(&self, w: u32, h: u32) -> Result<IDCompositionSurface> {
        unsafe {
            self.dcomp.CreateSurface(
                w.max(1),
                h.max(1),
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_ALPHA_MODE_PREMULTIPLIED,
            )
        }
    }

    pub fn visual(&self) -> Result<IDCompositionVisual2> {
        unsafe { self.dcomp.CreateVisual() }
    }
}

/// Draws into `rect_px` of a composition surface. The closure works in DIPs relative to the
/// surface origin; `scale` maps DIPs to physical pixels.
pub fn draw<S: Interface>(
    surface: &S,
    rect_px: Option<RECT>,
    scale: f32,
    f: impl FnOnce(&ID2D1DeviceContext) -> Result<()>,
) -> Result<()> {
    unsafe {
        let s: IDCompositionSurface = surface.cast()?;
        let mut offset = POINT::default();
        let dc: ID2D1DeviceContext = match &rect_px {
            Some(r) => s.BeginDraw(Some(r as *const RECT), &mut offset)?,
            None => s.BeginDraw(None, &mut offset)?,
        };
        let (l, t) = rect_px.map_or((0, 0), |r| (r.left, r.top));
        dc.SetDpi(96.0, 96.0);
        dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        dc.SetTransform(&Matrix3x2 {
            M11: scale,
            M12: 0.0,
            M21: 0.0,
            M22: scale,
            M31: (offset.x - l) as f32,
            M32: (offset.y - t) as f32,
        });
        let clip = rect_px.map(|r| {
            rect(
                r.left as f32 / scale,
                r.top as f32 / scale,
                r.right as f32 / scale,
                r.bottom as f32 / scale,
            )
        });
        if let Some(c) = &clip {
            dc.PushAxisAlignedClip(c, D2D1_ANTIALIAS_MODE_ALIASED);
        }
        let result = f(&dc);
        if clip.is_some() {
            dc.PopAxisAlignedClip();
        }
        s.EndDraw()?;
        result
    }
}

/// Draws into `r` (physical pixels) of a (virtual) surface with DIP coordinates whose origin is
/// the top-left of `r`, so huge surface coordinates never reach Direct2D's float math.
pub fn draw_local<S: Interface>(
    surface: &S,
    r: RECT,
    scale: f32,
    f: impl FnOnce(&ID2D1DeviceContext) -> Result<()>,
) -> Result<()> {
    unsafe {
        let s: IDCompositionSurface = surface.cast()?;
        let mut offset = POINT::default();
        let tb = std::time::Instant::now();
        let dc: ID2D1DeviceContext = s.BeginDraw(Some(&r as *const RECT), &mut offset)?;
        crate::SUB[2].fetch_add(
            tb.elapsed().as_micros() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        dc.SetDpi(96.0, 96.0);
        dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        dc.SetTransform(&Matrix3x2 {
            M11: scale,
            M12: 0.0,
            M21: 0.0,
            M22: scale,
            M31: offset.x as f32,
            M32: offset.y as f32,
        });
        dc.PushAxisAlignedClip(
            &rect(
                0.0,
                0.0,
                (r.right - r.left) as f32 / scale,
                (r.bottom - r.top) as f32 / scale,
            ),
            D2D1_ANTIALIAS_MODE_ALIASED,
        );
        let result = f(&dc);
        dc.PopAxisAlignedClip();
        let te = std::time::Instant::now();
        s.EndDraw()?;
        crate::SUB[3].fetch_add(
            te.elapsed().as_micros() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        result
    }
}

pub fn fill_round(
    dc: &ID2D1DeviceContext,
    r: D2D_RECT_F,
    radius: f32,
    brush: &ID2D1SolidColorBrush,
) {
    unsafe {
        dc.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: r,
                radiusX: radius,
                radiusY: radius,
            },
            brush,
        )
    }
}

pub fn stroke_round(
    dc: &ID2D1DeviceContext,
    r: D2D_RECT_F,
    radius: f32,
    brush: &ID2D1SolidColorBrush,
) {
    unsafe {
        let r = rect(r.left + 0.5, r.top + 0.5, r.right - 0.5, r.bottom - 0.5);
        dc.DrawRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: r,
                radiusX: radius,
                radiusY: radius,
            },
            brush,
            1.0,
            None,
        )
    }
}

/// Draws a layout through `Renderer`. D2D's own DrawTextLayout with ENABLE_COLOR_FONT sends every
/// glyph run down the color-glyph path, which made a freshly re-wrapped viewport cost ~3.5 ms;
/// here only runs whose font face actually has color glyphs (emoji) take that path.
pub fn text(
    dc: &ID2D1DeviceContext,
    layout: &IDWriteTextLayout,
    x: f32,
    y: f32,
    brush: &ID2D1SolidColorBrush,
) {
    unsafe {
        let r: IDWriteTextRenderer = Renderer {
            dc: dc.clone(),
            dc4: dc.cast().ok(),
            brush: brush.cast().unwrap(),
        }
        .into();
        let _ = layout.Draw(None, &r, x, y);
    }
}

#[implement(IDWriteTextRenderer)]
struct Renderer {
    dc: ID2D1DeviceContext,
    dc4: Option<ID2D1DeviceContext7>,
    brush: ID2D1Brush,
}

impl Renderer {
    fn brush(&self, effect: Ref<IUnknown>) -> ID2D1Brush {
        effect
            .ok()
            .ok()
            .and_then(|e| e.cast().ok())
            .unwrap_or_else(|| self.brush.clone())
    }
}

impl IDWritePixelSnapping_Impl for Renderer_Impl {
    fn IsPixelSnappingDisabled(&self, _: *const core::ffi::c_void) -> Result<BOOL> {
        Ok(false.into())
    }

    fn GetCurrentTransform(
        &self,
        _: *const core::ffi::c_void,
        t: *mut DWRITE_MATRIX,
    ) -> Result<()> {
        unsafe {
            let mut m = Matrix3x2::default();
            self.dc.GetTransform(&mut m);
            *t = DWRITE_MATRIX {
                m11: m.M11,
                m12: m.M12,
                m21: m.M21,
                m22: m.M22,
                dx: m.M31,
                dy: m.M32,
            };
        }
        Ok(())
    }

    fn GetPixelsPerDip(&self, _: *const core::ffi::c_void) -> Result<f32> {
        Ok(1.0)
    }
}

impl IDWriteTextRenderer_Impl for Renderer_Impl {
    fn DrawGlyphRun(
        &self,
        _: *const core::ffi::c_void,
        x: f32,
        y: f32,
        mode: DWRITE_MEASURING_MODE,
        run: *const DWRITE_GLYPH_RUN,
        desc: *const DWRITE_GLYPH_RUN_DESCRIPTION,
        effect: Ref<IUnknown>,
    ) -> Result<()> {
        unsafe {
            let brush = self.brush(effect);
            let color = (*run)
                .fontFace
                .as_ref()
                .and_then(|f| f.cast::<IDWriteFontFace2>().ok())
                .is_some_and(|f| f.IsColorFont().as_bool());
            let o = windows_numerics::Vector2 { X: x, Y: y };
            if color && self.dc4.is_some() {
                draw_emoji_run(&self.dc, x, y, &*run);
            } else {
                self.dc.DrawGlyphRun(o, run, Some(desc), &brush, mode);
            }
        }
        Ok(())
    }

    fn DrawUnderline(
        &self,
        _: *const core::ffi::c_void,
        x: f32,
        y: f32,
        u: *const DWRITE_UNDERLINE,
        effect: Ref<IUnknown>,
    ) -> Result<()> {
        unsafe {
            let u = &*u;
            self.dc.FillRectangle(
                &rect(
                    x,
                    y + u.offset,
                    x + u.width,
                    y + u.offset + u.thickness.max(1.0),
                ),
                &self.brush(effect),
            );
        }
        Ok(())
    }

    fn DrawStrikethrough(
        &self,
        _: *const core::ffi::c_void,
        _: f32,
        _: f32,
        _: *const DWRITE_STRIKETHROUGH,
        _: Ref<IUnknown>,
    ) -> Result<()> {
        Ok(())
    }

    fn DrawInlineObject(
        &self,
        _: *const core::ffi::c_void,
        _: f32,
        _: f32,
        _: Ref<IDWriteInlineObject>,
        _: BOOL,
        _: BOOL,
        _: Ref<IUnknown>,
    ) -> Result<()> {
        Ok(())
    }
}

type EmojiKey = (usize, u16, u32, u32);

struct EmojiCache {
    dc: ID2D1DeviceContext7,
    brush: ID2D1SolidColorBrush,
    glyphs: std::collections::HashMap<EmojiKey, Option<(ID2D1Bitmap1, f32, f32)>>,
}

thread_local! {
    static EMOJI: std::cell::RefCell<Option<EmojiCache>> = const { std::cell::RefCell::new(None) };
}

/// Color glyphs (COLRv1 emoji) are rasterized by D2D on every draw with no cache, which cost
/// ~3.5 ms per freshly drawn viewport; this rasterizes each (face, glyph, size, scale) once
/// into a bitmap and blits it afterwards, like an atlas.
unsafe fn draw_emoji_run(dc: &ID2D1DeviceContext, x: f32, y: f32, run: &DWRITE_GLYPH_RUN) {
    unsafe {
        let Some(face) = run.fontFace.as_ref() else {
            return;
        };
        let mut m = Matrix3x2::default();
        dc.GetTransform(&mut m);
        let scale = m.M11.abs().max(0.01);
        let em = run.fontEmSize;
        let rtl = run.bidiLevel % 2 == 1;
        let mut pen = x;
        EMOJI.with(|cell| {
            let mut cell = cell.borrow_mut();
            if cell.is_none() {
                let Ok(device) = dc.GetDevice() else { return };
                let Ok(rdc) = device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE) else {
                    return;
                };
                let Ok(rdc) = rdc.cast::<ID2D1DeviceContext7>() else {
                    return;
                };
                let Ok(brush) = rdc.CreateSolidColorBrush(&color(fixture::TEXT, 1.0), None) else {
                    return;
                };
                *cell = Some(EmojiCache {
                    dc: rdc,
                    brush,
                    glyphs: Default::default(),
                });
            }
            let cache = cell.as_mut().unwrap();
            for i in 0..run.glyphCount as usize {
                let glyph = *run.glyphIndices.add(i);
                let adv = if run.glyphAdvances.is_null() {
                    em
                } else {
                    *run.glyphAdvances.add(i)
                };
                let (ox, oy) = if run.glyphOffsets.is_null() {
                    (0.0, 0.0)
                } else {
                    let o = *run.glyphOffsets.add(i);
                    (o.advanceOffset, o.ascenderOffset)
                };
                if rtl {
                    pen -= adv;
                }
                let key = (face.as_raw() as usize, glyph, em.to_bits(), scale.to_bits());
                let entry = cache.glyphs.entry(key).or_insert_with(|| {
                    raster_emoji(&cache.dc, &cache.brush, face, glyph, em, adv, scale)
                });
                if let Some((bmp, w, h)) = entry {
                    let left = pen + ox - 0.1 * em;
                    let top = y - oy - 1.1 * em;
                    let dest = rect(left, top, left + *w / scale, top + *h / scale);
                    dc.DrawBitmap(
                        &*bmp,
                        Some(&dest),
                        1.0,
                        D2D1_INTERPOLATION_MODE_LINEAR,
                        None,
                        None,
                    );
                }
                if !rtl {
                    pen += adv;
                }
            }
        });
    }
}

unsafe fn raster_emoji(
    rdc: &ID2D1DeviceContext7,
    brush: &ID2D1SolidColorBrush,
    face: &IDWriteFontFace,
    glyph: u16,
    em: f32,
    adv: f32,
    scale: f32,
) -> Option<(ID2D1Bitmap1, f32, f32)> {
    unsafe {
        let (w, h) = (
            ((adv.max(em) * 1.2) * scale).ceil(),
            (em * 1.45 * scale).ceil(),
        );
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        let bmp = rdc
            .CreateBitmap(
                D2D_SIZE_U {
                    width: w as u32,
                    height: h as u32,
                },
                None,
                0,
                &props,
            )
            .ok()?;
        rdc.SetTarget(&bmp);
        rdc.SetDpi(96.0, 96.0);
        rdc.BeginDraw();
        rdc.Clear(Some(&D2D1_COLOR_F::default()));
        rdc.SetTransform(&Matrix3x2 {
            M11: scale,
            M12: 0.0,
            M21: 0.0,
            M22: scale,
            M31: 0.0,
            M32: 0.0,
        });
        let advance = adv;
        let run = DWRITE_GLYPH_RUN {
            fontFace: std::mem::ManuallyDrop::new(Some(face.clone())),
            fontEmSize: em,
            glyphCount: 1,
            glyphIndices: &glyph,
            glyphAdvances: &advance,
            glyphOffsets: std::ptr::null(),
            isSideways: false.into(),
            bidiLevel: 0,
        };
        rdc.DrawGlyphRunWithColorSupport(
            windows_numerics::Vector2 {
                X: 0.1 * em,
                Y: 1.1 * em,
            },
            &run,
            None,
            brush,
            None,
            0,
            DWRITE_MEASURING_MODE_NATURAL,
            D2D1_COLOR_BITMAP_GLYPH_SNAP_OPTION_DEFAULT,
        );
        let ok = rdc.EndDraw(None, None).is_ok();
        drop(std::mem::ManuallyDrop::into_inner(run.fontFace));
        rdc.SetTarget(None);
        ok.then_some((bmp, w, h))
    }
}

pub fn metrics(layout: &IDWriteTextLayout) -> DWRITE_TEXT_METRICS {
    let mut m = DWRITE_TEXT_METRICS::default();
    unsafe {
        let _ = layout.GetMetrics(&mut m);
    }
    m
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
