//! Inline link preview players (YouTube, Vimeo, Twitch): after Play, the
//! operating system's own web engine shows Caper's `play` page in a native
//! view kept exactly over the card's player box, as the Android and Apple
//! clients do with their web views. Nothing else in Caper uses a web engine.
//!
//! - Windows: WebView2, in process, as a child of Caper's window.
//! - Linux: WebKitGTK in the separate `caper-web-player` helper, so Caper
//!   itself never links GTK and still starts without it. On X11 its view is
//!   a child of Caper's window; Wayland cannot place another process's
//!   surface inside ours, so there it plays in its own small window.
//!
//! One player at a time. The view hides (still playing) while its box is
//! not fully inside the timeline's visible area or anything egui paints
//! above it (menus, popups, tooltips, dialogs, the viewer) meets it; the card
//! then shows its poster marked "Playing", so the layout never moves. It
//! goes away when another starts, the conversation changes, the card has not
//! been laid out for [`FORGET_AFTER_FRAMES`] frames, or the app closes.
//! The web view itself has Caper's player safety rules
//! ([`caper_web_player::engine`]).

use eframe::egui;

pub use caper_web_player::protocol::Bounds;

/// Platforms with an engine; elsewhere Play opens the page in the browser.
pub const SUPPORTED: bool = cfg!(any(windows, target_os = "linux"));

/// Frames a playing card may go without being laid out (scrolled far away,
/// or its message gone) before its player is closed.
pub const FORGET_AFTER_FRAMES: u32 = 90;

/// Which player block: its card's scope (the message id), card and block.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub scope: String,
    pub card: usize,
    pub block: usize,
}

/// Why a card cannot play here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// No web engine (WebKitGTK, the helper, or the WebView2 runtime).
    Missing,
    /// The engine started but failed.
    Failed,
}

impl Unavailable {
    pub fn message(self) -> &'static str {
        match self {
            Self::Missing if cfg!(windows) => {
                "Install the Microsoft Edge WebView2 Runtime to play videos in Caper"
            }
            Self::Missing => "Install WebKitGTK (libwebkit2gtk-4.1-0) to play videos in Caper",
            Self::Failed => "This video can’t be played here.",
        }
    }
}

/// What a card shows for its player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Waiting for the engine.
    Starting,
    /// The view is over the card (when visible).
    Inline,
    /// Playing in its own window (Wayland).
    Window,
    Unavailable(Unavailable),
}

/// What the engine reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    Ready {
        window: bool,
    },
    /// A link followed out of the player, for the system browser.
    Open(String),
    Failed(Unavailable),
    /// The engine went away by itself. Only the Linux helper process reports
    /// it; WebView2 lives in our own process.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Exited,
    /// The person closed the player's own window (Linux on Wayland).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Closed,
}

struct Active {
    key: Key,
    page: url::Url,
    provider: String,
    conversation: Option<String>,
    status: Status,
    unseen: u32,
}

/// Which player is active and what it shows, apart from any engine: each
/// method returns whether the engine must be destroyed.
#[derive(Default)]
pub struct Lifecycle {
    active: Option<Active>,
}

impl Lifecycle {
    pub fn status(&self, key: &Key) -> Option<Status> {
        self.active
            .as_ref()
            .filter(|active| &active.key == key)
            .map(|active| active.status)
    }

    /// Play `key`: replaces any other player; a card that could not play
    /// tries again.
    pub fn start(
        &mut self,
        key: Key,
        page: url::Url,
        provider: &str,
        conversation: Option<String>,
    ) -> bool {
        if let Some(active) = &self.active
            && active.key == key
            && !matches!(active.status, Status::Unavailable(_))
        {
            return false;
        }
        let destroy = self.active.is_some();
        self.active = Some(Active {
            key,
            page,
            provider: provider.to_owned(),
            conversation,
            status: Status::Starting,
            unseen: 0,
        });
        destroy
    }

    pub fn stop(&mut self) -> bool {
        self.active.take().is_some()
    }

