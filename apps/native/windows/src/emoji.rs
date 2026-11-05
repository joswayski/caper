//! Twemoji artwork for emoji in text, from the shared sheets
//! (`shared/emoji`), matched per grapheme exactly like the egui client's
//! `emoji::inline`. Each match becomes a DirectWrite inline object that
//! draws its sheet cell, so emoji look the same on every client and never go
//! through the colour-font rasterizer.
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::OnceLock;
use unicode_segmentation::UnicodeSegmentation;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::core::*;

#[derive(Debug, Deserialize)]
pub struct Entry {
    pub id: String,
    pub emoji: String,
    pub name: String,
    pub sheet: usize,
    pub x: u32,
    pub y: u32,
}

pub const CELL: u32 = 64;

pub fn catalog() -> &'static [Entry] {
    static CATALOG: OnceLock<Vec<Entry>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_slice(include_bytes!("../../../../shared/emoji/catalog.json"))
            .expect("embedded emoji catalog is valid")
    })
}

const SHEETS: [&[u8]; 15] = [
    include_bytes!("../../../../shared/emoji/sheet-0.png"),
    include_bytes!("../../../../shared/emoji/sheet-1.png"),
    include_bytes!("../../../../shared/emoji/sheet-2.png"),
    include_bytes!("../../../../shared/emoji/sheet-3.png"),
    include_bytes!("../../../../shared/emoji/sheet-4.png"),
    include_bytes!("../../../../shared/emoji/sheet-5.png"),
    include_bytes!("../../../../shared/emoji/sheet-6.png"),
    include_bytes!("../../../../shared/emoji/sheet-7.png"),
    include_bytes!("../../../../shared/emoji/sheet-8.png"),
    include_bytes!("../../../../shared/emoji/sheet-9.png"),
    include_bytes!("../../../../shared/emoji/sheet-10.png"),
    include_bytes!("../../../../shared/emoji/sheet-11.png"),
    include_bytes!("../../../../shared/emoji/sheet-12.png"),
    include_bytes!("../../../../shared/emoji/sheet-13.png"),
    include_bytes!("../../../../shared/emoji/sheet-14.png"),
];

pub fn unicode_id(value: &str) -> String {
    let parts: Vec<String> = value.chars().map(|c| format!("{:x}", c as u32)).collect();
    if parts.iter().any(|p| p == "200d") {
        parts.join("-")
    } else {
        parts
            .into_iter()
            .filter(|p| p != "fe0f")
            .collect::<Vec<_>>()
            .join("-")
    }
}

/// The catalog by Twemoji id, and by its emoji without U+FE0F.
fn index() -> &'static HashMap<String, usize> {
    static INDEX: OnceLock<HashMap<String, usize>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut index = HashMap::new();
        for (i, e) in catalog().iter().enumerate() {
            index.entry(e.id.clone()).or_insert(i);
            index.entry(e.emoji.replace('\u{fe0f}', "")).or_insert(i);
        }
        index
    })
}

/// Catalog emoji that are one character (no U+FE0F), by that character.
fn singles() -> &'static HashMap<char, usize> {
    static SINGLES: OnceLock<HashMap<char, usize>> = OnceLock::new();
    SINGLES.get_or_init(|| {
        let mut singles = HashMap::new();
        for (i, e) in catalog().iter().enumerate() {
            let mut cs = e.emoji.chars();
            if let (Some(c), None) = (cs.next(), cs.next()) {
                singles.entry(c).or_insert(i);
            }
        }
        singles
    })
}

/// UTF-16 ranges of the emoji graphemes in `text`, with their artwork. Plain
/// ASCII is never an emoji, and a text-default character (© ™ ♥ ➡) without
/// its U+FE0F stays text, like web.
pub fn inline(text: &str) -> Vec<(Range<usize>, &'static Entry)> {
    if text.is_ascii() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut pos16 = 0;
    for grapheme in text.graphemes(true) {
        let len16 = grapheme.encode_utf16().count();
        if !grapheme.is_ascii() {
            let mut cs = grapheme.chars();
            let entry = if let (Some(c), None) = (cs.next(), cs.next()) {
                singles().get(&c)
            } else {
                index()
                    .get(&unicode_id(grapheme))
                    .or_else(|| index().get(&grapheme.replace('\u{fe0f}', "")))
            };
            if let Some(&i) = entry {
                found.push((pos16..pos16 + len16, &catalog()[i]));
            }
        }
        pos16 += len16;
    }
    found
}

