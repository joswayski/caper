//! The egui client's Lucide icons (`apps/native/desktop/resources/icons`),
//! drawn as vectors with Direct2D's SVG renderer. Each icon and colour pair
//! is parsed once into an SVG document and reused.
use std::cell::RefCell;
use std::collections::HashMap;
use windows::Win32::Foundation::E_OUTOFMEMORY;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::core::*;
use windows_numerics::Matrix3x2;

macro_rules! icons {
    ($($name:ident => $file:literal,)*) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum Icon { $($name,)* }

        impl Icon {
            fn source(self) -> &'static str {
                match self {
                    $(Self::$name => include_str!(concat!("../../desktop/resources/icons/", $file)),)*
                }
            }
        }
    };
}

icons! {
    Check => "check.svg",
    Close => "x.svg",
    Copy => "copy.svg",
    Hash => "hash.svg",
    History => "history.svg",
    More => "ellipsis.svg",
    Pencil => "pencil.svg",
    Pin => "pin.svg",
    PinOff => "pin-off.svg",
    Reply => "reply.svg",
    Send => "send.svg",
    SmilePlus => "smile-plus.svg",
    Trash => "trash-2.svg",
}

thread_local! {
    static DOCS: RefCell<HashMap<(Icon, u32), ID2D1SvgDocument>> = RefCell::new(HashMap::new());
}

/// The 24-unit icon recoloured to `color` (0xRRGGBB, with `alpha`).
fn document(
    dc: &ID2D1DeviceContext5,
    icon: Icon,
    color: u32,
    alpha: f32,
) -> Result<ID2D1SvgDocument> {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
    let key = (icon, color << 8 | a);
    if let Some(d) = DOCS.with(|d| d.borrow().get(&key).cloned()) {
        return Ok(d);
    }
    let source = icon.source().replace(
        "stroke=\"#ffffff\"",
        &format!("stroke=\"#{color:06x}\" stroke-opacity=\"{alpha}\""),
    );
    let doc = unsafe {
        let stream = windows::Win32::UI::Shell::SHCreateMemStream(Some(source.as_bytes()))
            .ok_or_else(|| Error::from(E_OUTOFMEMORY))?;
        dc.CreateSvgDocument(
            &stream,
            D2D_SIZE_F {
                width: 24.0,
                height: 24.0,
            },
        )?
    };
    DOCS.with(|d| d.borrow_mut().insert(key, doc.clone()));
    Ok(doc)
}

/// Draws `icon` filling the square `r` (DIPs, in the context's current
/// transform) in colour `color` at opacity `alpha`.
pub fn draw(dc: &ID2D1DeviceContext, icon: Icon, r: D2D_RECT_F, color: u32, alpha: f32) {
    let Ok(dc5) = dc.cast::<ID2D1DeviceContext5>() else {
        return;
    };
    let Ok(doc) = document(&dc5, icon, color, alpha) else {
        return;
    };
    unsafe {
        let mut old = Matrix3x2::default();
        dc.GetTransform(&mut old);
        let k = (r.right - r.left) / 24.0;
        let local = Matrix3x2 {
            M11: k,
            M12: 0.0,
            M21: 0.0,
            M22: k,
            M31: r.left,
            M32: r.top,
        };
        dc.SetTransform(&(local * old));
        dc5.DrawSvgDocument(&doc);
        dc.SetTransform(&old);
    }
}