    /// End of a frame: in `conversation`, the card was laid out or not.
    pub fn frame(&mut self, conversation: Option<&str>, placed: bool) -> bool {
        let Some(active) = &mut self.active else {
            return false;
        };
        active.unseen = if placed { 0 } else { active.unseen + 1 };
        if active.conversation.as_deref() != conversation || active.unseen > FORGET_AFTER_FRAMES {
            self.active = None;
            return true;
        }
        false
    }

    pub fn notice(&mut self, notice: &Notice) -> bool {
        let Some(active) = &mut self.active else {
            return true;
        };
        match notice {
            Notice::Ready { window } => {
                if active.status == Status::Starting {
                    active.status = if *window {
                        Status::Window
                    } else {
                        Status::Inline
                    };
                }
                false
            }
            Notice::Open(_) => false,
            // The card keeps its message until it goes or plays again.
            Notice::Failed(why) => {
                active.status = Status::Unavailable(*why);
                true
            }
            Notice::Exited if active.status == Status::Starting => {
                active.status = Status::Unavailable(Unavailable::Missing);
                true
            }
            Notice::Exited | Notice::Closed => {
                self.active = None;
                true
            }
        }
    }

    /// The player waiting for an engine.
    fn starting(&self) -> Option<&Active> {
        self.active
            .as_ref()
            .filter(|active| active.status == Status::Starting)
    }
}

/// Whether the view can show over its box this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Shown,
    /// Partly outside the timeline's visible area.
    Clipped,
    /// Something egui paints above it meets it.
    Covered,
}

pub fn visibility(rect: egui::Rect, clip: egui::Rect, overlays: &[egui::Rect]) -> Visibility {
    // Half a point of slack for rounding at the edges.
    if !clip.expand(0.5).contains_rect(rect) {
        Visibility::Clipped
    } else if overlays
        .iter()
        .any(|overlay| overlay.intersect(rect).is_positive())
    {
        Visibility::Covered
    } else {
        Visibility::Shown
    }
}

/// `rect` (egui points) in physical pixels of the window's client area,
/// edges rounded so neighbouring boxes never gap.
pub fn physical(rect: egui::Rect, pixels_per_point: f32) -> Bounds {
    let x = (rect.min.x * pixels_per_point).round();
    let y = (rect.min.y * pixels_per_point).round();
    let right = (rect.max.x * pixels_per_point).round();
    let bottom = (rect.max.y * pixels_per_point).round();
    Bounds {
        x: x as i32,
        y: y as i32,
        width: (right - x).max(1.0) as u32,
        height: (bottom - y).max(1.0) as u32,
    }
}

/// What egui painted this frame above `layer` (and on the `extra` layers,
/// which paint over everything, such as a dialog's backdrop): each shape's
/// painted area, within its clip.
pub fn painted_above(
    context: &egui::Context,
    layer: egui::LayerId,
    extra: &[egui::LayerId],
) -> Vec<egui::Rect> {
    let order: Vec<egui::LayerId> = context.memory(|memory| memory.layer_ids().collect());
    let index = |id: &egui::LayerId| order.iter().position(|other| other == id);
    let own = index(&layer);
    let above = |other: &egui::LayerId| {
        *other != layer
            && match other.order.cmp(&layer.order) {
                std::cmp::Ordering::Equal => match (own, index(other)) {
                    (Some(own), Some(other)) => other > own,
                    // egui paints layers missing from its order last.
                    (_, None) => true,
                    (None, Some(_)) => false,
                },
                order => order.is_gt(),
            }
    };
    let layers: Vec<egui::LayerId> = order
        .iter()
        .filter(|other| above(other))
        .chain(extra.iter().filter(|other| **other != layer))
        .copied()
        .collect();
    context.graphics(|graphics| {
        layers
            .iter()
            .filter_map(|layer| graphics.get(*layer))
            .flat_map(|list| list.all_entries())
            .filter(|shape| !matches!(shape.shape, egui::Shape::Noop))
            .map(|shape| {
                shape
                    .shape
                    .visual_bounding_rect()
                    .intersect(shape.clip_rect)
            })
            .filter(|rect| rect.is_positive())
            .collect()
    })
}

