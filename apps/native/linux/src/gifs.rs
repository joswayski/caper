//! KLIPY GIFs and stickers, as on the egui desktop: each composer's GIF
//! button (shown once `GET /api/klipy` answers, asked again at most once a
//! minute while unavailable) opens the picker. It has GIFs and Stickers
//! tabs, a "Search KLIPY" field that searches after a 300 ms pause
//! (trending while empty), and a three-column grid that pages as it nears
//! its end, with loading, empty and Retry states and "Powered by KLIPY".
//! Choosing a tile sends it at once as its own message, answering the
//! composer's reply; KLIPY's share goes only once the server confirms the
//! send. Ads are never requested and are skipped if they arrive. Requests
//! go straight to KLIPY through `caper_client::klipy`. A child of `app`.

use super::{App, Compose, Pending, State};
use crate::ui::label;
use caper_client::klipy::{self, Gif, Item, Kind, Page, Share};
use caper_client::worker::Command;
use gtk::{glib, prelude::*};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How long a missing GIF configuration waits before it is asked again.
const RECHECK: Duration = Duration::from_secs(60);
/// The grid's column width, three to a row as desktop.
const TILE: i32 = 108;

/// The open picker: which composer it sends into, its tab, search and
/// results.
pub(super) struct Picker {
    popover: gtk::Popover,
    /// The thread it sends into; `None` is the channel composer.
    root: Option<String>,
    /// The conversation it opened in; navigating away closes it.
    channel: Option<String>,
    /// The inbox row it answers instead of a composer.
    activity: Option<caper_client::activity::Item>,
    kind: Kind,
    tabs: [gtk::ToggleButton; 2],
    search: gtk::SearchEntry,
    grid: gtk::Grid,
    scroller: gtk::ScrolledWindow,
    status: gtk::Label,
    retry: gtk::Button,
    /// The tab and trimmed query the shown results answer.
    shown: (Kind, String),
    items: Vec<(Box<Gif>, String)>,
    page: u32,
    has_next: bool,
    loading: bool,
    error: Option<String>,
    request: u64,
    /// How many of `items` the grid already shows.
    tiles: usize,
}

impl State {
    /// Whether GIFs can be sent here now: they are on, and you can write.
    pub(super) fn can_send_gif(&self) -> bool {
        self.klipy.is_some() && self.token.is_some() && self.session.is_some() && self.joined()
    }
}

impl App {
    /// Asks whether GIFs are on, unless known or asked within the minute.
    pub(super) fn check_klipy(&self, state: &mut State) {
        let Some(token) = state.token.clone() else {
            return;
        };
        if state.klipy.is_some() || state.klipy_checked.is_some_and(|at| at.elapsed() < RECHECK) {
            return;
        }
        state.klipy_checked = Some(Instant::now());
        self.worker.send(Command::Klipy { token });
    }

    pub(super) fn connect_gifs(self: &Rc<Self>) {
        for thread in [false, true] {
            let composer = if thread {
                &self.ui.thread_composer
            } else {
                &self.ui.composer
            };
            let app = self.clone();
            composer.gif.connect_clicked(move |button| {
                app.toggle_gif_picker(thread, button.upcast_ref());
            });
        }
    }

    /// The GIF buttons show while GIFs can be sent; a picker whose
    /// conversation or thread went away closes.
    pub(super) fn render_gifs(&self, state: &State) {
        let shown = state.can_send_gif();
        let editing = matches!(state.compose, Some(Compose::Edit { .. }));
        self.ui.composer.gif.set_visible(shown && !editing);
        self.ui.thread_composer.gif.set_visible(shown);
        let stale = self.gif_picker.borrow().as_ref().is_some_and(|picker| {
            // The inbox's picker answers a row, not the open conversation.
            picker.activity.is_none()
                && (!shown
                    || picker.channel != state.channel
                    || picker.root.as_ref().is_some_and(|root| {
                        state
                            .thread
                            .as_ref()
                            .is_none_or(|thread| thread.root != *root)
                    }))
        });
        if stale {
            self.close_gif_picker();
        }
    }

    /// The inbox reply's GIF option, answering `item`.
    pub(super) fn open_activity_gifs(
        self: &Rc<Self>,
        item: caper_client::activity::Item,
        anchor: &gtk::Widget,
    ) {
        self.close_gif_picker();
        self.open_gif_picker(None, None, Some(item), anchor);
    }

