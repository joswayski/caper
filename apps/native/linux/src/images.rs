//! Textures for the client: the web's Lucide icons, Caper's avatar mascots
//! (`caper_client::avatar_images`) and uploaded profile pictures.
//!
//! Every texture is made once and shared by every widget that shows it.
//! Uploaded pictures are cached by asset id, never by URL (signed URLs
//! re-sign daily), fetched and decoded off the main loop, and reported back
//! through the host channel; until one is ready, or if it fails, callers show
//! the person's mascot or initial.

use crate::host::Signal;
use caper_client::Rgba;
use caper_client::api::Api;
use gtk::{gdk, glib, prelude::*};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// Avatar textures are drawn at this size and scaled down by GTK.
const AVATAR_EDGE: u32 = 72;
/// Icons are drawn at twice their display size, for HiDPI.
const ICON_EDGE: u32 = 40;
const MAX_FETCHES: usize = 4;
/// Twemoji sheets are 1024px squares of 64px cells.
const SHEET_EDGE: usize = 1024;
const EMOJI_EDGE: usize = 64;

/// The Lucide icons the client draws (`desktop/resources/icons`, white
/// strokes recoloured per use).
macro_rules! icons {
    ($($name:ident => $file:literal,)*) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        #[allow(dead_code)]
        pub enum Icon {
            $($name,)*
        }

        fn icon_source(icon: Icon) -> &'static str {
            match icon {
                $(Icon::$name => include_str!(concat!("../../desktop/resources/icons/", $file, ".svg")),)*
            }
        }
    };
}

icons! {
    Send => "send",
    Users => "users",
    SmilePlus => "smile-plus",
    Smile => "smile",
    Reply => "reply",
    Thread => "thread",
    More => "ellipsis",
    Hash => "hash",
    Lock => "lock",
    Close => "x",
    LogOut => "log-out",
    FileText => "file-text",
    FileUp => "file-up",
    Play => "play",
    Pause => "pause",
    Plus => "plus",
    MessageCircle => "message-circle",
    Paperclip => "paperclip",
    Download => "download",
    ExternalLink => "external-link",
    Eye => "eye",
    EyeOff => "eye-off",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    ChevronDown => "chevron-down",
    Copy => "copy",
    Pin => "pin",
    PinOff => "pin-off",
    Search => "search",
    Settings => "settings",
    Bell => "bell",
    BellOff => "bell-off",
    Forward => "forward",
    Mic => "mic",
    MicOff => "mic-off",
    Headphones => "headphones",
    HeadphoneOff => "headphone-off",
    PhoneOff => "phone-off",
    Volume => "volume-2",
    VolumeOff => "volume-x",
    Pencil => "pencil",
    Trash => "trash-2",
    Link => "link",
    Inbox => "inbox",
    ArrowLeft => "arrow-left",
    Images => "images",
    Check => "check",
    RotateCcw => "rotate-ccw",
    Retry => "rotate-ccw",
    Message => "message-circle",
    Ban => "ban",
    AudioLines => "audio-lines",
    Speech => "speech",
}

/// What an avatar shows: an uploaded picture, a Caper mascot, or the
/// person's initial.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Avatar {
    pub name: String,
    pub mascot: Option<u16>,
    /// The uploaded picture's asset id and signed URL.
    pub picture: Option<(String, String)>,
}

impl Avatar {
    pub fn new(
        name: &str,
        avatar_id: Option<i32>,
        asset: &Option<String>,
        url: &Option<String>,
    ) -> Self {
        Self {
            name: name.to_owned(),
            mascot: caper_client::caper_avatar_index(avatar_id),
            picture: caper_client::pictures::Source::new(asset, url).map(|_| {
                (
                    asset.clone().unwrap_or_default(),
                    url.clone().unwrap_or_default(),
                )
            }),
        }
    }

    pub fn initial(&self) -> String {
        self.name
            .chars()
            .next()
            .unwrap_or('C')
            .to_uppercase()
            .collect()
    }
}

#[derive(Default)]
struct Pictures {
    ready: HashMap<String, gdk::Texture>,
    /// Asset ids being fetched, and the URL each last failed with (a
    /// re-signed URL tries again).
    loading: HashSet<String>,
    failed: HashMap<String, String>,
}

pub struct Images {
    /// Message media: attachments, link previews and GIFs.
    pub media: crate::media::Media,
    api: Api,
    signals: async_channel::Sender<Signal>,
    icons: RefCell<HashMap<(Icon, &'static str), gdk::Texture>>,
    mascots: RefCell<HashMap<u16, gdk::Texture>>,
    pictures: RefCell<Pictures>,
    /// Decoded Twemoji sheets (`caper_client::emoji::sheet`), then each
    /// emoji's 64px cell by catalog id.
    emoji_sheets: RefCell<HashMap<usize, Vec<u8>>>,
    emoji: RefCell<HashMap<String, gdk::Texture>>,
}

impl Images {
    pub fn new(api: Api, signals: async_channel::Sender<Signal>) -> Self {
        Self {
            media: crate::media::Media::new(api.clone(), signals.clone()),
            api,
            signals,
            icons: RefCell::default(),
            mascots: RefCell::default(),
            pictures: RefCell::default(),
            emoji_sheets: RefCell::default(),
            emoji: RefCell::default(),
        }
    }

