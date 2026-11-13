//! Drawing DirectWrite layouts through our own text renderer.
//!
//! Direct2D's `DrawTextLayout` with colour fonts enabled sends every glyph run
//! down the colour-glyph path, which made a freshly wrapped viewport cost about
//! 3.5 ms. Here plain runs go straight to `DrawGlyphRun`; only runs whose face
//! really has colour glyphs (emoji the Twemoji catalog lacks, which fall back
//! to Segoe UI Emoji) take the colour path, and each such glyph is rasterized
//! once per (face, glyph, size, scale) into a cached bitmap. Twemoji emoji are
//! inline objects (`emoji::Inline`) and draw their sheet cell.
use crate::gfx::{color, rect};
use std::cell::RefCell;
use std::collections::HashMap;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::core::*;
use windows_numerics::{Matrix3x2, Vector2};

thread_local! {
    static CURRENT: RefCell<Option<ID2D1DeviceContext>> = const { RefCell::new(None) };
    static COLOR_GLYPHS: RefCell<Option<ColorGlyphs>> = const { RefCell::new(None) };
}

/// The device context of the layout being drawn, for inline objects.
pub fn with_current_dc(f: impl FnOnce(&ID2D1DeviceContext)) {
    CURRENT.with(|c| {
        if let Some(dc) = c.borrow().as_ref() {
            f(dc);
        }
    });
}

/// Draws `layout` at (x, y) with `brush` as the default colour. Ranges with a
/// drawing effect (a brush) use that brush instead; a fully transparent one
/// (`Brushes::hidden`, a hidden spoiler) draws nothing there, emoji included.
pub fn draw(
    dc: &ID2D1DeviceContext,
    layout: &IDWriteTextLayout,
    x: f32,
    y: f32,
    brush: &ID2D1SolidColorBrush,
) {
    let Ok(brush) = brush.cast() else { return };
    let previous = CURRENT.with(|c| c.replace(Some(dc.clone())));
    let r: IDWriteTextRenderer = Renderer {
        dc: dc.clone(),
        color: dc.cast().ok(),
        brush,
    }
    .into();
    unsafe {
        let _ = layout.Draw(None, &r, x, y);
    }
    CURRENT.with(|c| *c.borrow_mut() = previous);
}

#[implement(IDWriteTextRenderer)]
struct Renderer {
    dc: ID2D1DeviceContext,
    color: Option<ID2D1DeviceContext7>,
    brush: ID2D1Brush,
}

impl Renderer {
    fn brush(&self, effect: &Ref<IUnknown>) -> ID2D1Brush {
        effect
            .ok()
            .ok()
            .and_then(|e| e.cast().ok())
            .unwrap_or_else(|| self.brush.clone())
    }
}

