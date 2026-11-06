//! Devices and shared drawing resources: a D3D11 device shared by Direct2D
//! and DirectComposition, DirectWrite with Satoshi in an in-memory font
//! collection, WIC for images, and the theme's brushes and text formats.
use crate::theme;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
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

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub struct Brushes {
    pub text: ID2D1SolidColorBrush,
    pub muted: ID2D1SolidColorBrush,
    pub border: ID2D1SolidColorBrush,
    pub raised: ID2D1SolidColorBrush,
    pub composer: ID2D1SolidColorBrush,
    pub hover: ID2D1SolidColorBrush,
    pub terracotta: ID2D1SolidColorBrush,
    pub selected: ID2D1SolidColorBrush,
    pub mention: ID2D1SolidColorBrush,
    pub caper: ID2D1SolidColorBrush,
    pub link: ID2D1SolidColorBrush,
    pub link_bg: ID2D1SolidColorBrush,
    pub selection: ID2D1SolidColorBrush,
    pub eggshell: ID2D1SolidColorBrush,
    pub blackout: ID2D1SolidColorBrush,
    /// Field placeholders: muted text at 65%.
    pub hint: ID2D1SolidColorBrush,
    pub error: ID2D1SolidColorBrush,
    /// Your reaction chips: pressed fill and light terracotta label.
    pub pressed: ID2D1SolidColorBrush,
    pub own: ID2D1SolidColorBrush,
    /// Quote panels: `--surface` with a softened border.
    pub surface: ID2D1SolidColorBrush,
    pub border_soft: ID2D1SolidColorBrush,
    pub terracotta_bright: ID2D1SolidColorBrush,
}

pub struct Formats {
    pub message: IDWriteTextFormat,
    pub name: IDWriteTextFormat,
    pub time: IDWriteTextFormat,
    pub initials: IDWriteTextFormat,
    pub reaction: IDWriteTextFormat,
    pub channel: IDWriteTextFormat,
    pub section: IDWriteTextFormat,
    pub space: IDWriteTextFormat,
    pub header: IDWriteTextFormat,
    pub member: IDWriteTextFormat,
    pub rail: IDWriteTextFormat,
    pub composer: IDWriteTextFormat,
    pub toolbar: IDWriteTextFormat,
    /// Sign-in: the 40px Black heading, 16px body and field text, 14px
    /// labels and notes, the 16px Medium action, 13.6px links, 13px errors.
    pub title: IDWriteTextFormat,
    pub body: IDWriteTextFormat,
    pub field: IDWriteTextFormat,
    pub label: IDWriteTextFormat,
    pub note: IDWriteTextFormat,
    pub action: IDWriteTextFormat,
    pub link_small: IDWriteTextFormat,
    pub error: IDWriteTextFormat,
    /// A grouped row's "(edited)" marker.
    pub edited: IDWriteTextFormat,
    /// A reply's quote panel: its text (at most three lines, then an
    /// ellipsis), a deleted original's notice, the author, the small
    /// muted labels and the avatar initials.
    pub quote: IDWriteTextFormat,
    pub quote_deleted: IDWriteTextFormat,
    pub quote_name: IDWriteTextFormat,
    pub quote_meta: IDWriteTextFormat,
    pub quote_initials: IDWriteTextFormat,
    /// A thread root's "N replies" and "View thread".
    pub thread_count: IDWriteTextFormat,
    pub thread_view: IDWriteTextFormat,
    /// Dialogs: the title, the edit dialog's note and button labels.
    pub dialog_title: IDWriteTextFormat,
    pub dialog_note: IDWriteTextFormat,
    pub button: IDWriteTextFormat,
    /// The reaction picker's labels: egui's 12.5px body text.
    pub picker: IDWriteTextFormat,
    /// The message menu: its 12.48px bold heading and its items.
    pub menu_title: IDWriteTextFormat,
    pub menu_item: IDWriteTextFormat,
}

pub struct Gfx {
    pub d2d: ID2D1Device,
    pub dc: ID2D1DeviceContext,
    pub dcomp: IDCompositionDesktopDevice,
    pub dwrite: IDWriteFactory5,
    pub fonts: IDWriteFontCollection1,
    pub wic: IWICImagingFactory,
    pub brushes: Brushes,
    pub formats: Formats,
    solids: std::cell::RefCell<std::collections::HashMap<u32, ID2D1SolidColorBrush>>,
}

