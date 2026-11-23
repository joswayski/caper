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
    ($($name:ident => $file:literal,)* ; $($inline:ident => $svg:literal,)*) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum Icon { $($name,)* $($inline,)* }

        impl Icon {
            fn source(self) -> &'static str {
                match self {
                    $(Self::$name => include_str!(concat!("../../desktop/resources/icons/", $file)),)*
                    $(Self::$inline => $svg,)*
                }
            }
        }
    };
}

icons! {
    ArrowRight => "arrow-right.svg",
    AudioLines => "audio-lines.svg",
    Bell => "bell.svg",
    BellOff => "bell-off.svg",
    Check => "check.svg",
    ChevronDown => "chevron-down.svg",
    ChevronLeft => "chevron-left.svg",
    ChevronRight => "chevron-right.svg",
    Close => "x.svg",
    Copy => "copy.svg",
    Download => "download.svg",
    Eye => "eye.svg",
    ExternalLink => "external-link.svg",
    EyeOff => "eye-off.svg",
    FileText => "file-text.svg",
    Hash => "hash.svg",
    Headphones => "headphones.svg",
    History => "history.svg",
    Link => "link.svg",
    Lock => "lock.svg",
    MessageCircle => "message-circle.svg",
    Menu => "menu.svg",
    Mic => "mic.svg",
    MicOff => "mic-off.svg",
    More => "ellipsis.svg",
    Pencil => "pencil.svg",
    PhoneOff => "phone-off.svg",
    Pin => "pin.svg",
    PinOff => "pin-off.svg",
    Pause => "pause.svg",
    Play => "play.svg",
    Plus => "plus.svg",
    Reply => "reply.svg",
    Retry => "rotate-ccw.svg",
    Search => "search.svg",
    Send => "send.svg",
    Settings => "settings.svg",
    Smile => "smile.svg",
    SmilePlus => "smile-plus.svg",
    Thread => "thread.svg",
    Trash => "trash-2.svg",
    Users => "users.svg",
    Volume => "volume-2.svg",
    VolumeX => "volume-x.svg",
    ;
    // The formatting toolbar's Lucide icons, as web's (lucide-react).
    Bold => r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 12h9a4 4 0 0 1 0 8H7a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h7a4 4 0 0 1 0 8"/></svg>"##,
    Italic => r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="19" x2="10" y1="4" y2="4"/><line x1="14" x2="5" y1="20" y2="20"/><line x1="15" x2="9" y1="4" y2="20"/></svg>"##,
    Underline => r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 4v6a6 6 0 0 0 12 0V4"/><line x1="4" x2="20" y1="20" y2="20"/></svg>"##,
    Strikethrough => r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 4H9a3 3 0 0 0-2.83 4"/><path d="M14 12a4 4 0 0 1 0 8H6"/><line x1="4" x2="20" y1="12" y2="12"/></svg>"##,
    // `shared/reply-socket.svg`, in a square viewBox: the reply string's
    // ends. Its mouth sits at (12, 8) of the 16-unit square.
    ReplySocket => r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="-14 -8 16 16" fill="none"><defs><linearGradient id="metal" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#646b70"/><stop offset=".32" stop-color="#a2a9ad"/><stop offset=".55" stop-color="#70787d"/><stop offset="1" stop-color="#30363a"/></linearGradient></defs><path d="M-12 -2.4Q-13 0 -12 2.4L-1 4.5V-4.5Z" fill="url(#metal)" stroke="#34383b" stroke-width=".6"/><path d="M-11 -1.6L-3 -3" stroke="#c3c8cb" stroke-opacity=".35" stroke-width=".6" stroke-linecap="round"/><ellipse cx="-1" rx="1.6" ry="4.5" fill="#252a2d" stroke="#b64d32" stroke-width=".8"/><ellipse cx="-1" rx=".7" ry="3.4" fill="#151719"/></svg>"##,
    TextQuote => r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17 6H3"/><path d="M21 12H8"/><path d="M21 18H8"/><path d="M3 12v6"/></svg>"##,
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
