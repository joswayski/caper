//! The window, its state, and the worker events that change them.
//!
//! Widgets are built once and stay alive; worker events update the state,
//! then only the widgets whose content changed. The message list is a
//! `GtkListView` over a `GListStore` that receives row diffs, so unchanged
//! rows keep their items and widgets.

use crate::host::{Signal, Wakes};
use crate::rows::{self, Op, Row};
use caper_client::api::Api;
use caper_client::gateway::GatewayEvent;
use caper_client::model::{Account, Apply, ChatSession, Space, SpaceDetail, Timeline};
use caper_client::worker::{Command, Event, Worker, current};
use gtk::{gdk, gio, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

const STYLE: &str = include_str!("style.css");

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Loading,
    SignIn,
    Chat,
}

struct Pending {
    client_id: String,
    text: String,
    created_at: String,
}

struct State {
    page: Page,
    /// Fences events: advanced by every conversation load.
    generation: u64,
    navigation: u64,
    request: u64,
    busy: bool,
    error: Option<String>,
    email: String,
    challenge: Option<String>,
    token: Option<String>,
    account: Option<Account>,
    spaces: Vec<Space>,
    space: Option<String>,
    detail: Option<SpaceDetail>,
    channel: Option<String>,
    channel_name: String,
    session: Option<ChatSession>,
    session_error: Option<String>,
    timeline: Timeline,
    has_more: bool,
    loading_older: bool,
    status: String,
    pending: Option<Pending>,
    send_error: Option<String>,
    /// A draft that failed to send, returned to an empty composer.
    returned_draft: Option<String>,
    rows: Vec<Row>,
    markup: rows::Markup,
    /// Advanced when the rail, channel list or member list changes.
    layout: u64,
}

struct Ui {
    window: gtk::ApplicationWindow,
    pages: gtk::Stack,
    steps: gtk::Stack,
    email: gtk::Entry,
    email_button: gtk::Button,
    code: gtk::Entry,
    code_button: gtk::Button,
    other_email: gtk::Button,
    code_hint: gtk::Label,
    signin_error: gtk::Label,
    rail: gtk::Box,
    space_name: gtk::Label,
    channels: gtk::ListBox,
    channel_title: gtk::Label,
    status: gtk::Label,
    members_toggle: gtk::ToggleButton,
    members_panel: gtk::Box,
    members_title: gtk::Label,
    members: gtk::ListBox,
    store: gio::ListStore,
    list: gtk::ListView,
    scroller: gtk::ScrolledWindow,
    composer: gtk::TextView,
    placeholder: gtk::Label,
    send: gtk::Button,
    composer_error: gtk::Label,
    error: gtk::Label,
}

pub struct App {
    ui: Ui,
    worker: Worker<Wakes>,
    state: RefCell<State>,
    /// Keep the newest message in view as rows arrive and are measured.
    follow: Cell<bool>,
    /// `pin_tail` is moving the list: its scrolling is not the reader's.
    settling: Cell<bool>,
    rendered_layout: Cell<u64>,
    channel_ids: RefCell<Vec<String>>,
    activity: Cell<Instant>,
}

pub fn start(application: &gtk::Application, base: &str) {
    let api = match Api::new(base) {
        Ok(api) => api,
        Err(error) => {
            eprintln!("{error}");
            application.quit();
            return;
        }
    };
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(true);
    }
    if let Some(display) = gdk::Display::default() {
        let provider = gtk::CssProvider::new();
        provider.load_from_data(STYLE);
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    let (sender, receiver) = async_channel::unbounded();
    let app = Rc::new(App {
        ui: Ui::build(application),
        worker: Worker::new(api, Wakes(sender)),
        state: RefCell::new(State {
            page: Page::Loading,
            generation: 0,
            navigation: 0,
            request: 0,
            busy: false,
            error: None,
            email: String::new(),
            challenge: None,
            token: None,
            account: None,
            spaces: Vec::new(),
            space: None,
            detail: None,
            channel: None,
            channel_name: String::new(),
            session: None,
            session_error: None,
            timeline: Timeline::default(),
            has_more: false,
            loading_older: false,
            status: String::new(),
            pending: None,
            send_error: None,
            returned_draft: None,
            rows: Vec::new(),
            markup: rows::Markup::default(),
            layout: 0,
        }),
        follow: Cell::new(true),
        settling: Cell::new(false),
        rendered_layout: Cell::new(u64::MAX),
        channel_ids: RefCell::new(Vec::new()),
        activity: Cell::new(Instant::now()),
    });
    app.connect();
    app.render();
    // The window shows before any network request finishes.
    app.ui.window.present();
    app.worker.send(Command::Restore { generation: 0 });
    glib::spawn_future_local(async move {
        while let Ok(signal) = receiver.recv().await {
            match signal {
                Signal::Wake => app.receive(),
                Signal::Title(title) => app.ui.window.set_title(Some(&title)),
                Signal::Front => app.ui.window.present(),
            }
        }
    });
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::builder().label(text).xalign(0.0).build();
    for class in classes {
        label.add_css_class(class);
    }
    label
}