    /// Closes a picker opened from the inbox.
    pub(super) fn close_activity_gifs(&self) {
        let inbox = self.gif_picker.try_borrow().is_ok_and(|open| {
            open.as_ref()
                .is_some_and(|picker| picker.activity.is_some())
        });
        if inbox {
            self.close_gif_picker();
        }
    }

    fn close_gif_picker(&self) {
        let open = self.gif_picker.borrow_mut().take();
        if let Some(open) = open {
            open.popover.popdown();
        }
    }

    fn toggle_gif_picker(self: &Rc<Self>, thread: bool, anchor: &gtk::Widget) {
        let (root, channel) = {
            let state = self.state.borrow();
            let root = thread
                .then(|| state.thread.as_ref().map(|thread| thread.root.clone()))
                .flatten();
            (root, state.channel.clone())
        };
        let same = self
            .gif_picker
            .borrow()
            .as_ref()
            .is_some_and(|picker| picker.activity.is_none() && picker.root == root);
        self.close_gif_picker();
        if same || thread && root.is_none() {
            return;
        }
        self.open_gif_picker(root, channel, None, anchor);
    }

    fn open_gif_picker(
        self: &Rc<Self>,
        root: Option<String>,
        channel: Option<String>,
        activity: Option<caper_client::activity::Item>,
        anchor: &gtk::Widget,
    ) {
        let popover: gtk::Popover = super::named("GtkPopover", "GIFs and stickers");
        popover.add_css_class("gif-picker");
        popover.set_position(gtk::PositionType::Top);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.set_size_request(352, 430);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = label("GIFs and stickers", &["dialog-title"]);
        title.set_hexpand(true);
        title.set_xalign(0.0);
        top.append(&title);
        let close: gtk::Button = super::named("GtkButton", "Close GIF picker");
        close.set_child(Some(&self.images.icon_image(
            crate::images::Icon::Close,
            "#b9bcbe",
            14,
        )));
        close.add_css_class("flat");
        top.append(&close);
        body.append(&top);
        let tab_line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tab_line.set_homogeneous(true);
        let gifs = gtk::ToggleButton::with_label("GIFs");
        let stickers = gtk::ToggleButton::with_label("Stickers");
        stickers.set_group(Some(&gifs));
        gifs.set_active(true);
        for tab in [&gifs, &stickers] {
            tab.add_css_class("gif-tab");
            tab_line.append(tab);
        }
        body.append(&tab_line);
        let search: gtk::SearchEntry = super::named("GtkSearchEntry", "Search KLIPY");
        search.set_placeholder_text(Some("Search KLIPY"));
        search.set_search_delay(300);
        body.append(&search);
        let grid = gtk::Grid::new();
        grid.set_column_spacing(4);
        grid.set_row_spacing(4);
        grid.set_valign(gtk::Align::Start);
        let status = label("", &["muted"]);
        status.set_visible(false);
        let retry = gtk::Button::with_label("Retry");
        retry.set_halign(gtk::Align::Center);
        retry.set_visible(false);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&grid);
        content.append(&status);
        content.append(&retry);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&content)
            .build();
        body.append(&scroller);
        let powered = label("Powered by KLIPY", &["muted", "small"]);
        powered.set_halign(gtk::Align::End);
        body.append(&powered);
        popover.set_child(Some(&body));
        popover.set_parent(anchor);

        let weak = Rc::downgrade(self);
        close.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.close_gif_picker();
            }
        });
        for (tab, kind) in [(&gifs, Kind::Gif), (&stickers, Kind::Sticker)] {
            let weak = Rc::downgrade(self);
            tab.connect_toggled(move |tab| {
                if !tab.is_active() {
                    return;
                }
                let Some(app) = weak.upgrade() else { return };
                let changed = app
                    .gif_picker
                    .borrow_mut()
                    .as_mut()
                    .is_some_and(|picker| std::mem::replace(&mut picker.kind, kind) != kind);
                if changed {
                    app.load_gifs(1);
                }
            });
        }
        let weak = Rc::downgrade(self);
        search.connect_search_changed(move |search| {
            let Some(app) = weak.upgrade() else { return };
            // Only a different query searches again.
            let same = app.gif_picker.borrow().as_ref().is_some_and(|picker| {
                picker.request > 0 && picker.shown == (picker.kind, search.text().trim().to_owned())
            });
            if !same {
                app.load_gifs(1);
            }
        });
        let weak = Rc::downgrade(self);
        retry.connect_clicked(move |_| {
            let Some(app) = weak.upgrade() else { return };
            let page = app.gif_picker.borrow().as_ref().map_or(1, |picker| {
                if picker.items.is_empty() {
                    1
                } else {
                    picker.page + 1
                }
            });
            app.load_gifs(page);
        });
        // Near the end of the grid: the next page.
        let weak = Rc::downgrade(self);
        scroller
            .vadjustment()
            .connect_value_changed(move |adjustment| {
                if adjustment.value() + adjustment.page_size() < adjustment.upper() - 200.0 {
                    return;
                }
                let Some(app) = weak.upgrade() else { return };
                let next = app.gif_picker.borrow().as_ref().and_then(|picker| {
                    (picker.has_next && !picker.loading && picker.error.is_none())
                        .then_some(picker.page + 1)
                });
                if let Some(page) = next {
                    glib::idle_add_local_once(move || app.load_gifs(page));
                }
            });
        let weak = Rc::downgrade(self);
        popover.connect_closed(move |popover| {
            if let Some(app) = weak.upgrade()
                && let Ok(mut open) = app.gif_picker.try_borrow_mut()
                && open
                    .as_ref()
                    .is_some_and(|picker| picker.popover == *popover)
            {
                open.take();
            }
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
        *self.gif_picker.borrow_mut() = Some(Picker {
            popover: popover.clone(),
            root,
            channel,
            activity,
            kind: Kind::Gif,
            tabs: [gifs, stickers],
            search: search.clone(),
            grid,
            scroller,
            status,
            retry,
            shown: (Kind::Gif, String::new()),
            items: Vec::new(),
            page: 0,
            has_next: false,
            loading: false,
            error: None,
            request: 0,
            tiles: 0,
        });
        popover.popup();
        search.grab_focus();
        self.load_gifs(1);
    }

    /// Requests a page of the current tab and query; page 1 replaces.
    fn load_gifs(self: &Rc<Self>, page: u32) {
        let config = self.state.borrow().klipy.clone();
        let (request, kind, query) = {
            let mut open = self.gif_picker.borrow_mut();
            let Some(picker) = open.as_mut() else {
                return;
            };
            let query = picker.search.text().trim().to_owned();
            if page <= 1 {
                picker.items.clear();
                picker.has_next = false;
                picker.tiles = 0;
                clear_grid(&picker.grid);
                picker.scroller.vadjustment().set_value(0.0);
            }
            picker.request += 1;
            picker.shown = (picker.kind, query.clone());
            picker.loading = true;
            picker.error = None;
            (picker.request, picker.kind, query)
        };
        let Some(config) = config else {
            if let Some(picker) = self.gif_picker.borrow_mut().as_mut() {
                picker.loading = false;
                picker.error = Some("GIFs are unavailable right now.".into());
            }
            self.render_gif_picker();
            return;
        };
        self.render_gif_picker();
        let (sender, receiver) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = sender.send_blocking(klipy::fetch(&config, kind, &query, page));
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            if let (Ok(result), Some(app)) = (receiver.recv().await, weak.upgrade()) {
                app.gifs_loaded(request, page, result);
            }
        });
    }

    fn gifs_loaded(self: &Rc<Self>, request: u64, page: u32, result: Result<Page, String>) {
        {
            let mut open = self.gif_picker.borrow_mut();
            let Some(picker) = open.as_mut().filter(|picker| picker.request == request) else {
                return;
            };
            picker.loading = false;
            match result {
                Ok(loaded) => {
                    picker.page = page;
                    picker.has_next = loaded.has_next;
                    // Desktop can't show ad HTML: only media tiles.
                    picker
                        .items
                        .extend(loaded.items.into_iter().filter_map(|item| match item {
                            Item::Media { gif, tile, .. } => {
                                let url = tile.gif.unwrap_or(tile.url);
                                Some((gif, url))
                            }
                            Item::Ad { .. } => None,
                        }));
                }
                Err(error) => picker.error = Some(error),
            }
        }
        self.render_gif_picker();
    }

    /// Adds the tiles that arrived and shows the picker's state.
    fn render_gif_picker(self: &Rc<Self>) {
        let open = self.gif_picker.borrow();
        let Some(picker) = open.as_ref() else {
            return;
        };
        for (index, (gif, url)) in picker.items.iter().enumerate().skip(picker.tiles) {
            let height = if gif.width > 0 {
                (TILE as f64 * f64::from(gif.height) / f64::from(gif.width)).round() as i32
            } else {
                TILE
            }
            .clamp(60, 180);
            let button: gtk::Button = super::named("GtkButton", gif.alt());
            button.add_css_class("gif-tile");
            button.set_tooltip_text(Some(gif.alt()));
            let picture = gtk::Picture::new();
            picture.set_size_request(TILE, height);
            picture.set_content_fit(gtk::ContentFit::Cover);
            picture.set_can_shrink(true);
            button.set_child(Some(&picture));
            picker
                .grid
                .attach(&button, (index % 3) as i32, (index / 3) as i32, 1, 1);
            self.images
                .media
                .bind(&picture, &klipy::media_key(url), url);
            let (weak, gif, query) = (Rc::downgrade(self), gif.clone(), picker.shown.1.clone());
            let kind = picker.shown.0;
            button.connect_clicked(move |_| {
                let Some(app) = weak.upgrade() else { return };
                let app2 = app.clone();
                let (gif, query) = (gif.clone(), query.clone());
                glib::idle_add_local_once(move || app2.choose_gif(*gif, kind, query));
            });
        }
        let tiles = picker.items.len();
        let (status, retry) = match (&picker.error, picker.loading) {
            (Some(error), _) => (error.clone(), true),
            (None, true) => ("Loading…".to_owned(), false),
            (None, false) if picker.items.is_empty() => ("No results".to_owned(), false),
            _ => (String::new(), false),
        };
        picker.status.set_label(&status);
        picker.status.set_visible(!status.is_empty());
        picker.retry.set_visible(retry);
        for (tab, kind) in picker.tabs.iter().zip([Kind::Gif, Kind::Sticker]) {
            if tab.is_active() != (picker.kind == kind) {
                tab.set_active(picker.kind == kind);
            }
        }
        drop(open);
        if let Some(picker) = self.gif_picker.borrow_mut().as_mut() {
            picker.tiles = tiles;
        }
    }

    fn choose_gif(self: &Rc<Self>, gif: Gif, kind: Kind, query: String) {
        let Some((root, activity)) = self
            .gif_picker
            .borrow()
            .as_ref()
            .map(|picker| (picker.root.clone(), picker.activity.clone()))
        else {
            return;
        };
        self.close_gif_picker();
        if let Some(item) = activity {
            let share = self.state.borrow().klipy.clone().map(|config| Share {
                config,
                kind,
                slug: gif.slug.clone(),
                query,
            });
            if let Some(share) = share {
                self.send_inbox_reply(&item, Some((gif, share)));
            }
            return;
        }
        self.send_gif(root, gif, kind, query);
    }

    /// Sends a chosen GIF as its own message, at once: into `root`'s thread
    /// (with its "Also send to channel" choice) or the channel, answering
    /// the composer's reply. The draft and its files stay.
    fn send_gif(self: &Rc<Self>, root: Option<String>, gif: Gif, kind: Kind, query: String) {
        {
            let mut state = self.state.borrow_mut();
            let (Some(config), Some(chat_token), Some(channel)) = (
                state.klipy.clone(),
                state.session.as_ref().map(|session| session.token.clone()),
                state.channel.clone(),
            ) else {
                return;
            };
            if state.pending.is_some() {
                state.send_error = Some("Wait for your last message to send first.".into());
                drop(state);
                self.render();
                return;
            }
            let thread = root.is_some();
            let reply_to_id = match (&state.compose, thread) {
                (Some(Compose::Reply { id, .. }), false) => Some(id.clone()),
                _ => None,
            };
            let broadcast = thread && self.ui.thread_composer.broadcast.is_active();
            let client_id = uuid::Uuid::new_v4().to_string();
            state.pending = Some(Pending {
                client_id: client_id.clone(),
                text: String::new(),
                created_at: chrono::Local::now().to_rfc3339(),
                thread: root.clone(),
                files: Vec::new(),
                share: Some(Share {
                    config,
                    kind,
                    slug: gif.slug.clone(),
                    query,
                }),
                gif: Some(gif.clone()),
            });
            if !thread && matches!(state.compose, Some(Compose::Reply { .. })) {
                state.compose = None;
            }
            state.send_error = None;
            self.worker.send(Command::Send {
                generation: state.generation,
                token: state.token.clone(),
                chat_token,
                channel,
                client_id,
                text: String::new(),
                thread_root_id: root,
                broadcast,
                reply_to_id,
                attachment_ids: Vec::new(),
                spoiler_attachment_ids: Vec::new(),
                gif: Some(gif),
                suppress_embeds: false,
            });
            if thread {
                self.ui.thread_composer.broadcast.set_active(false);
            } else {
                self.follow.set(true);
            }
        }
        self.render();
    }
}

fn clear_grid(grid: &gtk::Grid) {
    while let Some(child) = grid.first_child() {
        grid.remove(&child);
    }
}