/// Emoji artwork in use, copied cell by cell out of the sheets into small
/// atlas pages on the GPU: a whole sheet (4 MiB decoded) is never kept, only
/// the 64 px cells that were drawn.
struct Atlas {
    wic: IWICImagingFactory,
    pages: Vec<ID2D1Bitmap1>,
    cells: HashMap<(usize, u32, u32), Option<(usize, u32, u32)>>,
    next: u32,
}

const PAGE: u32 = 512;
const PER_PAGE: u32 = (PAGE / CELL) * (PAGE / CELL);

thread_local! {
    static ATLAS: RefCell<Option<Atlas>> = const { RefCell::new(None) };
}

/// Decodes only `entry`'s cell of its sheet (premultiplied BGRA).
fn decode_cell(wic: &IWICImagingFactory, entry: &Entry) -> Option<Vec<u8>> {
    unsafe {
        let stream = wic.CreateStream().ok()?;
        stream.InitializeFromMemory(SHEETS[entry.sheet]).ok()?;
        let decoder = wic
            .CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
            .ok()?;
        let converter = wic.CreateFormatConverter().ok()?;
        converter
            .Initialize(
                &decoder.GetFrame(0).ok()?,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .ok()?;
        let mut px = vec![0u8; (CELL * CELL * 4) as usize];
        let r = WICRect {
            X: entry.x as i32,
            Y: entry.y as i32,
            Width: CELL as i32,
            Height: CELL as i32,
        };
        converter.CopyPixels(&r, CELL * 4, &mut px).ok()?;
        Some(px)
    }
}

impl Atlas {
    /// The atlas page and position of `entry`'s cell, copying it in on first use.
    fn cell(&mut self, dc: &ID2D1DeviceContext, entry: &Entry) -> Option<(usize, u32, u32)> {
        let key = (entry.sheet, entry.x, entry.y);
        if let Some(c) = self.cells.get(&key) {
            return *c;
        }
        let slot = self.decode_into(dc, entry);
        self.cells.insert(key, slot);
        slot
    }

    fn decode_into(&mut self, dc: &ID2D1DeviceContext, entry: &Entry) -> Option<(usize, u32, u32)> {
        let px = decode_cell(&self.wic, entry)?;
        let (page, i) = ((self.next / PER_PAGE) as usize, self.next % PER_PAGE);
        if page == self.pages.len() {
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
            self.pages.push(unsafe {
                dc.CreateBitmap(
                    D2D_SIZE_U {
                        width: PAGE,
                        height: PAGE,
                    },
                    None,
                    0,
                    &props,
                )
                .ok()?
            });
        }
        let (x, y) = ((i % (PAGE / CELL)) * CELL, (i / (PAGE / CELL)) * CELL);
        let dst = D2D_RECT_U {
            left: x,
            top: y,
            right: x + CELL,
            bottom: y + CELL,
        };
        unsafe {
            self.pages[page]
                .CopyFromMemory(Some(&dst), px.as_ptr().cast(), CELL * 4)
                .ok()?
        };
        self.next += 1;
        Some((page, x, y))
    }
}

/// Draws `entry`'s artwork into `dest` (DIPs).
pub fn draw(dc: &ID2D1DeviceContext, entry: &Entry, dest: D2D_RECT_F) {
    let found = ATLAS.with(|a| {
        let mut a = a.borrow_mut();
        if a.is_none() {
            let wic = unsafe {
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?
            };
            *a = Some(Atlas {
                wic,
                pages: Vec::new(),
                cells: HashMap::new(),
                next: 0,
            });
        }
        let atlas = a.as_mut()?;
        let (page, x, y) = atlas.cell(dc, entry)?;
        Some((atlas.pages[page].clone(), x, y))
    });
    if let Some((bmp, x, y)) = found {
        let src = D2D_RECT_F {
            left: x as f32,
            top: y as f32,
            right: (x + CELL) as f32,
            bottom: (y + CELL) as f32,
        };
        unsafe {
            dc.DrawBitmap(
                &bmp,
                Some(&dest),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                Some(&src),
                None,
            )
        };
    }
}
/// One emoji inside a DirectWrite layout: a `size`-DIP square sitting on the
/// baseline like a capital letter, as wide as `size` plus `pad` either side.
#[implement(IDWriteInlineObject)]
pub struct Inline {
    pub entry: &'static Entry,
    pub size: f32,
    pub pad: f32,
    pub baseline: f32,
}

impl IDWriteInlineObject_Impl for Inline_Impl {
    fn Draw(
        &self,
        _: *const core::ffi::c_void,
        _: Ref<IDWriteTextRenderer>,
        x: f32,
        y: f32,
        _: BOOL,
        _: BOOL,
        _: Ref<IUnknown>,
    ) -> Result<()> {
        crate::text::with_current_dc(|dc| {
            draw(
                dc,
                self.entry,
                D2D_RECT_F {
                    left: x + self.pad,
                    top: y,
                    right: x + self.pad + self.size,
                    bottom: y + self.size,
                },
            )
        });
        Ok(())
    }

    fn GetMetrics(&self) -> Result<DWRITE_INLINE_OBJECT_METRICS> {
        Ok(DWRITE_INLINE_OBJECT_METRICS {
            width: self.size + 2.0 * self.pad,
            height: self.size,
            baseline: self.baseline,
            supportsSideways: false.into(),
        })
    }

    fn GetOverhangMetrics(&self) -> Result<DWRITE_OVERHANG_METRICS> {
        Ok(DWRITE_OVERHANG_METRICS::default())
    }

    fn GetBreakConditions(
        &self,
        before: *mut DWRITE_BREAK_CONDITION,
        after: *mut DWRITE_BREAK_CONDITION,
    ) -> Result<()> {
        unsafe {
            *before = DWRITE_BREAK_CONDITION_NEUTRAL;
            *after = DWRITE_BREAK_CONDITION_NEUTRAL;
        }
        Ok(())
    }
}

/// Replaces every emoji grapheme of `text` in `layout` with its artwork,
/// `scale` times the font size. Returns the matched ranges.
pub fn apply(
    layout: &IDWriteTextLayout,
    text: &str,
    font_size: f32,
    scale: f32,
) -> Result<Vec<Range<usize>>> {
    let size = font_size * scale;
    let mut out = Vec::new();
    for (r, entry) in inline(text) {
        // Sit the square so its centre is near the x-height middle of the text around it.
        let obj: IDWriteInlineObject = Inline {
            entry,
            size,
            pad: 0.5,
            baseline: size * 0.82,
        }
        .into();
        unsafe {
            layout.SetInlineObject(
                &obj,
                DWRITE_TEXT_RANGE {
                    startPosition: r.start as u32,
                    length: (r.end - r.start) as u32,
                },
            )?
        };
        out.push(r);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_like_the_egui_client() {
        // A skin-tone sequence and a ZWJ family are one emoji each, at their UTF-16 positions.
        let found = inline("hi 🙌🏽 and 👨‍👩‍👧!");
        let ranges: Vec<_> = found.iter().map(|(r, _)| r.clone()).collect();
        assert_eq!(ranges, vec![3..7, 12..20]);
        // Text-default characters stay text without U+FE0F, and become emoji with it.
        assert!(inline("© ™ ♥").is_empty());
        assert_eq!(inline("♥\u{fe0f}").len(), 1);
        // Digits and '#' are never emoji on their own.
        assert!(inline("#1 ünïcödé").is_empty());
    }
}