/// A widget of GType `class` with the accessible name `name`. gtk4-rs binds
/// `gtk_accessible_update_property` only from GTK 4.10 (`v4_10`), so this uses
/// GtkBuilder's `<accessibility>` element, which GTK 4.0 already reads.
fn named<T: IsA<glib::Object>>(class: &str, name: &str) -> T {
    let xml = format!(
        "<interface><object class=\"{class}\" id=\"widget\"><accessibility><property name=\"label\">{}</property></accessibility></object></interface>",
        glib::markup_escape_text(name)
    );
    gtk::Builder::from_string(&xml)
        .object("widget")
        .expect("a GTK widget class")
}

fn clear(container: &impl IsA<gtk::Widget>) {
    while let Some(child) = container.as_ref().first_child() {
        child.unparent();
    }
}

impl Ui {
    fn build(application: &gtk::Application) -> Self {
        let window = gtk::ApplicationWindow::builder()
            .application(application)
            .title("Caper")
            .default_width(1440)
            .default_height(900)
            .build();
        window.add_css_class("caper");
        let pages = gtk::Stack::new();

        let loading = label("Connecting to Caper…", &["muted"]);
        loading.set_halign(gtk::Align::Center);
        pages.add_named(&loading, Some("loading"));

        // Sign-in: email, then the emailed code.
        let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
        card.add_css_class("card");
        card.set_halign(gtk::Align::Center);
        card.set_valign(gtk::Align::Center);
        card.set_size_request(400, -1);
        let steps = gtk::Stack::new();
        steps.set_vhomogeneous(false);
        let email_step = gtk::Box::new(gtk::Orientation::Vertical, 12);
        email_step.append(&label("Sign in to Caper", &["title"]));
        let intro = label(
            "Use your email to create an account or log back in.",
            &["muted"],
        );
        intro.set_wrap(true);
        intro.set_max_width_chars(44);
        email_step.append(&intro);
        let email_label = label("Email", &["field"]);
        email_step.append(&email_label);
        let email = gtk::Entry::builder()
            .placeholder_text("you@example.com")
            .input_purpose(gtk::InputPurpose::Email)
            .build();
        // The label names the field for assistive technologies.
        email_label.set_mnemonic_widget(Some(&email));
        email_step.append(&email);
        let email_button = gtk::Button::with_label("Continue");
        email_button.add_css_class("primary");
        email_step.append(&email_button);
        steps.add_named(&email_step, Some("email"));
        let code_step = gtk::Box::new(gtk::Orientation::Vertical, 12);
        code_step.append(&label("Check your email", &["title"]));
        let code_hint = label("", &["muted"]);
        code_hint.set_wrap(true);
        // Wrapped labels ask for their whole text's width; keep the card's.
        code_hint.set_max_width_chars(44);
        code_step.append(&code_hint);
        let code_label = label("Sign-in code", &["field"]);
        code_step.append(&code_label);
        let code = gtk::Entry::builder()
            .placeholder_text("e.g. ABC234")
            .build();
        code_label.set_mnemonic_widget(Some(&code));
        code_step.append(&code);
        let code_button = gtk::Button::with_label("Continue");
        code_button.add_css_class("primary");
        code_step.append(&code_button);
        let other_email = gtk::Button::with_label("Use a different email");
        other_email.add_css_class("flat");
        code_step.append(&other_email);
        steps.add_named(&code_step, Some("code"));
        card.append(&steps);
        let signin_error = label("", &["error"]);
        signin_error.set_wrap(true);
        card.append(&signin_error);
        let signin = gtk::Box::new(gtk::Orientation::Vertical, 0);
        signin.add_css_class("signin");
        signin.append(&card);
        card.set_vexpand(true);
        pages.add_named(&signin, Some("signin"));

        // Chat: rail, channels, conversation, members.
        let chat = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let rail: gtk::Box = named("GtkBox", "Spaces");
        rail.set_orientation(gtk::Orientation::Vertical);
        rail.set_spacing(8);
        rail.add_css_class("rail");
        chat.append(&rail);

        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar.add_css_class("sidebar");
        sidebar.set_size_request(240, -1);
        let space_name = label("", &["space-name"]);
        space_name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        sidebar.append(&space_name);
        let channels: gtk::ListBox = named("GtkListBox", "Channels");
        channels.add_css_class("channels");
        let channel_scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&channels)
            .build();
        sidebar.append(&channel_scroller);
        chat.append(&sidebar);