/// Where the active card drew its box this frame.
#[derive(Clone, Copy)]
struct Placement {
    rect: egui::Rect,
    clip: egui::Rect,
    layer: egui::LayerId,
}

/// The app's one player.
#[derive(Default)]
pub struct WebPlayer {
    lifecycle: Lifecycle,
    engine: Option<engine::Engine>,
    conversation: Option<String>,
    placement: Option<Placement>,
    /// What the engine was last told: where it is and whether it shows.
    sent_bounds: Option<Bounds>,
    sent_shown: Option<bool>,
}

impl WebPlayer {
    pub fn status(&self, key: &Key) -> Option<Status> {
        self.lifecycle.status(key)
    }

    /// Call before drawing, with the open conversation.
    pub fn begin_frame(&mut self, conversation: Option<String>) {
        self.conversation = conversation;
        self.placement = None;
    }

    pub fn start(&mut self, context: &egui::Context, key: Key, page: url::Url, provider: &str) {
        if self
            .lifecycle
            .start(key, page, provider, self.conversation.clone())
        {
            self.destroy();
        }
        context.request_repaint();
    }

    pub fn stop(&mut self) {
        self.lifecycle.stop();
        self.destroy();
    }

    /// Bring the player's own window forward.
    pub fn raise(&mut self) {
        if let Some(engine) = &mut self.engine {
            engine.raise();
        }
    }

    /// The card for `key` drew its box at `rect`, inside `clip`, on `layer`.
    /// Returns false when another copy of the card already did this frame.
    pub fn place(
        &mut self,
        key: &Key,
        rect: egui::Rect,
        clip: egui::Rect,
        layer: egui::LayerId,
    ) -> bool {
        if self.placement.is_some() || self.lifecycle.status(key).is_none() {
            return false;
        }
        self.placement = Some(Placement { rect, clip, layer });
        true
    }

    /// Call after everything is drawn: starts, moves, shows or hides and
    /// closes the engine. `extra` names layers painted over everything
    /// without an egui area (a dialog's backdrop).
    pub fn end_frame(
        &mut self,
        context: &egui::Context,
        frame: &eframe::Frame,
        extra: &[egui::LayerId],
    ) {
        let notices = self
            .engine
            .as_mut()
            .map(engine::Engine::poll)
            .unwrap_or_default();
        for notice in notices {
            if let Notice::Open(url) = &notice
                && caper_web_player::openable(url)
            {
                context.open_url(egui::OpenUrl::new_tab(url));
            }
            if self.lifecycle.notice(&notice) {
                self.destroy();
            }
        }
        if self
            .lifecycle
            .frame(self.conversation.as_deref(), self.placement.is_some())
        {
            self.destroy();
        }
        let pixels_per_point = context.pixels_per_point();
        if self.engine.is_none()
            && let (Some(active), Some(placement)) = (self.lifecycle.starting(), self.placement)
        {
            let bounds = physical(placement.rect, pixels_per_point);
            match engine::Engine::create(
                context,
                frame,
                &active.page,
                &active.provider,
                bounds,
                placement.rect.size(),
            ) {
                Ok(engine) => self.engine = Some(engine),
                Err(why) => {
                    self.lifecycle.notice(&Notice::Failed(why));
                }
            }
            self.sent_bounds = None;
            self.sent_shown = None;
        }
        let Some(engine) = &mut self.engine else {
            return;
        };
        if !engine.inline() {
            return;
        }
        // Not laid out this frame: hidden where it was.
        let shown = self.placement.is_some_and(|placement| {
            let overlays = painted_above(context, placement.layer, extra);
            visibility(placement.rect, placement.clip, &overlays) == Visibility::Shown
        });
        if let Some(placement) = self.placement.filter(|_| shown) {
            let bounds = physical(placement.rect, pixels_per_point);
            if self.sent_bounds != Some(bounds) {
                engine.send_bounds(bounds);
                self.sent_bounds = Some(bounds);
            }
        }
        if self.sent_shown != Some(shown) {
            engine.set_visible(shown);
            self.sent_shown = Some(shown);
        }
        // A click egui saw was outside the view: keys come back to Caper.
        if context.input(|input| input.pointer.any_pressed()) {
            engine.blur();
        }
    }