    /// `icon` stroked in `color` (`#rrggbb`).
    pub fn icon(&self, icon: Icon, color: &'static str) -> Option<gdk::Texture> {
        if let Some(texture) = self.icons.borrow().get(&(icon, color)) {
            return Some(texture.clone());
        }
        let svg = icon_source(icon).replace("#ffffff", color);
        let texture = render_svg(svg.as_bytes(), ICON_EDGE)?;
        self.icons
            .borrow_mut()
            .insert((icon, color), texture.clone());
        Some(texture)
    }

    pub fn icon_image(&self, icon: Icon, color: &'static str, size: i32) -> gtk::Image {
        let image = gtk::Image::new();
        if let Some(texture) = self.icon(icon, color) {
            image.set_paintable(Some(&texture));
        }
        image.set_pixel_size(size);
        image
    }

    /// `entry`'s Twemoji, the same art every client draws.
    pub fn emoji(&self, entry: &caper_client::emoji::Entry) -> Option<gdk::Texture> {
        if let Some(texture) = self.emoji.borrow().get(&entry.id) {
            return Some(texture.clone());
        }
        let mut sheets = self.emoji_sheets.borrow_mut();
        let sheet = match sheets.entry(entry.sheet) {
            std::collections::hash_map::Entry::Occupied(sheet) => sheet.into_mut(),
            std::collections::hash_map::Entry::Vacant(slot) => {
                let sheet = gdk::Texture::from_bytes(&glib::Bytes::from_static(
                    caper_client::emoji::sheet(entry.sheet),
                ))
                .ok()?;
                let mut pixels = vec![0; SHEET_EDGE * SHEET_EDGE * 4];
                sheet.download(&mut pixels, SHEET_EDGE * 4);
                slot.insert(pixels)
            }
        };
        let (x, y) = (entry.x as usize, entry.y as usize);
        if x + EMOJI_EDGE > SHEET_EDGE || y + EMOJI_EDGE > SHEET_EDGE {
            return None;
        }
        let mut cell = Vec::with_capacity(EMOJI_EDGE * EMOJI_EDGE * 4);
        for row in y..y + EMOJI_EDGE {
            let start = (row * SHEET_EDGE + x) * 4;
            cell.extend_from_slice(&sheet[start..start + EMOJI_EDGE * 4]);
        }
        // `download` writes GDK's default format.
        let texture: gdk::Texture = gdk::MemoryTexture::new(
            EMOJI_EDGE as i32,
            EMOJI_EDGE as i32,
            gdk::MemoryFormat::B8g8r8a8Premultiplied,
            &glib::Bytes::from_owned(cell),
            EMOJI_EDGE * 4,
        )
        .into();
        self.emoji
            .borrow_mut()
            .insert(entry.id.clone(), texture.clone());
        Some(texture)
    }

    /// `entry` as an image `size` pixels square.
    pub fn emoji_image(&self, entry: &caper_client::emoji::Entry, size: i32) -> gtk::Image {
        let image = gtk::Image::new();
        if let Some(texture) = self.emoji(entry) {
            image.set_paintable(Some(&texture));
        }
        image.set_pixel_size(size);
        image
    }

    /// The texture `avatar` shows now: its picture once loaded (starting
    /// the load otherwise), else its mascot. `None` draws the initial.
    pub fn avatar(&self, avatar: &Avatar) -> Option<gdk::Texture> {
        if let Some((asset, url)) = &avatar.picture
            && let Some(texture) = self.picture(asset, url)
        {
            return Some(texture);
        }
        avatar.mascot.and_then(|index| self.mascot(index))
    }

    /// Whether `avatar`'s uploaded picture has loaded, so rows can tell
    /// when theirs changed.
    pub fn picture_ready(&self, avatar: &Avatar) -> bool {
        avatar
            .picture
            .as_ref()
            .is_some_and(|(asset, _)| self.pictures.borrow().ready.contains_key(asset))
    }

    fn mascot(&self, index: u16) -> Option<gdk::Texture> {
        if let Some(texture) = self.mascots.borrow().get(&index) {
            return Some(texture.clone());
        }
        let texture = render_svg(
            caper_client::avatar_images::SVG[usize::from(index)],
            AVATAR_EDGE,
        )?;
        self.mascots.borrow_mut().insert(index, texture.clone());
        Some(texture)
    }