        let conversation = gtk::Box::new(gtk::Orientation::Vertical, 0);
        conversation.add_css_class("conversation");
        conversation.set_hexpand(true);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        header.add_css_class("header");
        let channel_title = label("", &["channel-title"]);
        header.append(&channel_title);
        let status = label("", &["muted", "status"]);
        status.set_hexpand(true);
        header.append(&status);
        let members_toggle: gtk::ToggleButton = named("GtkToggleButton", "Members");
        members_toggle.set_icon_name("system-users-symbolic");
        members_toggle.set_tooltip_text(Some("Members"));
        members_toggle.set_active(true);
        members_toggle.set_valign(gtk::Align::Center);
        members_toggle.add_css_class("flat");
        header.append(&members_toggle);
        conversation.append(&header);

        let error = label("", &["error", "banner"]);
        error.set_wrap(true);
        conversation.append(&error);

        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let list: gtk::ListView = named("GtkListView", "Messages");
        list.set_model(Some(&gtk::NoSelection::new(Some(store.clone()))));
        list.set_factory(Some(&rows::factory()));
        list.add_css_class("messages");
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();
        conversation.append(&scroller);

        let composer_error = label("", &["error", "composer-error"]);
        composer_error.set_wrap(true);
        conversation.append(&composer_error);
        let composer_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        composer_box.add_css_class("composer");
        let composer: gtk::TextView = named("GtkTextView", "Message");
        composer.set_wrap_mode(gtk::WrapMode::WordChar);
        composer.set_accepts_tab(false);
        composer.set_hexpand(true);
        let placeholder = label("", &["placeholder"]);
        placeholder.set_can_target(false);
        placeholder.set_valign(gtk::Align::Start);
        let field = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(200)
            .child(&composer)
            .hexpand(true)
            .build();
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&field));
        overlay.add_overlay(&placeholder);
        overlay.set_hexpand(true);
        composer_box.append(&overlay);
        let send: gtk::Button = named("GtkButton", "Send message");
        send.set_icon_name("mail-send-symbolic");
        send.set_tooltip_text(Some("Send"));
        send.set_valign(gtk::Align::End);
        send.add_css_class("flat");
        composer_box.append(&send);
        conversation.append(&composer_box);
        chat.append(&conversation);

        let members_panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        members_panel.add_css_class("members");
        members_panel.set_size_request(220, -1);
        let members_title = label("", &["section"]);
        members_panel.append(&members_title);
        let members: gtk::ListBox = named("GtkListBox", "Members");
        members.set_selection_mode(gtk::SelectionMode::None);
        members.add_css_class("member-list");
        members_panel.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vexpand(true)
                .child(&members)
                .build(),
        );
        chat.append(&members_panel);
        pages.add_named(&chat, Some("chat"));
        window.set_child(Some(&pages));

        Self {
            window,
            pages,
            steps,
            email,
            email_button,
            code,
            code_button,
            other_email,
            code_hint,
            signin_error,
            rail,
            space_name,
            channels,
            channel_title,
            status,
            members_toggle,
            members_panel,
            members_title,
            members,
            store,
            list,
            scroller,
            composer,
            placeholder,
            send,
            composer_error,
            error,
        }
    }
}