    fn destroy(&mut self) {
        self.engine = None;
        self.sent_bounds = None;
        self.sent_shown = None;
    }
}

/// No engine here: Play opens the page in the browser ([`SUPPORTED`]).
#[cfg(not(any(windows, target_os = "linux")))]
mod engine {
    use eframe::egui;

    use super::{Bounds, Notice, Unavailable};

    pub struct Engine;

    impl Engine {
        pub fn create(
            _context: &egui::Context,
            _frame: &eframe::Frame,
            _page: &url::Url,
            _provider: &str,
            _bounds: Bounds,
            _size: egui::Vec2,
        ) -> Result<Self, Unavailable> {
            Err(Unavailable::Missing)
        }

        pub fn poll(&mut self) -> Vec<Notice> {
            Vec::new()
        }

        pub fn inline(&self) -> bool {
            true
        }

        pub fn send_bounds(&mut self, _bounds: Bounds) {}

        pub fn set_visible(&mut self, _visible: bool) {}

        pub fn blur(&mut self) {}

        pub fn raise(&mut self) {}
    }
}

#[cfg(windows)]
mod engine {
    //! WebView2 in process, as a child window of Caper's.

    use std::cell::RefCell;
    use std::rc::Rc;

    use caper_web_player::engine::{self as shared, wry};
    use eframe::egui;

    use super::{Bounds, Notice, Unavailable};

    pub struct Engine {
        view: wry::WebView,
        notices: Rc<RefCell<Vec<Notice>>>,
    }

    fn rect(bounds: Bounds) -> wry::Rect {
        wry::Rect {
            position: wry::dpi::PhysicalPosition::new(bounds.x, bounds.y).into(),
            size: wry::dpi::PhysicalSize::new(bounds.width, bounds.height).into(),
        }
    }

    impl Engine {
        pub fn create(
            context: &egui::Context,
            frame: &eframe::Frame,
            page: &url::Url,
            _provider: &str,
            bounds: Bounds,
            _size: egui::Vec2,
        ) -> Result<Self, Unavailable> {
            // The runtime is a separate system component; without it, say so.
            if wry::webview_version().is_err() {
                return Err(Unavailable::Missing);
            }
            let notices = Rc::new(RefCell::new(vec![Notice::Ready { window: false }]));
            let open: shared::Open = {
                let (notices, context) = (notices.clone(), context.clone());
                Rc::new(move |url| {
                    notices.borrow_mut().push(Notice::Open(url));
                    context.request_repaint();
                })
            };
            let view = shared::builder(page, open)
                .with_bounds(rect(bounds))
                .with_visible(false)
                .build_as_child(frame)
                .map_err(|_| Unavailable::Failed)?;
            Ok(Self { view, notices })
        }

        pub fn poll(&mut self) -> Vec<Notice> {
            std::mem::take(&mut self.notices.borrow_mut())
        }

        pub fn send_bounds(&mut self, bounds: Bounds) {
            let _ = self.view.set_bounds(rect(bounds));
        }

        pub fn set_visible(&mut self, visible: bool) {
            let _ = self.view.set_visible(visible);
        }

        pub fn inline(&self) -> bool {
            true
        }

        pub fn blur(&mut self) {
            let _ = self.view.focus_parent();
        }

        pub fn raise(&mut self) {}
    }
}

#[cfg(target_os = "linux")]
mod engine {
    //! The `caper-web-player` helper, over its stdin/stdout.

    use std::io::{BufRead, BufReader, Write};
    use std::path::PathBuf;
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::sync::mpsc::{self, Receiver};
    use std::sync::{Arc, Mutex};

    use caper_web_player::protocol::{self, Event};
    use eframe::egui;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::{Bounds, Notice, Unavailable};