/// Satoshi, embedded unchanged (see shared/fonts/README.md).
const SATOSHI: [&[u8]; 4] = [
    include_bytes!("../../../../shared/fonts/cache/Satoshi-Regular.otf"),
    include_bytes!("../../../../shared/fonts/cache/Satoshi-Medium.otf"),
    include_bytes!("../../../../shared/fonts/cache/Satoshi-Bold.otf"),
    include_bytes!("../../../../shared/fonts/cache/Satoshi-Black.otf"),
];

/// The web app's wordmark, drawn with Direct2D's SVG renderer.
const WORDMARK: &[u8] = include_bytes!("../../../web/public/caper-wordmark.svg");

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
            let d3d: ID3D11Device = d3d.ok_or_else(|| Error::from(E_FAIL))?;
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
                    data.as_ptr().cast(),
                    data.len() as u32,
                    None,
                )?;
                builder.AddFontFile(&file)?;
            }
            let fonts = dwrite.CreateFontCollectionFromFontSet(&builder.CreateFontSet()?)?;
            let wic: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let brush = |c: u32, a: f32| dc.CreateSolidColorBrush(&color(c, a), None);
            let brushes = Brushes {
                text: brush(theme::TEXT, 1.0)?,
                muted: brush(theme::TEXT_MUTED, 1.0)?,
                border: brush(theme::BORDER, 1.0)?,
                raised: brush(theme::SURFACE_RAISED, 1.0)?,
                composer: brush(theme::SURFACE_COMPOSER, 1.0)?,
                hover: brush(theme::HOVER.0, theme::HOVER.1)?,
                terracotta: brush(theme::TERRACOTTA, 1.0)?,
                selected: brush(theme::TERRACOTTA, 0.3)?,
                mention: brush(theme::TERRACOTTA, 0.24)?,
                caper: brush(theme::CAPER, 1.0)?,
                link: brush(theme::LINK, 1.0)?,
                link_bg: brush(theme::LINK, 0.1)?,
                selection: brush(theme::SELECTION.0, theme::SELECTION.1)?,
                eggshell: brush(0xF1EDE4, 1.0)?,
                blackout: brush(theme::BLACKOUT, 1.0)?,
                hint: brush(theme::TEXT_MUTED, 0.65)?,
                error: brush(theme::ERROR, 1.0)?,
                pressed: brush(0x39231E, 1.0)?,
                own: brush(0xF2AE9C, 1.0)?,
                surface: brush(0x151719, 1.0)?,
                border_soft: brush(theme::BORDER, 0.7)?,
                terracotta_bright: brush(theme::TERRACOTTA_BRIGHT, 1.0)?,
            };
            let fmt = |size: f32, weight: DWRITE_FONT_WEIGHT| {
                dwrite.CreateTextFormat(
                    w!("Satoshi"),
                    &fonts,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("en-us"),
                )
            };
            let centered = |size: f32, weight: DWRITE_FONT_WEIGHT| -> Result<IDWriteTextFormat> {
                let f = fmt(size, weight)?;
                f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                Ok(f)
            };
            let formats = Formats {
                message: fmt(15.0, DWRITE_FONT_WEIGHT_MEDIUM)?,
                name: fmt(15.0, DWRITE_FONT_WEIGHT_BOLD)?,
                time: fmt(11.0, DWRITE_FONT_WEIGHT_MEDIUM)?,
                initials: centered(13.0, DWRITE_FONT_WEIGHT_BOLD)?,
                reaction: fmt(12.0, DWRITE_FONT_WEIGHT_MEDIUM)?,
                channel: fmt(15.0, DWRITE_FONT_WEIGHT_MEDIUM)?,
                section: fmt(13.0, DWRITE_FONT_WEIGHT_BOLD)?,
                space: fmt(17.0, DWRITE_FONT_WEIGHT_BOLD)?,
                header: fmt(16.0, DWRITE_FONT_WEIGHT_MEDIUM)?,
                member: fmt(14.0, DWRITE_FONT_WEIGHT_BOLD)?,
                rail: centered(13.0, DWRITE_FONT_WEIGHT_BOLD)?,
                composer: fmt(15.0, DWRITE_FONT_WEIGHT_MEDIUM)?,
                toolbar: centered(14.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                title: fmt(40.0, DWRITE_FONT_WEIGHT_BLACK)?,
                body: fmt(16.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                field: fmt(16.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                label: fmt(14.0, DWRITE_FONT_WEIGHT_BOLD)?,
                note: fmt(14.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                action: fmt(16.0, DWRITE_FONT_WEIGHT_MEDIUM)?,
                link_small: fmt(13.6, DWRITE_FONT_WEIGHT_NORMAL)?,
                error: fmt(13.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                edited: fmt(10.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                quote: fmt(12.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                quote_deleted: dwrite.CreateTextFormat(
                    w!("Satoshi"),
                    &fonts,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_ITALIC,
                    DWRITE_FONT_STRETCH_NORMAL,
                    12.0,
                    w!("en-us"),
                )?,
                quote_name: fmt(12.0, DWRITE_FONT_WEIGHT_BOLD)?,
                quote_meta: fmt(11.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                quote_initials: centered(7.0, DWRITE_FONT_WEIGHT_BOLD)?,
                thread_count: fmt(12.0, DWRITE_FONT_WEIGHT_BOLD)?,
                thread_view: fmt(12.0, DWRITE_FONT_WEIGHT_NORMAL)?,
                dialog_title: fmt(18.4, DWRITE_FONT_WEIGHT_BOLD)?,
                dialog_note: fmt(12.8, DWRITE_FONT_WEIGHT_NORMAL)?,
                button: fmt(12.0, DWRITE_FONT_WEIGHT_BOLD)?,
                picker: fmt(12.5, DWRITE_FONT_WEIGHT_NORMAL)?,
                menu_title: fmt(12.48, DWRITE_FONT_WEIGHT_BOLD)?,
                menu_item: fmt(12.48, DWRITE_FONT_WEIGHT_NORMAL)?,
            };
            let f = &formats;
            for f in [
                &f.field,
                &f.action,
                &f.link_small,
                &f.name,
                &f.time,
                &f.reaction,
                &f.channel,
                &f.section,
                &f.space,
                &f.header,
                &f.member,
                &f.quote_name,
                &f.quote_meta,
                &f.thread_count,
                &f.thread_view,
            ] {
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            }
            let sign = dwrite.CreateEllipsisTrimmingSign(&f.quote)?;
            f.quote.SetTrimming(
                &DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_WORD,
                    ..Default::default()
                },
                &sign,
            )?;
            // egui's line height for body text is 1.35 em; match it so wrapped paragraphs line up.
            for f in [&f.message, &f.composer] {
                f.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, 15.0 * 1.35, 15.0 * 1.05)?;
            }
            Ok(Self {
                d2d,
                dc,
                dcomp,
                dwrite,
                fonts,
                wic,
                brushes,
                formats,
                solids: Default::default(),
            })
        }
    }

    /// The wordmark as an SVG document scaled to `w`×`h` DIPs.
    pub fn wordmark(&self, w: f32, h: f32) -> Result<ID2D1SvgDocument> {
        unsafe {
            let stream = windows::Win32::UI::Shell::SHCreateMemStream(Some(WORDMARK))
                .ok_or_else(|| Error::from(E_OUTOFMEMORY))?;
            let dc: ID2D1DeviceContext5 = self.dc.cast()?;
            dc.CreateSvgDocument(
                &stream,
                D2D_SIZE_F {
                    width: w,
                    height: h,
                },
            )
        }
    }

    /// An opaque brush of colour c, created once.
    pub fn solid(&self, c: u32) -> ID2D1SolidColorBrush {
        self.solids
            .borrow_mut()
            .entry(c)
            .or_insert_with(|| unsafe {
                self.dc
                    .CreateSolidColorBrush(&color(c, 1.0), None)
                    .expect("solid brush")
            })
            .clone()
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

    /// Decodes an encoded image (PNG, JPEG, ...) into a premultiplied BGRA bitmap.
    pub fn decode(&self, bytes: &[u8]) -> Result<ID2D1Bitmap1> {
        unsafe {
            let stream = self.wic.CreateStream()?;
            stream.InitializeFromMemory(bytes)?;
            let decoder = self.wic.CreateDecoderFromStream(
                &stream,
                std::ptr::null(),
                WICDecodeMetadataCacheOnDemand,
            )?;
            let frame = decoder.GetFrame(0)?;
            let converter = self.wic.CreateFormatConverter()?;
            converter.Initialize(
                &frame,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )?;
            self.dc.CreateBitmapFromWicBitmap(&converter, None)
        }
    }

    /// A bitmap from premultiplied BGRA pixels.
    pub fn bitmap(&self, w: u32, h: u32, bgra: &[u8]) -> Result<ID2D1Bitmap1> {
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        unsafe {
            self.dc.CreateBitmap(
                D2D_SIZE_U {
                    width: w,
                    height: h,
                },
                Some(bgra.as_ptr().cast()),
                w * 4,
                &props,
            )
        }
    }
}

fn begin(dc: &ID2D1DeviceContext, scale: f32, dx: f32, dy: f32) {
    unsafe {
        dc.SetDpi(96.0, 96.0);
        dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        dc.SetTransform(&Matrix3x2 {
            M11: scale,
            M12: 0.0,
            M21: 0.0,
            M22: scale,
            M31: dx,
            M32: dy,
        });
    }
}

/// Draws a whole composition surface in DIPs (origin at its top-left).
pub fn draw(
    surface: &IDCompositionSurface,
    scale: f32,
    f: impl FnOnce(&ID2D1DeviceContext) -> Result<()>,
) -> Result<()> {
    unsafe {
        let mut offset = POINT::default();
        let dc: ID2D1DeviceContext = surface.BeginDraw(None, &mut offset)?;
        begin(&dc, scale, offset.x as f32, offset.y as f32);
        let result = f(&dc);
        surface.EndDraw()?;
        result
    }
}

/// Draws into `r` (physical pixels) of a surface with DIP coordinates whose
/// origin is the top-left of `r`, so huge virtual-surface coordinates never
/// reach Direct2D's float math. Only `r` is updated.
pub fn draw_local<S: Interface>(
    surface: &S,
    r: RECT,
    scale: f32,
    f: impl FnOnce(&ID2D1DeviceContext) -> Result<()>,
) -> Result<()> {
    unsafe {
        let s: IDCompositionSurface = surface.cast()?;
        let mut offset = POINT::default();
        let dc: ID2D1DeviceContext = s.BeginDraw(Some(&r), &mut offset)?;
        begin(&dc, scale, offset.x as f32, offset.y as f32);
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
        s.EndDraw()?;
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
    let r = rect(r.left + 0.5, r.top + 0.5, r.right - 0.5, r.bottom - 0.5);
    unsafe {
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

pub fn metrics(layout: &IDWriteTextLayout) -> DWRITE_TEXT_METRICS {
    let mut m = DWRITE_TEXT_METRICS::default();
    unsafe {
        let _ = layout.GetMetrics(&mut m);
    }
    m
}

/// Rectangles covering a UTF-16 range of a layout, relative to the layout origin.
pub fn range_rects(
    layout: &IDWriteTextLayout,
    start: u32,
    len: u32,
) -> Vec<DWRITE_HIT_TEST_METRICS> {
    unsafe {
        let mut n = 0u32;
        let _ = layout.HitTestTextRange(start, len, 0.0, 0.0, None, &mut n);
        let mut out = vec![DWRITE_HIT_TEST_METRICS::default(); n as usize];
        if n == 0
            || layout
                .HitTestTextRange(start, len, 0.0, 0.0, Some(&mut out), &mut n)
                .is_err()
        {
            return Vec::new();
        }
        out.truncate(n as usize);
        out
    }
}

/// UTF-16 position under a point relative to the layout origin.
pub fn hit_point(layout: &IDWriteTextLayout, x: f32, y: f32) -> u32 {
    let (mut trailing, mut inside) = (BOOL(0), BOOL(0));
    let mut m = DWRITE_HIT_TEST_METRICS::default();
    unsafe {
        let _ = layout.HitTestPoint(x, y, &mut trailing, &mut inside, &mut m);
    }
    m.textPosition + if trailing.as_bool() { m.length } else { 0 }
}