/// Whether a range's drawing effect hides it (a brush at opacity 0).
fn hidden(effect: &Ref<IUnknown>) -> bool {
    effect
        .ok()
        .ok()
        .and_then(|e| e.cast::<ID2D1Brush>().ok())
        .is_some_and(|b| unsafe { b.GetOpacity() } <= 0.0)
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
        let mut m = Matrix3x2::default();
        unsafe {
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
        if hidden(&effect) {
            return Ok(());
        }
        unsafe {
            let run = &*run;
            let is_color = run
                .fontFace
                .as_ref()
                .and_then(|f| f.cast::<IDWriteFontFace2>().ok())
                .is_some_and(|f| f.IsColorFont().as_bool());
            if is_color && self.color.is_some() {
                draw_color_run(&self.dc, x, y, run);
            } else {
                self.dc.DrawGlyphRun(
                    Vector2 { X: x, Y: y },
                    run,
                    Some(desc),
                    &self.brush(&effect),
                    mode,
                );
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
        if hidden(&effect) {
            return Ok(());
        }
        unsafe {
            let u = &*u;
            self.dc.FillRectangle(
                &rect(
                    x,
                    y + u.offset,
                    x + u.width,
                    y + u.offset + u.thickness.max(1.0),
                ),
                &self.brush(&effect),
            );
        }
        Ok(())
    }

    fn DrawStrikethrough(
        &self,
        _: *const core::ffi::c_void,
        x: f32,
        y: f32,
        s: *const DWRITE_STRIKETHROUGH,
        effect: Ref<IUnknown>,
    ) -> Result<()> {
        if hidden(&effect) {
            return Ok(());
        }
        unsafe {
            let s = &*s;
            self.dc.FillRectangle(
                &rect(
                    x,
                    y + s.offset,
                    x + s.width,
                    y + s.offset + s.thickness.max(1.0),
                ),
                &self.brush(&effect),
            );
        }
        Ok(())
    }

    fn DrawInlineObject(
        &self,
        context: *const core::ffi::c_void,
        x: f32,
        y: f32,
        object: Ref<IDWriteInlineObject>,
        sideways: BOOL,
        rtl: BOOL,
        effect: Ref<IUnknown>,
    ) -> Result<()> {
        if hidden(&effect) {
            return Ok(());
        }
        let object = object.ok()?;
        let me: IDWriteTextRenderer = self.to_interface();
        unsafe {
            object.Draw(
                Some(context),
                &me,
                x,
                y,
                sideways.as_bool(),
                rtl.as_bool(),
                effect.as_ref(),
            )
        }
    }
}

type GlyphKey = (usize, u16, u32, u32);

struct ColorGlyphs {
    dc: ID2D1DeviceContext7,
    brush: ID2D1SolidColorBrush,
    glyphs: HashMap<GlyphKey, Option<(ID2D1Bitmap1, f32, f32)>>,
}

/// Blits each glyph of a colour-font run from the cache, rasterizing misses.
unsafe fn draw_color_run(dc: &ID2D1DeviceContext, x: f32, y: f32, run: &DWRITE_GLYPH_RUN) {
    let Some(face) = run.fontFace.as_ref() else {
        return;
    };
    let mut m = Matrix3x2::default();
    unsafe { dc.GetTransform(&mut m) };
    let scale = m.M11.abs().max(0.01);
    let em = run.fontEmSize;
    let rtl = run.bidiLevel % 2 == 1;
    let mut pen = x;
    COLOR_GLYPHS.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.is_none() {
            let setup = || -> Result<ColorGlyphs> {
                let rdc = unsafe {
                    dc.GetDevice()?
                        .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?
                }
                .cast::<ID2D1DeviceContext7>()?;
                let brush =
                    unsafe { rdc.CreateSolidColorBrush(&color(crate::theme::TEXT, 1.0), None)? };
                Ok(ColorGlyphs {
                    dc: rdc,
                    brush,
                    glyphs: HashMap::new(),
                })
            };
            let Ok(c) = setup() else { return };
            *cell = Some(c);
        }
        let Some(cache) = cell.as_mut() else { return };
        for i in 0..run.glyphCount as usize {
            let (glyph, adv, (ox, oy)) = unsafe {
                (
                    *run.glyphIndices.add(i),
                    if run.glyphAdvances.is_null() {
                        em
                    } else {
                        *run.glyphAdvances.add(i)
                    },
                    if run.glyphOffsets.is_null() {
                        (0.0, 0.0)
                    } else {
                        let o = *run.glyphOffsets.add(i);
                        (o.advanceOffset, o.ascenderOffset)
                    },
                )
            };
            if rtl {
                pen -= adv;
            }
            let key = (face.as_raw() as usize, glyph, em.to_bits(), scale.to_bits());
            let entry = cache.glyphs.entry(key).or_insert_with(|| unsafe {
                raster(&cache.dc, &cache.brush, face, glyph, em, adv, scale)
            });
            if let Some((bmp, w, h)) = entry {
                let left = pen + ox - 0.1 * em;
                let top = y - oy - 1.1 * em;
                unsafe {
                    dc.DrawBitmap(
                        &*bmp,
                        Some(&rect(left, top, left + *w / scale, top + *h / scale)),
                        1.0,
                        D2D1_INTERPOLATION_MODE_LINEAR,
                        None,
                        None,
                    )
                };
            }
            if !rtl {
                pen += adv;
            }
        }
    });
}

unsafe fn raster(
    rdc: &ID2D1DeviceContext7,
    brush: &ID2D1SolidColorBrush,
    face: &IDWriteFontFace,
    glyph: u16,
    em: f32,
    adv: f32,
    scale: f32,
) -> Option<(ID2D1Bitmap1, f32, f32)> {
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
    unsafe {
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
        let run = DWRITE_GLYPH_RUN {
            fontFace: std::mem::ManuallyDrop::new(Some(face.clone())),
            fontEmSize: em,
            glyphCount: 1,
            glyphIndices: &glyph,
            glyphAdvances: &adv,
            glyphOffsets: std::ptr::null(),
            isSideways: false.into(),
            bidiLevel: 0,
        };
        rdc.DrawGlyphRunWithColorSupport(
            Vector2 {
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
