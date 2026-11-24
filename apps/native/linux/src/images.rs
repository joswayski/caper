//! Textures for the client: the web's Lucide icons, Caper's avatar mascots
//! (`caper_client::avatar_images`) and uploaded profile pictures.
//!
//! Every texture is made once and shared by every widget that shows it.
//! Uploaded pictures are cached by asset id, never by URL (signed URLs
//! re-sign daily), fetched and decoded off the main loop, and reported back
//! through the host channel; until one is ready, or if it fails, callers show
//! the person's mascot or initial. A picture is kept at the size the largest
//! avatar showing it draws (`Pictures::wanted`), not the size it decodes at.

use crate::host::Signal;
use caper_client::Rgba;
use caper_client::api::Api;
use gtk::{gdk, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Avatar textures are drawn at this size and scaled down by GTK.
const AVATAR_EDGE: u32 = 72;
/// Icons are drawn at twice their display size, for HiDPI.
const ICON_EDGE: u32 = 40;
const MAX_FETCHES: usize = 4;
/// The sizes uploaded pictures are kept at (`Pictures::wanted`), below
/// `caper_client::pictures::DISPLAY_EDGE`.
const PICTURE_EDGES: [u32; 2] = [48, 96];
/// Twemoji sheets are 2048px squares of 128px cells.
const SHEET_EDGE: usize = caper_client::emoji::SHEET_EDGE as usize;
const EMOJI_EDGE: usize = caper_client::emoji::CELL_EDGE as usize;
/// Emoji cells kept before the ones nothing shows are dropped.
const EMOJI_KEPT: usize = 128;

/// The Lucide icons the client draws (`desktop/resources/icons`, white
/// strokes recoloured per use).
macro_rules! icons {
    ($($name:ident => $file:literal,)* ; $($inline:ident => $svg:literal,)*) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        #[allow(dead_code)]
        pub enum Icon {
            $($name,)*
            $($inline,)*
        }

        fn icon_source(icon: Icon) -> &'static str {
            match icon {
                $(Icon::$name => include_str!(concat!("../../desktop/resources/icons/", $file, ".svg")),)*
                $(Icon::$inline => $svg,)*
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
    ArrowRight => "arrow-right",
    Images => "images",
    Check => "check",
    RotateCcw => "rotate-ccw",
    Retry => "rotate-ccw",
    Message => "message-circle",
    Ban => "ban",
    AudioLines => "audio-lines",
    Speech => "speech",
    Crop => "crop",
    Menu => "menu",
    Archive => "archive",
    Restore => "archive-restore",
    Brush => "brush",
    Type => "type",
    Undo => "undo-2",
    Redo => "redo-2",
    Eraser => "eraser",
    ;
    // Lucide's moon, for Do Not Disturb.
    Moon => r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/></svg>"##,
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
    /// Each loaded picture (an animation's first frame), and the edge it
    /// was fetched for (`Pictures::wanted`).
    ready: HashMap<String, (gdk::Texture, u32)>,
    /// Animated pictures' frames, beside their first frame in `ready`.
    animations: HashMap<String, Rc<AnimationFrames>>,
    /// The largest avatar that has asked for each picture, in device pixels.
    shown: HashMap<String, u32>,
    /// Asset ids being fetched, and the URL each last failed with (a
    /// re-signed URL tries again).
    loading: HashSet<String>,
    failed: HashMap<String, String>,
    /// Pictures (asset id and URL) asked for while the fetches were busy,
    /// the latest last: one starts as each fetch finishes.
    waiting: Vec<(String, String)>,
}

pub struct Images {
    /// Message media: attachments, link previews and GIFs.
    pub media: crate::media::Media,
    /// Loopback media routes for FFmpeg, shared by voice and video.
    pub streams: std::rc::Rc<crate::playback::Streams>,
    /// The one voice message playing, app-wide (`crate::playback`).
    pub voice: std::rc::Rc<crate::playback::Voice>,
    api: Api,
    signals: async_channel::Sender<Signal>,
    icons: RefCell<HashMap<(Icon, &'static str), gdk::Texture>>,
    mascots: RefCell<HashMap<u16, gdk::Texture>>,
    branding: RefCell<HashMap<u16, gdk::Texture>>,
    pictures: RefCell<Pictures>,
    /// The Twemoji sheet decoded last (`caper_client::emoji::sheet`), by its
    /// index. A decoded sheet is 16 MiB, so only one stays: runs of emoji
    /// (a picker category, a message's reactions) mostly share theirs.
    emoji_sheet: RefCell<Option<(usize, Vec<u8>)>>,
    /// Each emoji's 128px cell by catalog id, and the count past which cells
    /// nothing shows are dropped (`EMOJI_KEPT` at first).
    emoji: RefCell<HashMap<String, gdk::Texture>>,
    emoji_trim: Cell<usize>,
}

impl Images {
    pub fn new(api: Api, signals: async_channel::Sender<Signal>) -> Self {
        let streams = std::rc::Rc::new(crate::playback::Streams::default());
        Self {
            media: crate::media::Media::new(api.clone(), signals.clone()),
            streams: streams.clone(),
            voice: std::rc::Rc::new(crate::playback::Voice::new(streams)),
            api,
            signals,
            icons: RefCell::default(),
            mascots: RefCell::default(),
            branding: RefCell::default(),
            pictures: RefCell::default(),
            emoji_sheet: RefCell::default(),
            emoji: RefCell::default(),
            emoji_trim: Cell::new(EMOJI_KEPT),
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
        // Icons decorate a named control or sit beside their text; screen
        // readers skip them.
        let image = gtk::Image::builder()
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
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
        let (x, y) = (entry.x as usize, entry.y as usize);
        if x + EMOJI_EDGE > SHEET_EDGE || y + EMOJI_EDGE > SHEET_EDGE {
            return None;
        }
        let mut decoded = self.emoji_sheet.borrow_mut();
        if decoded
            .as_ref()
            .is_none_or(|(index, _)| *index != entry.sheet)
        {
            // Free the last sheet before decoding the next.
            *decoded = None;
            let sheet = gdk::Texture::from_bytes(&glib::Bytes::from_static(
                caper_client::emoji::sheet(entry.sheet),
            ))
            .ok()?;
            let mut pixels = vec![0; SHEET_EDGE * SHEET_EDGE * 4];
            sheet.download(&mut pixels, SHEET_EDGE * 4);
            *decoded = Some((entry.sheet, pixels));
        }
        let (_, sheet) = decoded.as_ref()?;
        let mut cell = Vec::with_capacity(EMOJI_EDGE * EMOJI_EDGE * 4);
        for row in y..y + EMOJI_EDGE {
            let start = (row * SHEET_EDGE + x) * 4;
            cell.extend_from_slice(&sheet[start..start + EMOJI_EDGE * 4]);
        }
        // `download` writes GDK's default format, premultiplied BGRA.
        let texture = texture_from(EMOJI_EDGE, EMOJI_EDGE, cell, true, true);
        let mut cells = self.emoji.borrow_mut();
        cells.insert(entry.id.clone(), texture.clone());
        // A browsed picker leaves thousands of cells behind (64 KiB each).
        // Past the mark, keep only those a widget still shows; when most
        // are showing, the mark moves up rather than trimming every time.
        if cells.len() > self.emoji_trim.get() {
            cells.retain(|_, cell| cell.ref_count() > 1);
            self.emoji_trim.set(EMOJI_KEPT.max(cells.len() * 2));
        }
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

    /// The texture `avatar` shows now, `edge` device pixels square: its
    /// picture once loaded (starting the load otherwise), else its mascot.
    /// `None` draws the initial.
    pub fn avatar(&self, avatar: &Avatar, edge: u32) -> Option<gdk::Texture> {
        if let Some((asset, url)) = &avatar.picture
            && let Some(texture) = self.picture(asset, url, edge)
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

    /// Transparent branding Caper character for empty conversations.
    pub fn branding(&self, index: usize) -> Option<gdk::Texture> {
        let key = u16::try_from(index).ok()?;
        if let Some(texture) = self.branding.borrow().get(&key) {
            return Some(texture.clone());
        }
        let svg = caper_client::avatar_images::BRANDING.get(index)?;
        let texture = render_svg(svg, 176)?;
        self.branding.borrow_mut().insert(key, texture.clone());
        Some(texture)
    }

    /// The picture for an avatar `edge` device pixels square. One kept for
    /// smaller avatars shows meanwhile, and is fetched again at this size.
    fn picture(&self, asset: &str, url: &str, edge: u32) -> Option<gdk::Texture> {
        let mut pictures = self.pictures.borrow_mut();
        match pictures.shown.get_mut(asset) {
            Some(shown) => *shown = edge.max(*shown),
            None => {
                pictures.shown.insert(asset.to_owned(), edge);
            }
        }
        let ready = pictures.ready.get(asset).cloned();
        if pictures.outgrown(asset) {
            self.fetch(&mut pictures, asset, url);
        }
        ready.map(|(texture, _)| texture)
    }

    /// Starts fetching `asset`, unless it is on its way or failed at this
    /// URL; while the fetches are busy it waits its turn.
    fn fetch(&self, pictures: &mut Pictures, asset: &str, url: &str) {
        if pictures.loading.contains(asset)
            || pictures
                .failed
                .get(asset)
                .is_some_and(|failed| failed == url)
        {
            return;
        }
        if pictures.loading.len() >= MAX_FETCHES {
            // What was asked for last (the rows in view) goes first.
            pictures.waiting.retain(|(waiting, _)| waiting != asset);
            pictures.waiting.push((asset.to_owned(), url.to_owned()));
            return;
        }
        pictures.loading.insert(asset.to_owned());
        let edge = pictures.wanted(asset);
        let animate = motion();
        let (api, signals) = (self.api.clone(), self.signals.clone());
        let (asset, url) = (asset.to_owned(), url.to_owned());
        std::thread::spawn(move || {
            let image = api
                .fetch_media(&url)
                .ok()
                .and_then(|bytes| caper_client::pictures::decode(&bytes, animate))
                .filter(|frames| !frames.is_empty())
                .map(|frames| fit(frames, edge));
            let _ = signals.try_send(Signal::Picture {
                asset,
                url,
                image,
                edge,
            });
        });
    }

    /// A fetch finished: keep the picture, or remember the URL that failed.
    /// `edge` is the size the fetch kept it at (`DISPLAY_EDGE` for a picture
    /// this client just prepared, which is whole).
    pub fn loaded(
        &self,
        asset: String,
        url: String,
        frames: Option<caper_client::pictures::Frames>,
        edge: u32,
    ) {
        let mut pictures = self.pictures.borrow_mut();
        pictures.loading.remove(&asset);
        let frames: Option<AnimationFrames> = frames.and_then(|frames| {
            frames
                .iter()
                .map(|(image, delay)| texture(image).map(|texture| (texture, *delay)))
                .collect()
        });
        match frames.filter(|frames| !frames.is_empty()) {
            Some(frames) => {
                pictures.failed.remove(&asset);
                pictures
                    .ready
                    .insert(asset.clone(), (frames[0].0.clone(), edge));
                if frames.len() > 1 {
                    pictures.animations.insert(asset, Rc::new(frames));
                } else {
                    pictures.animations.remove(&asset);
                }
            }
            None => {
                pictures.failed.insert(asset, url);
            }
        }
        // The fetch that finished makes room for one that waited.
        while pictures.loading.len() < MAX_FETCHES
            && let Some((asset, url)) = pictures.waiting.pop()
        {
            if pictures.outgrown(&asset) {
                self.fetch(&mut pictures, &asset, &url);
            }
        }
    }

    /// An animated picture's frames, once loaded.
    fn animation(&self, avatar: &Avatar) -> Option<Rc<AnimationFrames>> {
        let (asset, _) = avatar.picture.as_ref()?;
        self.pictures.borrow().animations.get(asset).cloned()
    }

    pub fn clear_pictures(&self) {
        self.media.clear();
        let mut pictures = self.pictures.borrow_mut();
        pictures.ready.clear();
        pictures.animations.clear();
        pictures.shown.clear();
        pictures.waiting.clear();
        pictures.failed.clear();
    }
}

/// A Lucide icon the shared set lacks, from its SVG elements (`elements`),
/// stroked in `color` (`#rrggbb`). Screen readers skip it, as `icon_image`.
pub fn lucide_image(elements: &str, color: &str, size: i32) -> gtk::Image {
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"24\" height=\"24\" viewBox=\"0 0 24 24\" \
         fill=\"none\" stroke=\"{color}\" stroke-width=\"2\" stroke-linecap=\"round\" \
         stroke-linejoin=\"round\">{elements}</svg>"
    );
    let image = gtk::Image::builder()
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build();
    if let Some(texture) = render_svg(svg.as_bytes(), ICON_EDGE) {
        image.set_paintable(Some(&texture));
    }
    image.set_pixel_size(size);
    image
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
    Some(texture_from(
        edge as usize,
        edge as usize,
        pixmap.take(),
        false,
        true,
    ))
}

/// A texture from `pixels`: BGRA (`bgra`) or RGBA, premultiplied or
/// straight alpha. The cairo renderer converts a texture to cairo's format,
/// premultiplied BGRA, on every frame that draws it. GTK 4.14 and later copy
/// that format as is (other formats go through a threaded per-pixel
/// conversion); GTK 4.8 converts only straight alpha without floats. So
/// textures are stored in whichever this GTK draws cheapest.
pub fn texture_from(
    width: usize,
    height: usize,
    mut pixels: Vec<u8>,
    bgra: bool,
    premultiplied: bool,
) -> gdk::Texture {
    thread_local! {
        static CAIRO_FORMAT: bool = gtk::check_version(4, 14, 0).is_none();
    }
    let format = if CAIRO_FORMAT.with(|cairo| *cairo) {
        for pixel in pixels.as_chunks_mut::<4>().0 {
            if !bgra {
                pixel.swap(0, 2);
            }
            let alpha = u16::from(pixel[3]);
            if !premultiplied && alpha < 255 {
                for channel in &mut pixel[..3] {
                    *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
                }
            }
        }
        gdk::MemoryFormat::B8g8r8a8Premultiplied
    } else {
        if premultiplied {
            for pixel in pixels.as_chunks_mut::<4>().0 {
                let alpha = u16::from(pixel[3]);
                if alpha > 0 && alpha < 255 {
                    for channel in &mut pixel[..3] {
                        *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
                    }
                }
            }
        }
        if bgra {
            gdk::MemoryFormat::B8g8r8a8
        } else {
            gdk::MemoryFormat::R8g8b8a8
        }
    };
    gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        format,
        &glib::Bytes::from_owned(pixels),
        width * 4,
    )
    .into()
}

/// The Caper wordmark at `width`×`height` pixels.
pub fn wordmark(width: u32, height: u32) -> Option<gdk::Texture> {
    svg_texture(
        include_bytes!("../../../web/public/caper-wordmark.svg"),
        width,
        height,
    )
}

/// The Caper face that heads What's new and the update review.
pub fn caper_face(size: u32) -> Option<gdk::Texture> {
    svg_texture(
        include_bytes!("../../../web/public/caper-face.svg"),
        size,
        size,
    )
}

fn svg_texture(svg: &[u8], width: u32, height: u32) -> Option<gdk::Texture> {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(
            width as f32 / tree.size().width(),
            height as f32 / tree.size().height(),
        ),
        &mut pixmap.as_mut(),
    );
    Some(texture_from(
        width as usize,
        height as usize,
        pixmap.take(),
        false,
        true,
    ))
}

/// An animated picture's frames and how long each shows.
type AnimationFrames = crate::animate::Frames;

/// Whether pictures may move: GTK's animation setting, which follows the
/// desktop's reduced-motion choice.
pub fn motion() -> bool {
    gtk::Settings::default().is_none_or(|settings| settings.is_gtk_enable_animations())
}

impl Pictures {
    /// The edge `asset` is kept at: the first of `PICTURE_EDGES` that covers
    /// the largest avatar showing it. A picture stays decoded, an animation
    /// in every frame, so one only rows show is not kept at the size a
    /// profile editor draws it.
    fn wanted(&self, asset: &str) -> u32 {
        let shown = self.shown.get(asset).copied().unwrap_or(0);
        PICTURE_EDGES
            .into_iter()
            .find(|edge| shown <= *edge)
            .unwrap_or(caper_client::pictures::DISPLAY_EDGE)
    }

    /// Whether `asset` is missing, or kept smaller than an avatar now wants.
    fn outgrown(&self, asset: &str) -> bool {
        self.ready
            .get(asset)
            .is_none_or(|(_, kept)| *kept < self.wanted(asset))
    }
}

/// A picture's frames at most `edge` pixels square.
fn fit(frames: caper_client::pictures::Frames, edge: u32) -> caper_client::pictures::Frames {
    frames
        .into_iter()
        .map(|(frame, delay)| {
            let [width, height] = frame.size;
            if width.max(height) <= edge as usize || frame.pixels.len() != width * height * 4 {
                return (frame, delay);
            }
            let image = image::RgbaImage::from_raw(width as u32, height as u32, frame.pixels)
                .expect("the frame holds its size in pixels");
            let fitted = image::imageops::thumbnail(&image, edge, edge);
            (Rgba::from_image(fitted), delay)
        })
        .collect()
}

fn texture(image: &Rgba) -> Option<gdk::Texture> {
    crate::media::texture(image)
}

/// An avatar widget: the picture or mascot, else the initial on a raised
/// circle, with an optional presence dot.
#[derive(Clone)]
pub struct AvatarView {
    pub root: gtk::Overlay,
    picture: gtk::Image,
    size: i32,
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
        // Beside the person's name wherever it shows, so screen readers
        // skip it.
        let picture = gtk::Image::builder()
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
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
            root.add_overlay(&dot);
            dot
        });
        Self {
            root,
            picture,
            size,
            initial,
            dot,
        }
    }

    pub fn set(&self, images: &Images, avatar: &Avatar) {
        let edge = (self.size * self.root.scale_factor()).max(0) as u32;
        let texture = images.avatar(avatar, edge);
        let animation = images.animation(avatar).filter(|_| motion());
        match texture {
            Some(texture) => {
                // Every avatar plays on one timeline: a row filled again
                // keeps its animation's place instead of blinking back to
                // the first frame.
                let texture = match &animation {
                    Some(frames) => {
                        crate::animate::play(&self.picture, frames, 0, |image, frame| {
                            if let Some(image) = image.downcast_ref::<gtk::Image>() {
                                image.set_paintable(Some(frame));
                            }
                        })
                    }
                    None => {
                        crate::animate::stop(&self.picture);
                        texture
                    }
                };
                self.picture.set_paintable(Some(&texture));
                self.picture.set_visible(true);
                self.initial.set_visible(false);
            }
            None => {
                crate::animate::stop(&self.picture);
                self.picture.clear();
                self.picture.set_visible(false);
                self.initial.set_label(&avatar.initial());
                self.initial.set_visible(true);
            }
        }
    }

    /// Drops the presence dot's tooltip, for an avatar inside a control
    /// whose own tooltip (and so its description) says more.
    pub fn quiet_presence(&self) {
        if let Some(dot) = &self.dot {
            dot.set_tooltip_text(None);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn textures_draw_the_same_pixels_whatever_they_are_stored_as() {
        // Downloads come out premultiplied BGRA, cairo's own format.
        let drawn = |texture: gdk::Texture| {
            let mut pixels = [0; 8];
            texture.download(&mut pixels, 8);
            pixels
        };
        let expected = [25, 50, 100, 128, 0, 0, 255, 255];
        // Decoded pictures: straight RGBA.
        let straight = vec![200, 100, 50, 128, 255, 0, 0, 255];
        assert_eq!(drawn(texture_from(2, 1, straight, false, false)), expected);
        // Rendered SVGs: premultiplied RGBA.
        let premultiplied = vec![100, 50, 25, 128, 255, 0, 0, 255];
        assert_eq!(
            drawn(texture_from(2, 1, premultiplied, false, true)),
            expected
        );
        // Emoji cells: premultiplied BGRA.
        let cell = vec![25, 50, 100, 128, 0, 0, 255, 255];
        assert_eq!(drawn(texture_from(2, 1, cell, true, true)), expected);
    }

    fn frames(edge: usize, count: usize) -> caper_client::pictures::Frames {
        let frame = || Rgba {
            size: [edge, edge],
            pixels: vec![255; edge * edge * 4],
        };
        (0..count)
            .map(|_| (frame(), Duration::from_millis(100)))
            .collect()
    }

    #[test]
    fn pictures_are_kept_at_the_size_the_largest_avatar_draws() {
        let mut pictures = Pictures::default();
        assert_eq!(pictures.wanted("a"), 48, "nothing has asked yet");
        pictures.shown.insert("a".into(), 34);
        assert_eq!(pictures.wanted("a"), 48);
        // A 34px row avatar on a 2x display, then a 96px editor on one.
        pictures.shown.insert("a".into(), 68);
        assert_eq!(pictures.wanted("a"), 96);
        pictures.shown.insert("a".into(), 192);
        assert_eq!(pictures.wanted("a"), caper_client::pictures::DISPLAY_EDGE);
        assert!(pictures.outgrown("a"), "not loaded yet");

        let fitted = fit(frames(128, 3), 48);
        assert_eq!(fitted.len(), 3);
        for (frame, delay) in &fitted {
            assert_eq!(frame.size, [48, 48]);
            assert_eq!(frame.pixels.len(), 48 * 48 * 4);
            assert_eq!(*delay, Duration::from_millis(100));
        }
        let small = fit(frames(40, 2), 48);
        assert!(
            small.iter().all(|(frame, _)| frame.size == [40, 40]),
            "never enlarged"
        );
    }
}