    fn picture(&self, asset: &str, url: &str) -> Option<gdk::Texture> {
        let mut pictures = self.pictures.borrow_mut();
        if let Some(texture) = pictures.ready.get(asset) {
            return Some(texture.clone());
        }
        if pictures.loading.contains(asset)
            || pictures
                .failed
                .get(asset)
                .is_some_and(|failed| failed == url)
            || pictures.loading.len() >= MAX_FETCHES
        {
            return None;
        }
        pictures.loading.insert(asset.to_owned());
        let (api, signals) = (self.api.clone(), self.signals.clone());
        let (asset, url) = (asset.to_owned(), url.to_owned());
        std::thread::spawn(move || {
            let image = api
                .fetch_media(&url)
                .ok()
                .and_then(|bytes| caper_client::pictures::decode(&bytes, false))
                .and_then(|frames| frames.into_iter().next())
                .map(|(image, _)| image);
            let _ = signals.try_send(Signal::Picture { asset, url, image });
        });
        None
    }

    /// A fetch finished: keep the picture, or remember the URL that failed.
    pub fn loaded(&self, asset: String, url: String, image: Option<Rgba>) {
        let mut pictures = self.pictures.borrow_mut();
        pictures.loading.remove(&asset);
        match image.and_then(|image| texture(&image)) {
            Some(texture) => {
                pictures.failed.remove(&asset);
                pictures.ready.insert(asset, texture);
            }
            None => {
                pictures.failed.insert(asset, url);
            }
        }
    }

    pub fn clear_pictures(&self) {
        self.media.clear();
        let mut pictures = self.pictures.borrow_mut();
        pictures.ready.clear();
        pictures.failed.clear();
    }
}

fn render_svg(svg: &[u8], edge: u32) -> Option<gdk::Texture> {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(edge, edge)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(
            edge as f32 / tree.size().width(),
            edge as f32 / tree.size().height(),
        ),
        &mut pixmap.as_mut(),
    );
    let bytes = glib::Bytes::from_owned(pixmap.take());
    Some(
        gdk::MemoryTexture::new(
            edge as i32,
            edge as i32,
            gdk::MemoryFormat::R8g8b8a8Premultiplied,
            &bytes,
            edge as usize * 4,
        )
        .into(),
    )
}

fn texture(image: &Rgba) -> Option<gdk::Texture> {
    let [width, height] = image.size;
    if width == 0 || height == 0 || image.pixels.len() != width * height * 4 {
        return None;
    }
    let bytes = glib::Bytes::from(&image.pixels[..]);
    Some(
        gdk::MemoryTexture::new(
            width as i32,
            height as i32,
            gdk::MemoryFormat::R8g8b8a8,
            &bytes,
            width * 4,
        )
        .into(),
    )
}

/// An avatar widget: the picture or mascot, else the initial on a raised
/// circle, with an optional presence dot.
#[derive(Clone)]
pub struct AvatarView {
    pub root: gtk::Overlay,
    picture: gtk::Image,
    initial: gtk::Label,
    dot: Option<gtk::Box>,
}

impl AvatarView {
    pub fn new(size: i32, presence: bool) -> Self {
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        frame.add_css_class("avatar");
        frame.set_size_request(size, size);
        frame.set_overflow(gtk::Overflow::Hidden);
        frame.set_valign(gtk::Align::Start);
        frame.set_halign(gtk::Align::Start);
        // Avatars are square: an image at a fixed pixel size has exactly
        // that natural size, where a picture would ask for its texture's.
        let picture = gtk::Image::new();
        picture.set_pixel_size(size);
        let initial = gtk::Label::new(None);
        initial.add_css_class("initial");
        initial.set_size_request(size, size);
        frame.append(&picture);
        frame.append(&initial);
        let root = gtk::Overlay::new();
        root.set_child(Some(&frame));
        root.set_valign(gtk::Align::Start);
        root.set_halign(gtk::Align::Start);
        let dot = presence.then(|| {
            let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            dot.add_css_class("presence");
            dot.set_halign(gtk::Align::End);
            dot.set_valign(gtk::Align::End);
            dot.set_size_request(10, 10);
            root.add_overlay(&dot);
            dot
        });
        Self {
            root,
            picture,
            initial,
            dot,
        }
    }

    pub fn set(&self, images: &Images, avatar: &Avatar) {
        match images.avatar(avatar) {
            Some(texture) => {
                self.picture.set_paintable(Some(&texture));
                self.picture.set_visible(true);
                self.initial.set_visible(false);
            }
            None => {
                self.picture.clear();
                self.picture.set_visible(false);
                self.initial.set_label(&avatar.initial());
                self.initial.set_visible(true);
            }
        }
    }

    /// `online`, `idle`, or anything else for offline.
    pub fn set_presence(&self, status: &str) {
        if let Some(dot) = &self.dot {
            dot.set_css_classes(&[
                "presence",
                match status {
                    "online" => "online",
                    "idle" => "idle",
                    _ => "offline",
                },
            ]);
            dot.set_tooltip_text(Some(match status {
                "online" => "Online",
                "idle" => "Idle",
                _ => "Offline",
            }));
        }
    }
}