    /// `CAPER_WEB_PLAYER`, else beside the app, else the .deb's library
    /// folder (as the updater and FFmpeg).
    fn helper() -> Option<PathBuf> {
        let name = "caper-web-player";
        let beside = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.canonicalize().ok())
            .and_then(|exe| exe.parent().map(|dir| dir.join(name)));
        std::env::var_os("CAPER_WEB_PLAYER")
            .map(PathBuf::from)
            .into_iter()
            .chain(beside)
            .chain([PathBuf::from("/usr/lib/caper-desktop").join(name)])
            .find(|path| path.is_file())
    }

    /// Why the helper stopped before it was ready, from its exit status and
    /// the end of its error output.
    pub fn classify(code: Option<i32>, stderr: &str) -> Unavailable {
        if code == Some(127) || stderr.contains("error while loading shared libraries") {
            Unavailable::Missing
        } else {
            Unavailable::Failed
        }
    }

    enum Line {
        Event(Event),
        End,
    }

    pub struct Engine {
        /// Always present until dropped.
        child: Option<Child>,
        stdin: Option<ChildStdin>,
        lines: Receiver<Line>,
        stderr: Arc<Mutex<String>>,
        /// Its own window (Wayland), not a child of Caper's.
        window: bool,
        ready: bool,
        ended: bool,
    }

    impl Engine {
        pub fn create(
            context: &egui::Context,
            frame: &eframe::Frame,
            page: &url::Url,
            provider: &str,
            bounds: Bounds,
            size: egui::Vec2,
        ) -> Result<Self, Unavailable> {
            let helper = helper().ok_or(Unavailable::Missing)?;
            let mut command = Command::new(helper);
            command.arg("--page").arg(page.as_str());
            let parent: Option<std::ffi::c_ulong> =
                match frame.window_handle().map(|handle| handle.as_raw()) {
                    Ok(RawWindowHandle::Xlib(handle)) => Some(handle.window),
                    Ok(RawWindowHandle::Xcb(handle)) => Some(handle.window.get().into()),
                    _ => None,
                };
            match parent {
                Some(parent) => {
                    // Caper's window is an X11 one even under a Wayland
                    // session (XWayland), so GTK must use X11 too.
                    command
                        .env("GDK_BACKEND", "x11")
                        .arg("--parent")
                        .arg(parent.to_string())
                        .arg("--bounds")
                        .arg(bounds.to_string());
                }
                None => {
                    command
                        .arg("--window")
                        .arg(format!(
                            "{},{}",
                            size.x.round().max(1.0),
                            size.y.round().max(1.0)
                        ))
                        .arg("--title")
                        .arg(format!(
                            "Caper — {}",
                            caper_web_player::provider_name(provider)
                        ));
                }
            }
            let mut child = command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|_| Unavailable::Missing)?;
            let (sender, lines) = mpsc::channel();
            if let Some(stdout) = child.stdout.take() {
                let context = context.clone();
                std::thread::spawn(move || {
                    for line in BufReader::new(stdout).lines() {
                        let Ok(line) = line else { break };
                        if let Some(event) = Event::parse(&line) {
                            let _ = sender.send(Line::Event(event));
                            context.request_repaint();
                        }
                    }
                    let _ = sender.send(Line::End);
                    context.request_repaint();
                });
            }
            let stderr = Arc::new(Mutex::new(String::new()));
            if let Some(output) = child.stderr.take() {
                let stderr = stderr.clone();
                std::thread::spawn(move || {
                    // Only the last few lines matter (a missing library).
                    for line in BufReader::new(output).lines() {
                        let Ok(line) = line else { break };
                        if let Ok(mut tail) = stderr.lock() {
                            if tail.len() > 4096 {
                                tail.clear();
                            }
                            tail.push_str(&line);
                            tail.push('\n');
                        }
                    }
                });
            }
            Ok(Self {
                stdin: child.stdin.take(),
                child: Some(child),
                lines,
                stderr,
                window: parent.is_none(),
                ready: false,
                ended: false,
            })
        }

        pub fn poll(&mut self) -> Vec<Notice> {
            let mut notices = Vec::new();
            while let Ok(line) = self.lines.try_recv() {
                match line {
                    Line::Event(Event::Ready) => {
                        self.ready = true;
                        notices.push(Notice::Ready {
                            window: self.window,
                        });
                    }
                    Line::Event(Event::Open(url)) => notices.push(Notice::Open(url)),
                    Line::Event(Event::Error(_)) if !self.ready => {
                        notices.push(Notice::Failed(Unavailable::Failed));
                    }
                    Line::Event(Event::Error(_)) => {}
                    Line::Event(Event::Closed) => notices.push(Notice::Closed),
                    Line::End => self.ended = true,
                }
            }
            if self.ended {
                // stdout closed: it is exiting, if not already gone.
                let code = self
                    .child
                    .as_mut()
                    .and_then(|child| child.wait().ok())
                    .and_then(|status| status.code());
                let stderr = self
                    .stderr
                    .lock()
                    .map(|tail| tail.clone())
                    .unwrap_or_default();
                notices.push(if self.ready {
                    Notice::Exited
                } else {
                    Notice::Failed(classify(code, &stderr))
                });
            }
            notices
        }

        fn send(&mut self, command: protocol::Command) {
            if let Some(stdin) = &mut self.stdin
                && writeln!(stdin, "{command}")
                    .and_then(|()| stdin.flush())
                    .is_err()
            {
                self.stdin = None;
            }
        }

        pub fn send_bounds(&mut self, bounds: Bounds) {
            self.send(protocol::Command::Bounds(bounds));
        }

        pub fn set_visible(&mut self, visible: bool) {
            self.send(if visible {
                protocol::Command::Show
            } else {
                protocol::Command::Hide
            });
        }

        pub fn inline(&self) -> bool {
            !self.window
        }

        /// X11 keyboard focus follows the window manager; nothing to do.
        pub fn blur(&mut self) {}

        pub fn raise(&mut self) {
            self.send(protocol::Command::Raise);
        }
    }

    impl Drop for Engine {
        fn drop(&mut self) {
            self.send(protocol::Command::Close);
            self.stdin = None;
            // Reap it without blocking the app; kill it if it lingers.
            let Some(mut child) = self.child.take() else {
                return;
            };
            std::thread::spawn(move || {
                for _ in 0..20 {
                    if matches!(child.try_wait(), Ok(Some(_))) {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                let _ = child.kill();
                let _ = child.wait();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(scope: &str) -> Key {
        Key {
            scope: scope.into(),
            card: 0,
            block: 2,
        }
    }

    fn page() -> url::Url {
        url::Url::parse("https://embed.caper.chat/play/youtube/dQw4w9WgXcQ").unwrap()
    }

    fn rect(x: f32, y: f32, width: f32, height: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, height))
    }

    #[test]
    fn the_view_shows_only_when_its_box_is_fully_visible_and_uncovered() {
        let clip = rect(340.0, 55.0, 720.0, 350.0);
        let player = rect(415.0, 113.0, 400.0, 225.0);
        assert_eq!(visibility(player, clip, &[]), Visibility::Shown);
        // Rounding at the clip's edge still counts as inside.
        let flush = rect(415.0, 55.2, 400.0, 225.0);
        assert_eq!(visibility(flush, clip, &[]), Visibility::Shown);
        for clipped in [
            rect(415.0, 40.0, 400.0, 225.0),
            rect(415.0, 300.0, 400.0, 225.0),
            rect(900.0, 113.0, 400.0, 225.0),
            rect(415.0, -400.0, 400.0, 225.0),
        ] {
            assert_eq!(visibility(clipped, clip, &[]), Visibility::Clipped);
        }
        let menu = rect(450.0, 490.0 - 400.0, 72.0, 90.0);
        assert_eq!(visibility(player, clip, &[menu]), Visibility::Covered);
        let backdrop = rect(0.0, 0.0, 1280.0, 800.0);
        assert_eq!(visibility(player, clip, &[backdrop]), Visibility::Covered);
        // Touching edges or elsewhere on screen is not covering.
        let beside = rect(815.0, 113.0, 100.0, 100.0);
        let below = rect(415.0, 338.0, 400.0, 20.0);
        assert_eq!(
            visibility(player, clip, &[beside, below]),
            Visibility::Shown
        );
    }

    #[test]
    fn physical_bounds_follow_the_scale_and_never_gap() {
        assert_eq!(
            physical(rect(371.0, 500.0, 382.0, 215.0), 1.0),
            Bounds {
                x: 371,
                y: 500,
                width: 382,
                height: 215
            }
        );
        assert_eq!(
            physical(rect(10.25, 20.5, 400.0, 225.0), 1.5),
            Bounds {
                x: 15,
                y: 31,
                width: 600,
                height: 337
            }
        );
        let tiny = physical(rect(1.0, 1.0, 0.1, 0.1), 2.0);
        assert_eq!((tiny.width, tiny.height), (1, 1));
    }

    #[test]
    fn overlays_are_what_egui_paints_above_the_timeline() {
        let context = egui::Context::default();
        let player = rect(100.0, 100.0, 200.0, 120.0);
        let backdrop = egui::LayerId::new(egui::Order::Background, egui::Id::new("backdrop"));
        let frame = |menu: Option<egui::Pos2>, scrim: bool| {
            let mut covering = Vec::new();
            let _ = context.run(egui::RawInput::default(), |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    // The card itself and anything else in its own layer.
                    ui.painter().rect_filled(player, 6.0, egui::Color32::RED);
                    ui.label("timeline");
                });
                if let Some(position) = menu {
                    egui::Area::new(egui::Id::new("menu"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(position)
                        .show(context, |ui| {
                            ui.set_min_size(egui::vec2(80.0, 60.0));
                            ui.painter()
                                .rect_filled(ui.max_rect(), 8.0, egui::Color32::BLACK);
                        });
                }
                if scrim {
                    context.layer_painter(backdrop).rect_filled(
                        context.viewport_rect(),
                        0.0,
                        egui::Color32::from_black_alpha(190),
                    );
                }
                covering = painted_above(context, egui::LayerId::background(), &[backdrop]);
            });
            visibility(player, rect(0.0, 0.0, 1000.0, 800.0), &covering)
        };
        assert_eq!(frame(None, false), Visibility::Shown);
        // A new area paints nothing in its first (sizing) pass.
        frame(Some(egui::pos2(150.0, 150.0)), false);
        assert_eq!(
            frame(Some(egui::pos2(150.0, 150.0)), false),
            Visibility::Covered
        );
        // A menu elsewhere leaves it alone; closing one shows it at once.
        for _ in 0..2 {
            assert_eq!(
                frame(Some(egui::pos2(600.0, 500.0)), false),
                Visibility::Shown
            );
        }
        assert_eq!(frame(None, false), Visibility::Shown);
        assert_eq!(frame(None, true), Visibility::Covered);
        assert_eq!(frame(None, false), Visibility::Shown);
    }

    #[test]
    fn one_player_at_a_time_and_each_ends_with_its_conversation_or_view() {
        let mut life = Lifecycle::default();
        let general = Some("general".to_owned());
        assert!(!life.start(key("a"), page(), "youtube", general.clone()));
        assert_eq!(life.status(&key("a")), Some(Status::Starting));
        assert!(life.starting().is_some());
        assert!(!life.notice(&Notice::Ready { window: false }));
        assert_eq!(life.status(&key("a")), Some(Status::Inline));
        assert!(life.starting().is_none());
        // Pressing Play again on the playing card changes nothing.
        assert!(!life.start(key("a"), page(), "youtube", general.clone()));
        assert_eq!(life.status(&key("a")), Some(Status::Inline));
        // Another card replaces it.
        assert!(life.start(key("b"), page(), "vimeo", general.clone()));
        assert_eq!(life.status(&key("a")), None);
        assert_eq!(life.status(&key("b")), Some(Status::Starting));
        // Out of view briefly keeps it; far out of view ends it.
        for _ in 0..FORGET_AFTER_FRAMES {
            assert!(!life.frame(general.as_deref(), false));
        }
        assert!(!life.frame(general.as_deref(), true));
        for _ in 0..FORGET_AFTER_FRAMES {
            assert!(!life.frame(general.as_deref(), false));
        }
        assert!(life.frame(general.as_deref(), false));
        assert_eq!(life.status(&key("b")), None);
        // Switching conversation ends it, even while laid out.
        life.start(key("c"), page(), "twitch", general.clone());
        assert!(life.frame(Some("design"), true));
        assert_eq!(life.status(&key("c")), None);
        assert!(!life.frame(Some("design"), true), "nothing left to end");
        // Closing the app.
        life.start(key("d"), page(), "youtube", None);
        assert!(life.stop());
        assert!(!life.stop());
    }

    #[test]
    fn engine_failures_leave_a_message_until_played_again() {
        let mut life = Lifecycle::default();
        life.start(key("a"), page(), "youtube", None);
        // The helper exits before it is ready: WebKitGTK is missing.
        assert!(life.notice(&Notice::Exited));
        assert_eq!(
            life.status(&key("a")),
            Some(Status::Unavailable(Unavailable::Missing))
        );
        assert!(life.starting().is_none(), "no retry loop");
        assert!(!life.frame(None, true), "the message stays while shown");
        // Play again (after installing) retries.
        assert!(life.start(key("a"), page(), "youtube", None));
        assert_eq!(life.status(&key("a")), Some(Status::Starting));
        assert!(life.notice(&Notice::Failed(Unavailable::Failed)));
        assert_eq!(
            life.status(&key("a")),
            Some(Status::Unavailable(Unavailable::Failed))
        );
        // A player that stops after it started simply goes.
        life.start(key("b"), page(), "youtube", None);
        life.notice(&Notice::Ready { window: false });
        assert!(life.notice(&Notice::Exited));
        assert_eq!(life.status(&key("b")), None);
        // Wayland: its own window, which the person may close.
        life.start(key("c"), page(), "youtube", None);
        assert!(!life.notice(&Notice::Ready { window: true }));
        assert_eq!(life.status(&key("c")), Some(Status::Window));
        assert!(!life.notice(&Notice::Open("https://www.youtube.com/".into())));
        assert!(life.notice(&Notice::Closed));
        assert_eq!(life.status(&key("c")), None);
    }

    #[test]
    fn missing_webkitgtk_is_told_apart_from_other_failures() {
        #[cfg(target_os = "linux")]
        {
            assert_eq!(
                engine::classify(
                    Some(127),
                    "caper-web-player: error while loading shared libraries: libwebkit2gtk-4.1.so.0: cannot open shared object file"
                ),
                Unavailable::Missing
            );
            assert_eq!(
                engine::classify(None, "error while loading shared libraries: libgtk-3.so.0"),
                Unavailable::Missing
            );
            assert_eq!(
                engine::classify(Some(1), "Gtk-WARNING: cannot open display"),
                Unavailable::Failed
            );
        }
        assert!(
            Unavailable::Missing
                .message()
                .contains("to play videos in Caper")
        );
    }

    #[test]
    fn placement_belongs_to_the_active_card_once_a_frame() {
        let mut player = WebPlayer::default();
        let context = egui::Context::default();
        player.begin_frame(None);
        let area = rect(0.0, 0.0, 100.0, 60.0);
        let layer = egui::LayerId::background();
        assert!(!player.place(&key("a"), area, area, layer), "not playing");
        player.start(&context, key("a"), page(), "youtube");
        assert!(!player.place(&key("b"), area, area, layer));
        assert!(player.place(&key("a"), area, area, layer));
        assert!(
            !player.place(&key("a"), area, area, layer),
            "a second copy of the card shows the poster"
        );
        player.begin_frame(None);
        assert!(player.place(&key("a"), area, area, layer));
        player.stop();
        assert_eq!(player.status(&key("a")), None);
    }
}