impl App {
    fn connect(self: &Rc<Self>) {
        let ui = &self.ui;
        let app = self.clone();
        ui.email.connect_activate(move |_| app.request_code());
        let app = self.clone();
        ui.email_button.connect_clicked(move |_| app.request_code());
        let app = self.clone();
        ui.code.connect_activate(move |_| app.verify());
        let app = self.clone();
        ui.code_button.connect_clicked(move |_| app.verify());
        let app = self.clone();
        ui.other_email.connect_clicked(move |_| {
            {
                let mut state = app.state.borrow_mut();
                state.challenge = None;
                state.error = None;
            }
            app.render();
        });

        let panel = ui.members_panel.clone();
        ui.members_toggle
            .connect_toggled(move |toggle| panel.set_visible(toggle.is_active()));

        let app = self.clone();
        ui.channels.connect_row_activated(move |_, row| {
            let id = usize::try_from(row.index())
                .ok()
                .and_then(|index| app.channel_ids.borrow().get(index).cloned());
            if let Some(id) = id {
                let mut state = app.state.borrow_mut();
                if state.channel.as_deref() != Some(&id) {
                    let space = state.space.clone();
                    app.navigate(&mut state, space, Some(id));
                }
                drop(state);
                app.render();
            }
        });

        // Enter sends; Shift+Enter is a new line.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let app = self.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            app.active();
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
                && !modifiers.contains(gdk::ModifierType::SHIFT_MASK)
            {
                app.send_message();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        ui.composer.add_controller(keys);
        let app = self.clone();
        ui.send.connect_clicked(move |_| app.send_message());
        let placeholder = ui.placeholder.clone();
        ui.composer
            .buffer()
            .connect_changed(move |buffer| placeholder.set_visible(buffer.char_count() == 0));

        let adjustment = ui.scroller.vadjustment();
        let app = self.clone();
        adjustment.connect_value_changed(move |adjustment| {
            if app.settling.get() {
                return;
            }
            let bottom = adjustment.upper() - adjustment.page_size();
            app.follow.set(adjustment.value() >= bottom - 48.0);
            app.active();
            // Near the top: load earlier history, after this signal.
            if adjustment.upper() > adjustment.page_size()
                && adjustment.value() < adjustment.page_size()
            {
                let app = app.clone();
                glib::idle_add_local_once(move || app.load_older());
            }
        });
    }

    /// Tells the gateway someone is here, at most every few seconds.
    fn active(&self) {
        if self.activity.get().elapsed() > Duration::from_secs(5) {
            self.activity.set(Instant::now());
            self.worker.send(Command::Activity);
        }
    }

    fn request_code(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            if state.busy {
                return;
            }
            let email = self.ui.email.text().trim().to_owned();
            if email.contains('@') {
                state.busy = true;
                state.error = None;
                state.email.clone_from(&email);
                self.worker.send(Command::RequestCode {
                    generation: state.generation,
                    email,
                });
            } else {
                state.error = Some("Enter your email address.".into());
            }
        }
        self.render();
    }

    fn verify(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            let mut code = self.ui.code.text().to_string();
            caper_client::sign_in::clean_sign_in_code(&mut code);
            if state.busy {
                return;
            }
            let Some(challenge) = state.challenge.clone() else {
                return;
            };
            if code.is_empty() {
                state.error = Some("Enter the code from your email.".into());
            } else {
                state.busy = true;
                state.error = None;
                self.worker.send(Command::VerifyCode {
                    generation: state.generation,
                    challenge,
                    code,
                });
            }
        }
        self.render();
    }

    fn establish(&self, state: &mut State, token: String, account: Account, spaces: Vec<Space>) {
        state.token = Some(token);
        state.account = Some(account);
        state.spaces = spaces;
        state.page = Page::Chat;
        state.layout += 1;
        if let Some(space) = state.spaces.first().map(|space| space.id.clone()) {
            self.navigate(state, Some(space), None);
        }
    }

    fn navigate(&self, state: &mut State, space: Option<String>, channel: Option<String>) {
        state.generation += 1;
        state.navigation += 1;
        state.loading_older = false;
        state.session = None;
        state.session_error = None;
        state.pending = None;
        state.status = "Loading…".into();
        state.error = None;
        if state.space != space {
            state.detail = None;
            state.space.clone_from(&space);
        }
        if let Some(name) = state.detail.as_ref().and_then(|detail| {
            detail
                .channels
                .iter()
                .find(|entry| Some(&entry.id) == channel.as_ref())
                .map(|entry| entry.name.clone())
        }) {
            state.channel_name = name;
        }
        state.channel.clone_from(&channel);
        state.layout += 1;
        let name = state
            .account
            .as_ref()
            .and_then(|account| account.display_name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "Guest".into());
        self.worker.send(Command::PrepareNavigation {
            generation: state.generation,
            navigation: state.navigation,
            token: state.token.clone(),
            space,
            channel,
            name,
            cached: None,
            visit: false,
            preferred: None,
        });
    }

    fn select_space(self: &Rc<Self>, id: String) {
        {
            let mut state = self.state.borrow_mut();
            if state.space.as_deref() == Some(&id) {
                return;
            }
            self.navigate(&mut state, Some(id), None);
        }
        self.render();
    }

    fn reload(&self, state: &mut State) {
        let (space, channel) = (state.space.clone(), state.channel.clone());
        self.navigate(state, space, channel);
    }

    fn load_older(self: &Rc<Self>) {
        let Ok(mut state) = self.state.try_borrow_mut() else {
            return;
        };
        if state.loading_older || !state.has_more {
            return;
        }
        let Some(channel) = state.channel.clone() else {
            return;
        };
        let Some(before) = state
            .timeline
            .messages()
            .find(|message| message.is_channel_message())
            .map(|message| message.seq.clone())
        else {
            return;
        };
        state.loading_older = true;
        state.request += 1;
        self.worker.send(Command::LoadOlder {
            generation: state.generation,
            request: state.request,
            token: state.token.clone(),
            channel,
            before,
        });
    }

    fn send_message(self: &Rc<Self>) {
        let buffer = self.ui.composer.buffer();
        let draft = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        let text = draft.trim();
        if text.is_empty() {
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            if state.pending.is_some() {
                state.send_error = Some("Wait for your last message to send first.".into());
                drop(state);
                self.render();
                return;
            }
            let (Some(chat_token), Some(channel)) = (
                state.session.as_ref().map(|session| session.token.clone()),
                state.channel.clone(),
            ) else {
                return;
            };
            let client_id = uuid::Uuid::new_v4().to_string();
            state.pending = Some(Pending {
                client_id: client_id.clone(),
                text: text.to_owned(),
                created_at: chrono::Local::now().to_rfc3339(),
            });
            state.send_error = None;
            self.worker.send(Command::Send {
                generation: state.generation,
                token: state.token.clone(),
                chat_token,
                channel,
                client_id,
                text: text.to_owned(),
                thread_root_id: None,
                broadcast: false,
                reply_to_id: None,
                attachment_ids: Vec::new(),
                spoiler_attachment_ids: Vec::new(),
                gif: None,
                suppress_embeds: false,
            });
        }
        self.follow.set(true);
        buffer.set_text("");
        self.render();
    }

    /// Scrolls to the newest row, again on each frame while rows near it
    /// are measured (heights start as estimates), until it stays at the end.
    fn pin_tail(self: &Rc<Self>) {
        if self.settling.replace(true) {
            return;
        }
        let app = self.clone();
        let frames = Cell::new(0);
        self.ui.list.add_tick_callback(move |list, _| {
            let frames = frames.replace(frames.get() + 1) + 1;
            let count = app.ui.store.n_items();
            let adjustment = app.ui.scroller.vadjustment();
            let settled = frames > 2
                && adjustment.value() >= adjustment.upper() - adjustment.page_size() - 1.0;
            if count == 0 || settled || frames > 60 {
                app.settling.set(false);
                return glib::ControlFlow::Break;
            }
            // Moving the adjustment alone leaves the list anchored where it was.
            let _ = list.activate_action("list.scroll-to-item", Some(&(count - 1).to_variant()));
            glib::ControlFlow::Continue
        });
    }

    /// Drains the worker's events, then updates what they changed.
    fn receive(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            while let Ok(event) = self.worker.events.try_recv() {
                self.handle(&mut state, event);
            }
        }
        self.render();
    }

    fn handle(&self, state: &mut State, event: Event) {
        match event {
            Event::Restored { generation, result } if generation == state.generation => {
                match result {
                    Ok(Some((token, account, spaces))) => {
                        self.establish(state, token, account, spaces.spaces)
                    }
                    Ok(None) => state.page = Page::SignIn,
                    Err(error) => {
                        state.page = Page::SignIn;
                        state.error = Some(error);
                    }
                }
            }
            Event::CodeRequested { generation, result } if generation == state.generation => {
                state.busy = false;
                match result {
                    Ok(challenge) => state.challenge = Some(challenge),
                    Err(error) => state.error = Some(error),
                }
            }
            Event::Verified {
                generation,
                result,
                attempts_remaining,
            } if generation == state.generation => {
                state.busy = false;
                match result {
                    Ok((token, account, spaces)) => {
                        state.challenge = None;
                        self.worker.send(Command::PersistCredential {
                            generation: state.generation,
                            token: token.clone(),
                        });
                        self.establish(state, token, account, spaces.spaces);
                    }
                    Err(error) => {
                        state.error = Some(match attempts_remaining {
                            Some(left) => format!("{error} {left} attempts left."),
                            None => error,
                        })
                    }
                }
            }
            Event::Credential {
                result: Err(error), ..
            } => state.error = Some(error),
            Event::NavigationPrepared {
                generation,
                navigation,
                result,
            } if generation == state.generation && navigation == state.navigation => match result {
                Ok(prepared) => {
                    if let Some(detail) = prepared.detail {
                        state.detail = Some(detail);
                    }
                    match prepared.conversation {
                        Some((history, session)) => {
                            state.channel = Some(history.channel.id.clone());
                            state.channel_name = history.channel.name.clone();
                            state.markup.clear();
                            if let Err(error) =
                                state.timeline.reset(history.messages, &history.cursor)
                            {
                                state.error = Some(error);
                            }
                            state.has_more = history.has_more;
                            match session {
                                Ok(session) => state.session = Some(session),
                                Err(error) => state.session_error = Some(error),
                            }
                            self.follow.set(true);
                            state.status = "Connecting…".into();
                            self.worker.send(Command::Connect {
                                generation: state.generation,
                                token: state.token.clone(),
                                channel: history.channel.id,
                                cursor: state.timeline.cursor(),
                                presence: None,
                                media: Default::default(),
                            });
                        }
                        None => {
                            state.channel = None;
                            state.channel_name.clear();
                            state.timeline = Timeline::default();
                            state.status.clear();
                        }
                    }
                    state.layout += 1;
                }
                Err(error) => {
                    state.status.clear();
                    state.error = Some(error.message);
                }
            },
            Event::OlderLoaded {
                generation,
                request,
                channel,
                result,
            } if generation == state.generation
                && request == state.request
                && state.channel.as_deref() == Some(&channel) =>
            {
                state.loading_older = false;
                match result {
                    Ok(history) => {
                        state.has_more = history.has_more;
                        if let Err(error) = state.timeline.prepend(history.messages) {
                            state.error = Some(error);
                        }
                    }
                    Err(error) => state.error = Some(error.message),
                }
            }
            Event::Sent {
                generation,
                channel,
                client_id,
                result,
            } if current(
                generation,
                state.generation,
                Some(&channel),
                state.channel.as_deref(),
            ) && state
                .pending
                .as_ref()
                .is_some_and(|pending| pending.client_id == client_id) =>
            {
                let pending = state.pending.take();
                match result {
                    Ok(message) => {
                        if state.timeline.merge_sent(message).is_err() {
                            state.send_error =
                                Some("Caper returned an invalid send confirmation.".into());
                        }
                    }
                    Err(failure) => {
                        state.send_error = Some(failure.message);
                        state.returned_draft = pending.map(|pending| pending.text);
                    }
                }
            }
            Event::Gateway(event) => self.gateway(state, event),
            _ => {}
        }
    }

    fn gateway(&self, state: &mut State, event: GatewayEvent) {
        let selected = state.channel.clone();
        let fresh = |generation: u64, channel: &str| {
            current(
                generation,
                state.generation,
                Some(channel),
                selected.as_deref(),
            )
        };
        let applied = match event {
            GatewayEvent::Status {
                generation,
                channel,
                online,
                detail,
            } if fresh(generation, &channel) => {
                state.status = if online { "Live".into() } else { detail };
                Ok(Apply::Applied)
            }
            GatewayEvent::Message {
                generation,
                channel,
                message,
            } if fresh(generation, &channel) => {
                let mine = state.session.as_ref().map(|session| &session.author.id);
                if state.pending.as_ref().is_some_and(|pending| {
                    pending.client_id == message.client_message_id
                        && Some(&message.author.id) == mine
                }) {
                    state.pending = None;
                }
                state.timeline.apply(*message)
            }
            GatewayEvent::Reactions {
                generation,
                channel,
                update,
            } if fresh(generation, &channel) => state.timeline.apply_reactions(update),
            GatewayEvent::Attachments {
                generation,
                channel,
                update,
            } if fresh(generation, &channel) => state.timeline.apply_attachments(update),
            GatewayEvent::Embeds {
                generation,
                channel,
                update,
            } if fresh(generation, &channel) => {
                state.timeline.apply_embeds(*update);
                Ok(Apply::Applied)
            }
            GatewayEvent::Pin {
                generation,
                channel,
                update,
            } if fresh(generation, &channel) => state.timeline.apply_pin(*update),
            GatewayEvent::Forward {
                generation,
                channel,
                update,
            } if fresh(generation, &channel) => state.timeline.apply_forward(*update),
            GatewayEvent::Edit {
                generation,
                channel,
                update,
            } if fresh(generation, &channel) => state.timeline.apply_edit(*update),
            GatewayEvent::Resync {
                generation,
                channel,
            } if fresh(generation, &channel) => Ok(Apply::Resync),
            GatewayEvent::AccessDenied {
                generation,
                channel,
                detail,
            } if fresh(generation, &channel) => {
                state.session = None;
                state.session_error = Some(detail);
                Ok(Apply::Applied)
            }
            _ => Ok(Apply::Duplicate),
        };
        if matches!(applied, Ok(Apply::Resync) | Err(_)) {
            self.reload(state);
        }
    }

    fn render(self: &Rc<Self>) {
        self.render_rows();
        self.render_layout();
        let ui = &self.ui;
        let mut state = self.state.borrow_mut();
        ui.pages.set_visible_child_name(match state.page {
            Page::Loading => "loading",
            Page::SignIn => "signin",
            Page::Chat => "chat",
        });
        ui.steps
            .set_visible_child_name(if state.challenge.is_some() {
                "code"
            } else {
                "email"
            });
        ui.code_hint.set_label(&format!(
            "Enter the six-character code sent to {}. It expires in 10 minutes.",
            state.email
        ));
        ui.email_button.set_sensitive(!state.busy);
        ui.code_button.set_sensitive(!state.busy);
        ui.email_button
            .set_label(if state.busy { "Sending…" } else { "Continue" });
        ui.code_button.set_label(if state.busy {
            "Checking…"
        } else {
            "Continue"
        });
        let error = state.error.as_deref().unwrap_or_default();
        ui.signin_error.set_label(error);
        ui.signin_error.set_visible(!error.is_empty());
        ui.error.set_label(error);
        ui.error.set_visible(!error.is_empty());

        ui.channel_title
            .set_label(&if state.channel_name.is_empty() {
                String::new()
            } else {
                format!("# {}", state.channel_name)
            });
        ui.status.set_label(&state.status);
        let can_send = state.session.is_some();
        ui.composer.set_sensitive(can_send);
        ui.send.set_sensitive(can_send);
        let prompt = match &state.session_error {
            Some(error) => error.clone(),
            None => format!("Message #{}", state.channel_name),
        };
        ui.placeholder.set_label(&prompt);
        let send_error = state.send_error.as_deref().unwrap_or_default();
        ui.composer_error.set_label(send_error);
        ui.composer_error.set_visible(!send_error.is_empty());
        if let Some(draft) = state.returned_draft.take() {
            let buffer = ui.composer.buffer();
            if buffer.char_count() == 0 {
                drop(state);
                buffer.set_text(&draft);
            }
        }
    }

    /// Applies the row diff to the list's model: only new or changed rows
    /// get new items, so only they are bound again.
    fn render_rows(self: &Rc<Self>) {
        let changes: Vec<(usize, usize, Vec<glib::BoxedAnyObject>)> = {
            let mut state = self.state.borrow_mut();
            let state = &mut *state;
            let draft =
                state
                    .pending
                    .as_ref()
                    .zip(state.session.as_ref())
                    .map(|(pending, session)| {
                        (
                            pending.client_id.as_str(),
                            &session.author,
                            pending.created_at.as_str(),
                            pending.text.as_str(),
                        )
                    });
            let rows = rows::build(state.timeline.messages(), draft, &mut state.markup);
            let item = |index: usize| glib::BoxedAnyObject::new(rows[index].clone());
            let changes = rows::diff(&state.rows, &rows)
                .into_iter()
                .map(|op| match op {
                    Op::Replace(index) => (index, 1, vec![item(index)]),
                    Op::Splice {
                        position,
                        remove,
                        insert,
                    } => (position, remove, insert.map(item).collect()),
                })
                .collect();
            state.rows = rows;
            changes
        };
        let changed = !changes.is_empty();
        for (position, remove, items) in changes {
            self.ui.store.splice(position as u32, remove as u32, &items);
        }
        if changed && self.follow.get() {
            self.pin_tail();
        }
    }

    fn render_layout(self: &Rc<Self>) {
        let state = self.state.borrow();
        if self.rendered_layout.replace(state.layout) == state.layout {
            return;
        }
        let ui = &self.ui;
        clear(&ui.rail);
        for space in &state.spaces {
            // The visible label is two letters; the space's name is its accessible name.
            let button: gtk::Button = named("GtkButton", &space.name);
            button.set_label(&space_initials(&space.name));
            button.set_tooltip_text(Some(&space.name));
            if state.space.as_deref() == Some(&space.id) {
                button.add_css_class("selected");
            }
            let app = self.clone();
            let id = space.id.clone();
            button.connect_clicked(move |_| app.select_space(id.clone()));
            ui.rail.append(&button);
        }

        let detail = state.detail.as_ref();
        ui.space_name
            .set_label(detail.map_or("", |detail| detail.space.name.as_str()));
        clear(&ui.channels);
        let mut ids = Vec::new();
        for channel in detail.map_or(&[][..], |detail| &detail.channels) {
            let name = label(&format!("# {}", channel.name), &[]);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            if !channel.joined {
                name.add_css_class("muted");
            }
            ui.channels.append(&name);
            if state.channel.as_deref() == Some(&channel.id)
                && let Some(row) = ui.channels.row_at_index(ids.len() as i32)
            {
                ui.channels.select_row(Some(&row));
            }
            ids.push(channel.id.clone());
        }
        *self.channel_ids.borrow_mut() = ids;

        clear(&ui.members);
        let members = detail.map_or_else(Vec::new, |detail| {
            detail.member_order(state.channel.as_deref())
        });
        ui.members_title
            .set_label(&format!("Members — {}", members.len()));
        for member in members {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let avatar = label(&rows::initials(&member.display_name), &["avatar", "small"]);
            avatar.add_css_class(&format!("tint-{}", rows::tint(&member.id)));
            avatar.set_xalign(0.5);
            line.append(&avatar);
            let name = label(&member.display_name, &[]);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&name);
            if member.owner {
                line.append(&label("Owner", &["muted", "badge"]));
            }
            ui.members.append(&line);
        }
    }
}

/// The rail's label: the first word's first two letters, as desktop shows.
fn space_initials(name: &str) -> String {
    let mut chars = name.split_whitespace().next().unwrap_or("?").chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars.take(1)).collect())
        .unwrap_or_default()
}
