//! Settings, the one screen every client shares: Profile (Edit profile),
//! Appearance, Notifications (Do Not Disturb, "Notify me about" and voice
//! starting), Voice & audio, Startup (launch at login, and keeping Caper
//! running when its window closes), Privacy & blocked accounts (with Delete
//! account), About (version, updates, What's new), Log out and Quit Caper.
//! Your avatar in the account bar opens Do Not Disturb's menu. Account
//! choices save as soon as they change and revert with an inline error. A
//! child of `app`.

use super::{App, State};
use crate::app::named;
use crate::images::{Avatar, AvatarView, Icon};
use crate::ui::label;
use caper_client::model::{DELETE_ACCOUNT_URL, DM_PRIVACY, NotificationLevel};
use caper_client::notifications::{self, Change, PAUSE_PRESETS, Scope};
use caper_client::worker::AccountOperation;
use gtk::{glib, prelude::*};
use std::rc::Rc;

/// A theme's colours and rules. Its provider outranks style.css, so a light
/// theme's `.composer` border and focus ring would come back on the floating
/// composers: they keep web's soft light shadow, focused or not.
pub(super) fn theme_css(doc: &caper_client::themes::ThemeDocument) -> String {
    let mut css = doc.to_gtk_css();
    if doc.is_light() {
        css.push_str(
            ".composer.floating,\n\
             .composer.floating:focus-within {\n\
               border-color: transparent;\n\
               box-shadow: 0 2px 8px rgba(15, 23, 42, 0.06);\n\
             }\n",
        );
    }
    css
}

const LEVELS: [NotificationLevel; 3] = [
    NotificationLevel::All,
    NotificationLevel::Mentions,
    NotificationLevel::Nothing,
];

/// Account settings kept in the app's state.
#[derive(Default)]
pub(super) struct AccountSettings {
    pub privacy: Option<String>,
    pub privacy_saving: bool,
    pub privacy_error: Option<String>,
    pub notifications: notifications::Notifications,
    /// The latest account notification change was Do Not Disturb, so a
    /// failed save shows its error there.
    pub pause_last: bool,
    pub blocks_error: Option<String>,
}

/// What a failed Do Not Disturb change says.
const PAUSE_ERROR: &str = "That change wasn’t saved. Try again.";

/// Do Not Disturb's end label ("Until 3:45 PM"), or `None` while it is off.
fn pause_label(notifications: &notifications::Notifications) -> Option<String> {
    notifications
        .paused(chrono::Utc::now())
        .map(|pause| notifications::pause_label(pause, &chrono::Local::now()))
}

/// The `pausedUntil` a Do Not Disturb preset sends now.
fn pause_change(preset: notifications::PausePreset) -> Change {
    Change::Pause(Some(notifications::pause_value(
        preset,
        &chrono::Local::now(),
    )))
}

pub(super) struct SettingsDialog {
    window: gtk::Window,
    profile_avatar: AvatarView,
    profile_name: gtk::Label,
    profile_username: gtk::Label,
    obsidian_card: gtk::Button,
    paper_card: gtk::Button,
    startup_error: gtk::Label,
    voice_send: gtk::Switch,
    connection: gtk::Button,
    diagnostics: gtk::Button,
    dnd_status: gtk::Label,
    dnd_presets: Vec<gtk::Button>,
    dnd_off: gtk::Button,
    dnd_error: gtk::Label,
    levels: Vec<(NotificationLevel, gtk::CheckButton)>,
    voice_started: gtk::CheckButton,
    notifications_status: gtk::Label,
    privacy: Vec<(&'static str, gtk::CheckButton)>,
    privacy_status: gtk::Label,
    blocked: gtk::Box,
    /// The blocked accounts the list shows, so it rebuilds only on change.
    blocked_shown: Option<Vec<String>>,
    /// The Updates section.
    pub(super) updates: super::updates::UpdatesRow,
}

fn theme_preset_card(id: &str, title: &str, mode: &str) -> gtk::Button {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let swatch = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    swatch.add_css_class("theme-swatch");
    swatch.add_css_class(&format!("theme-swatch-{id}"));
    swatch.set_height_request(54);

    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 4);
    sidebar.add_css_class("theme-swatch-sidebar");
    sidebar.set_width_request(44);
    for classes in [
        &["theme-swatch-bar", "brand"][..],
        &["theme-swatch-bar", "muted", "short"][..],
        &["theme-swatch-bar", "muted", "med"][..],
    ] {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        for class in classes {
            bar.add_css_class(class);
        }
        sidebar.append(&bar);
    }
    swatch.append(&sidebar);

    let canvas = gtk::Box::new(gtk::Orientation::Vertical, 4);
    canvas.add_css_class("theme-swatch-canvas");
    canvas.set_hexpand(true);
    let top_bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    top_bar.add_css_class("theme-swatch-bar");
    top_bar.add_css_class("primary");
    canvas.append(&top_bar);
    let sub_bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    sub_bar.add_css_class("theme-swatch-bar");
    sub_bar.add_css_class("muted");
    sub_bar.add_css_class("short");
    canvas.append(&sub_bar);
    let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    canvas.append(&spacer);
    let pill = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    pill.add_css_class("theme-swatch-pill");
    pill.set_halign(gtk::Align::End);
    canvas.append(&pill);
    swatch.append(&canvas);
    card.append(&swatch);

    let meta = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name = label(title, &["theme-card-name"]);
    name.set_hexpand(true);
    meta.append(&name);
    meta.append(&label(mode, &["muted", "small"]));
    card.append(&meta);

    let button: gtk::Button = named("GtkButton", &format!("{title} ({mode})"));
    button.add_css_class("theme-card");
    button.set_hexpand(true);
    button.set_child(Some(&card));
    button
}

fn section(root: &gtk::Box, title: &str, first: bool) {
    if !first {
        let rule = gtk::Separator::new(gtk::Orientation::Horizontal);
        rule.add_css_class("settings-rule");
        root.append(&rule);
    }
    let heading = label(title, &["settings-section"]);
    root.append(&heading);
}

/// A switch with its title and a quieter description, like the desktop's.
fn switch_row(root: &gtk::Box, title: &str, detail: &str) -> gtk::Switch {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 8);
    text.set_hexpand(true);
    text.append(&label(title, &["settings-label"]));
    let detail = label(detail, &["muted", "small"]);
    detail.set_wrap(true);
    text.append(&detail);
    row.append(&text);
    let switch = crate::app::named::<gtk::Switch>("GtkSwitch", title);
    switch.set_valign(gtk::Align::Center);
    row.append(&switch);
    root.append(&row);
    switch
}

fn status(label: &gtk::Label, text: Option<&str>, error: bool) {
    label.set_label(text.unwrap_or_default());
    label.set_visible(text.is_some());
    if error {
        label.add_css_class("error");
    } else {
        label.remove_css_class("error");
    }
}

impl App {
    pub(super) fn open_settings(self: &Rc<Self>) {
        if let Some(dialog) = self.settings.borrow().as_ref() {
            dialog.window.present();
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            if !state.settings.privacy_saving {
                state.settings.privacy_error = None;
                self.account_op(&state, AccountOperation::LoadPrivacy);
            }
            state.settings.blocks_error = None;
            self.account_op(&state, AccountOperation::LoadBlocks);
            self.load_notifications(&mut state);
        }
        let window = gtk::Window::builder()
            .transient_for(&self.ui.window)
            .modal(true)
            .title("Settings")
            .default_width(460)
            .default_height(640)
            .build();
        window.add_css_class("caper");
        window.add_css_class("dialog");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.add_css_class("dialog-body");
        root.append(&label("Settings", &["dialog-title"]));
        let signed_in = self.state.borrow().account.is_some();

        // Profile: your picture, name and @username, and Edit profile,
        // which opens over Settings and returns to it.
        let profile_avatar = AvatarView::new(48, false);
        let profile_name = label("", &["author"]);
        profile_name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let profile_username = label("", &["muted", "small"]);
        profile_username.set_ellipsize(gtk::pango::EllipsizeMode::End);
        if signed_in {
            section(&root, "Profile", true);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.append(&profile_avatar.root);
            let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
            names.set_hexpand(true);
            names.set_valign(gtk::Align::Center);
            names.append(&profile_name);
            names.append(&profile_username);
            row.append(&names);
            let edit = gtk::Button::with_label("Edit profile");
            edit.set_valign(gtk::Align::Center);
            let weak = Rc::downgrade(self);
            edit.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.open_profile_editor();
                }
            });
            row.append(&edit);
            root.append(&row);
        }

        section(&root, "Appearance", !signed_in);
        let theme_note = label(
            "Choose between Obsidian Dark and Paper Light.",
            &["muted", "small"],
        );
        theme_note.set_wrap(true);
        root.append(&theme_note);
        let preset_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        preset_row.set_homogeneous(true);
        let obsidian_card = theme_preset_card("obsidian", "Obsidian", "Dark");
        let paper_card = theme_preset_card("paper", "Paper", "Light");
        preset_row.append(&obsidian_card);
        preset_row.append(&paper_card);
        root.append(&preset_row);

        {
            let app = Rc::downgrade(self);
            obsidian_card.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    app.apply_theme_preset("obsidian");
                    app.render_settings();
                }
            });
        }
        {
            let app = Rc::downgrade(self);
            paper_card.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    app.apply_theme_preset("paper");
                    app.render_settings();
                }
            });
        }

        let mut levels = Vec::new();
        let mut privacy = Vec::new();
        let voice_started = gtk::CheckButton::with_label("When voice starts in a space");
        let notifications_status = label("", &["small"]);
        let privacy_status = label("", &["small"]);
        let blocked = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let dnd_status = label("", &["muted", "small"]);
        let mut dnd_presets = Vec::new();
        let dnd_off: gtk::Button = named("GtkButton", "Turn off Do Not Disturb");
        crate::app::show_text(&dnd_off, "Turn off");
        let dnd_error = label("", &["error", "small"]);
        if signed_in {
            section(&root, "Notifications", false);
            // Do Not Disturb: its state, the presets, and Turn off.
            root.append(&label("Do Not Disturb", &["settings-label"]));
            let state_line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            dnd_status.set_hexpand(true);
            dnd_status.set_xalign(0.0);
            state_line.append(&dnd_status);
            dnd_off.set_valign(gtk::Align::Center);
            state_line.append(&dnd_off);
            root.append(&state_line);
            let rows = [
                gtk::Box::new(gtk::Orientation::Horizontal, 8),
                gtk::Box::new(gtk::Orientation::Horizontal, 8),
            ];
            for (index, (title, preset)) in PAUSE_PRESETS.into_iter().enumerate() {
                let button = gtk::Button::with_label(title);
                let weak = Rc::downgrade(self);
                button.connect_clicked(move |_| {
                    if let Some(app) = weak.upgrade() {
                        app.change_notifications(Scope::Account, pause_change(preset));
                    }
                });
                rows[usize::from(index >= 3)].append(&button);
                dnd_presets.push(button);
            }
            for row in rows {
                root.append(&row);
            }
            let weak = Rc::downgrade(self);
            dnd_off.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.change_notifications(Scope::Account, Change::Pause(None));
                }
            });
            dnd_error.set_wrap(true);
            root.append(&dnd_error);

            root.append(&label(
                "Notify me about",
                &["settings-label", "settings-gap"],
            ));
            let mut group: Option<gtk::CheckButton> = None;
            for level in LEVELS {
                let check = gtk::CheckButton::with_label(notifications::account_level_label(level));
                check.set_group(group.as_ref());
                group.get_or_insert_with(|| check.clone());
                let weak = Rc::downgrade(self);
                check.connect_toggled(move |check| {
                    if check.is_active()
                        && let Some(app) = weak.upgrade()
                    {
                        app.change_notifications(Scope::Account, Change::Level(Some(level)));
                    }
                });
                root.append(&check);
                levels.push((level, check));
            }
            let weak = Rc::downgrade(self);
            voice_started.connect_toggled(move |check| {
                if let Some(app) = weak.upgrade() {
                    let current = app
                        .state
                        .borrow()
                        .settings
                        .notifications
                        .account_voice_started();
                    if current != check.is_active() {
                        app.change_notifications(
                            Scope::Account,
                            Change::VoiceStarted(Some(check.is_active())),
                        );
                    }
                }
            });
            root.append(&voice_started);
            notifications_status.set_wrap(true);
            root.append(&notifications_status);
        }

        section(&root, "Voice & audio", false);
        let voice_send = switch_row(
            &root,
            "Send voice messages as soon as I stop recording",
            "Off: a stopped recording waits in the composer with your text and files until you press Send.",
        );
        voice_send.set_active(self.voice_send_on_stop());
        let audio_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        audio_buttons.add_css_class("settings-gap");
        let audio_test = gtk::Button::with_label("Audio test");
        // Connection details show during a call, Audio diagnostics for
        // debug accounts (`render_settings`).
        let connection = gtk::Button::with_label("Connection details");
        let diagnostics = gtk::Button::with_label("Audio diagnostics");
        for (button, run) in [(&audio_test, 0), (&connection, 1), (&diagnostics, 2)] {
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    match run {
                        0 => app.open_audio(),
                        other => app.open_report(other == 1),
                    }
                }
            });
            audio_buttons.append(button);
        }
        root.append(&audio_buttons);

        section(&root, "Startup", false);
        let startup = switch_row(
            &root,
            "Launch at login",
            "Open Caper when you sign in to your computer.",
        );
        let startup_error = label("", &["error", "small"]);
        startup_error.set_wrap(true);
        root.append(&startup_error);
        match caper_client::startup::enabled() {
            Ok(enabled) => {
                startup.set_active(enabled);
                status(&startup_error, None, false);
            }
            Err(error) => status(
                &startup_error,
                Some(&format!("Could not read startup settings: {error}")),
                true,
            ),
        }

        let keep_running = switch_row(
            &root,
            caper_client::background::SETTING_TITLE,
            caper_client::background::SETTING_DETAIL,
        );
        keep_running.set_active(self.keep_running());
        if let Some(row) = keep_running.parent() {
            row.set_margin_top(8);
        }

        if signed_in {
            section(&root, "Privacy & blocked accounts", false);
            root.append(&label("Who can start a DM with you", &["settings-label"]));
            let mut group: Option<gtk::CheckButton> = None;
            for (value, title, detail) in DM_PRIVACY {
                let check = gtk::CheckButton::with_label(title);
                check.set_group(group.as_ref());
                group.get_or_insert_with(|| check.clone());
                let weak = Rc::downgrade(self);
                check.connect_toggled(move |check| {
                    if check.is_active()
                        && let Some(app) = weak.upgrade()
                    {
                        app.save_privacy(value);
                    }
                });
                root.append(&check);
                if let Some(detail) = detail {
                    let detail = label(detail, &["muted", "small", "radio-detail"]);
                    detail.set_wrap(true);
                    root.append(&detail);
                }
                privacy.push((value, check));
            }
            privacy_status.set_wrap(true);
            root.append(&privacy_status);
            root.append(&label(
                "Blocked accounts",
                &["settings-label", "settings-gap"],
            ));
            root.append(&blocked);
            // As desktop: the link leaves Caper, so it says so with an icon.
            let delete: gtk::LinkButton = named("GtkLinkButton", "Delete account");
            delete.set_uri(DELETE_ACCOUNT_URL);
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            content.append(&label("Delete account", &[]));
            content.append(&self.images.icon_image(Icon::ExternalLink, "#e5735a", 12));
            delete.set_child(Some(&content));
            delete.add_css_class("danger-link");
            delete.set_halign(gtk::Align::Start);
            root.append(&delete);
        }

        let updates = self.updates_section(&root);

        let rule = gtk::Separator::new(gtk::Orientation::Horizontal);
        rule.add_css_class("settings-rule");
        root.append(&rule);
        // Closing the window can keep Caper running, so it quits from here.
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        if signed_in {
            let log_out = gtk::Button::with_label("Log out");
            let weak = Rc::downgrade(self);
            log_out.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.sign_out();
                }
            });
            actions.append(&log_out);
        }
        let quit = gtk::Button::with_label("Quit Caper");
        let weak = Rc::downgrade(self);
        quit.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.quit_app();
            }
        });
        actions.append(&quit);
        root.append(&actions);

        let weak = Rc::downgrade(self);
        startup.connect_state_set(move |switch, requested| {
            let Some(app) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let result = caper_client::startup::set_enabled(requested);
            if let Some(dialog) = app.settings.borrow().as_ref() {
                match &result {
                    Ok(()) => status(&dialog.startup_error, None, false),
                    Err(error) => status(
                        &dialog.startup_error,
                        Some(&format!("Could not change startup settings: {error}")),
                        true,
                    ),
                }
            }
            if result.is_err() {
                // Stay where the system is.
                switch.set_state(!requested);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        keep_running.connect_active_notify(move |switch| {
            if let Some(app) = weak.upgrade() {
                app.set_keep_running(switch.is_active());
            }
        });
        let weak = Rc::downgrade(self);
        voice_send.connect_active_notify(move |switch| {
            if let Some(app) = weak.upgrade() {
                app.set_voice_send_on_stop(switch.is_active());
                app.render();
            }
        });

        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&root)
            .build();
        window.set_child(Some(&scroll));
        let weak = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(app) = weak.upgrade() {
                app.settings.borrow_mut().take();
            }
            glib::Propagation::Proceed
        });
        let keys = gtk::EventControllerKey::new();
        let weak = window.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape
                && let Some(window) = weak.upgrade()
            {
                window.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
        *self.settings.borrow_mut() = Some(SettingsDialog {
            window: window.clone(),
            profile_avatar,
            profile_name,
            profile_username,
            obsidian_card,
            paper_card,
            startup_error,
            voice_send,
            connection,
            diagnostics,
            dnd_status,
            dnd_presets,
            dnd_off,
            dnd_error,
            levels,
            voice_started,
            notifications_status,
            privacy,
            privacy_status,
            blocked,
            blocked_shown: None,
            updates,
        });
        self.render_settings();
        self.render_updates();
        window.present();
    }

    pub(super) fn close_settings(&self) {
        let dialog = self.settings.borrow_mut().take();
        if let Some(dialog) = dialog {
            dialog.window.destroy();
        }
    }

    pub(super) fn current_theme(&self) -> caper_client::themes::ThemeDocument {
        let parsed = self
            .prefs
            .get::<String>(caper_client::themes::THEME_PREF_KEY)
            .and_then(|raw| caper_client::themes::parse_theme_preference(&raw).ok());
        match parsed {
            Some(doc) if doc.id == "paper" || doc.is_light() => caper_client::themes::paper(),
            _ => caper_client::themes::obsidian(),
        }
    }

    pub(super) fn apply_theme_preset(&self, id: &str) {
        let doc = if id == "paper" {
            caper_client::themes::paper()
        } else {
            caper_client::themes::obsidian()
        };
        self.prefs
            .set(caper_client::themes::THEME_PREF_KEY, doc.id.clone());
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_application_prefer_dark_theme(!doc.is_light());
        }
        self.theme_style.load_from_data(&theme_css(&doc));
    }

    pub(super) fn load_notifications(&self, state: &mut State) {
        if state.token.is_some()
            && let Some(revision) = state.settings.notifications.start_load()
        {
            self.account_op(state, AccountOperation::LoadNotifications { revision });
        }
    }

    /// Applies a choice at once and saves it; a failed save reverts with
    /// an inline error.
    pub(super) fn change_notifications(self: &Rc<Self>, scope: Scope, change: Change) {
        {
            // Render mirrors saved values into the buttons, which toggles
            // them while the state is borrowed: nothing to save then.
            let Ok(mut state) = self.state.try_borrow_mut() else {
                return;
            };
            let unchanged = matches!(
                (&scope, &change),
                (Scope::Account, Change::Level(Some(level)))
                    if state.settings.notifications.account_level() == Some(*level)
            );
            if unchanged
                || state.token.is_none()
                || !state.settings.notifications.begin(&scope, &change)
            {
                drop(state);
                self.render_settings();
                return;
            }
            if scope == Scope::Account {
                state.settings.pause_last = matches!(change, Change::Pause(_));
            }
            // The account bar shows Do Not Disturb, rows their mutes.
            state.layout += 1;
            self.account_op(
                &state,
                AccountOperation::SaveNotifications { scope, change },
            );
        }
        self.render_layout();
        self.render_settings();
    }

    /// The window a dialog opened now sits over: Settings while it is
    /// open (Edit profile, Audio test, What's new), else the main window.
    pub(super) fn dialog_parent(&self) -> gtk::Window {
        self.settings
            .try_borrow()
            .ok()
            .and_then(|dialog| dialog.as_ref().map(|dialog| dialog.window.clone()))
            .unwrap_or_else(|| self.ui.window.clone().upcast())
    }

    /// The account bar's moon: "Pause notifications" with the presets
    /// while Do Not Disturb is off; its end, Turn off and the presets (to
    /// change the end) while it is on.
    pub(super) fn dnd_menu(self: &Rc<Self>, anchor: &gtk::Button) {
        let (paused, ready, error) = {
            let mut state = self.state.borrow_mut();
            if !state.settings.notifications.ready() {
                self.load_notifications(&mut state);
            }
            let notifications = &state.settings.notifications;
            (
                pause_label(notifications),
                notifications.ready() && !notifications.saving(&Scope::Account),
                state.settings.pause_last && notifications.error(&Scope::Account).is_some(),
            )
        };
        let popover = gtk::Popover::new();
        popover.add_css_class("menu");
        let items = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let heading = match &paused {
            Some(until) => format!("Do Not Disturb · {until}"),
            None => "Pause notifications".to_owned(),
        };
        let title = label(&heading, &["title"]);
        title.set_xalign(0.0);
        title.set_margin_start(12);
        title.set_margin_end(12);
        title.set_margin_top(6);
        title.set_margin_bottom(4);
        items.append(&title);
        let add = |text: &str, preset: Option<notifications::PausePreset>| {
            let button = gtk::Button::with_label(text);
            button.add_css_class("flat");
            button.add_css_class("menu-item");
            button.set_sensitive(ready);
            if let Some(label) = button.child().and_downcast::<gtk::Label>() {
                label.set_xalign(0.0);
            }
            let (weak, menu) = (Rc::downgrade(self), popover.clone());
            button.connect_clicked(move |_| {
                menu.popdown();
                if let Some(app) = weak.upgrade() {
                    // A preset's time counts from the click.
                    let change = preset.map_or(Change::Pause(None), pause_change);
                    app.change_notifications(Scope::Account, change);
                }
            });
            items.append(&button);
        };
        if paused.is_some() {
            add("Turn off", None);
            items.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        }
        for (text, preset) in PAUSE_PRESETS {
            add(text, Some(preset));
        }
        if error {
            let line = label(PAUSE_ERROR, &["error", "small"]);
            line.set_wrap(true);
            line.set_max_width_chars(28);
            line.set_xalign(0.0);
            line.set_margin_start(12);
            line.set_margin_end(12);
            line.set_margin_bottom(6);
            items.append(&line);
        }
        popover.set_child(Some(&items));
        super::popup(&popover, anchor.upcast_ref());
    }

    fn save_privacy(self: &Rc<Self>, value: &str) {
        {
            let Ok(mut state) = self.state.try_borrow_mut() else {
                return;
            };
            let settings = &mut state.settings;
            if settings.privacy_saving || settings.privacy.as_deref() == Some(value) {
                drop(state);
                self.render_settings();
                return;
            }
            let previous = settings.privacy.replace(value.to_owned());
            settings.privacy_saving = true;
            settings.privacy_error = None;
            self.account_op(
                &state,
                AccountOperation::SavePrivacy {
                    value: value.to_owned(),
                    previous,
                },
            );
        }
        self.render_settings();
    }

    pub(super) fn render_settings(self: &Rc<Self>) {
        let Ok(mut dialog) = self.settings.try_borrow_mut() else {
            return;
        };
        let Some(dialog) = dialog.as_mut() else {
            return;
        };
        let Ok(state) = self.state.try_borrow() else {
            return;
        };
        let settings = &state.settings;
        let is_paper = self.current_theme().id == "paper";
        if is_paper {
            dialog.obsidian_card.remove_css_class("selected");
            dialog.paper_card.add_css_class("selected");
        } else {
            dialog.obsidian_card.add_css_class("selected");
            dialog.paper_card.remove_css_class("selected");
        }
        if let Some(account) = &state.account {
            let name = state.name();
            dialog.profile_avatar.set(
                &self.images,
                &Avatar::new(
                    &name,
                    account.avatar_id,
                    &account.avatar_asset,
                    &account.avatar_url,
                ),
            );
            dialog.profile_name.set_label(&name);
            dialog.profile_username.set_label(
                &account
                    .username
                    .as_ref()
                    .map(|username| format!("@{username}"))
                    .unwrap_or_default(),
            );
        }
        dialog.voice_send.set_active(self.voice_send_on_stop());
        dialog.connection.set_visible(
            self.voice
                .try_borrow()
                .is_ok_and(|voice| voice.diagnostics.is_some()),
        );
        dialog.diagnostics.set_visible(
            state
                .account
                .as_ref()
                .is_some_and(|account| account.debug_enabled),
        );
        // Notifications: only enabled once loaded.
        let level = settings.notifications.account_level();
        let saving = settings.notifications.saving(&Scope::Account);
        let account_error = settings.notifications.error(&Scope::Account);
        // Do Not Disturb.
        let paused = pause_label(&settings.notifications);
        dialog.dnd_status.set_label(
            &paused
                .as_ref()
                .map_or_else(|| "Off".to_owned(), |until| format!("On · {until}")),
        );
        for button in &dialog.dnd_presets {
            button.set_sensitive(level.is_some() && !saving);
        }
        dialog.dnd_off.set_visible(paused.is_some());
        dialog.dnd_off.set_sensitive(!saving);
        let pause_error = settings.pause_last && account_error.is_some();
        status(&dialog.dnd_error, pause_error.then_some(PAUSE_ERROR), true);
        for (choice, check) in &dialog.levels {
            check.set_sensitive(level.is_some());
            if level == Some(*choice) && !check.is_active() {
                check.set_active(true);
            }
        }
        dialog
            .voice_started
            .set_sensitive(level.is_some() && !saving);
        let voice = settings.notifications.account_voice_started();
        if dialog.voice_started.is_active() != voice {
            dialog.voice_started.set_active(voice);
        }
        let (text, error) = if let Some(error) = account_error.filter(|_| !pause_error) {
            (Some(error.to_owned()), true)
        } else if level.is_none() {
            match settings.notifications.load_error() {
                Some(error) => (Some(error.to_owned()), true),
                None => (Some("Loading…".to_owned()), false),
            }
        } else {
            (None, false)
        };
        status(&dialog.notifications_status, text.as_deref(), error);
        // Privacy.
        let loaded = settings.privacy.is_some();
        for (value, check) in &dialog.privacy {
            check.set_sensitive(loaded);
            if settings.privacy.as_deref() == Some(*value) && !check.is_active() {
                check.set_active(true);
            }
        }
        let (text, error) = match &settings.privacy_error {
            Some(error) => (Some(error.clone()), true),
            None if !loaded => (Some("Loading…".to_owned()), false),
            None => (None, false),
        };
        status(&dialog.privacy_status, text.as_deref(), error);
        // Blocked accounts, rebuilt only when the list changes.
        let ids = state
            .blocks
            .as_ref()
            .map(|blocks| blocks.iter().map(|block| block.id.clone()).collect());
        let shown = (ids.clone(), settings.blocks_error.clone(), state.block_busy);
        if dialog.blocked_shown.as_ref() != shown.0.as_ref()
            || shown.1.is_some()
            || dialog.blocked.first_child().is_none()
        {
            while let Some(child) = dialog.blocked.first_child() {
                dialog.blocked.remove(&child);
            }
            match &state.blocks {
                None if shown.1.is_none() => {
                    dialog
                        .blocked
                        .append(&label("Loading…", &["muted", "small"]));
                }
                None => {}
                Some(blocks) if blocks.is_empty() => {
                    dialog
                        .blocked
                        .append(&label("You haven’t blocked anyone.", &["muted", "small"]));
                }
                Some(blocks) => {
                    for account in blocks {
                        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                        row.add_css_class("blocked-account");
                        let view = crate::images::AvatarView::new(28, false);
                        view.set(
                            &self.images,
                            &crate::images::Avatar::new(
                                &account.display_name,
                                account.avatar_id,
                                &account.avatar_asset,
                                &account.avatar_url,
                            ),
                        );
                        row.append(&view.root);
                        let names = gtk::Box::new(gtk::Orientation::Vertical, 0);
                        names.set_hexpand(true);
                        names.append(&label(&account.display_name, &["field"]));
                        names.append(&label(
                            &format!("@{}", account.username),
                            &["muted", "small"],
                        ));
                        row.append(&names);
                        let unblock = gtk::Button::with_label("Unblock");
                        unblock.set_valign(gtk::Align::Center);
                        let (weak, account) = (Rc::downgrade(self), account.clone());
                        unblock.connect_clicked(move |_| {
                            if let Some(app) = weak.upgrade() {
                                app.set_block(account.clone(), false, None);
                            }
                        });
                        row.append(&unblock);
                        dialog.blocked.append(&row);
                    }
                }
            }
            if let Some(error) = &shown.1 {
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                line.append(&label(error, &["error", "small"]));
                let retry = gtk::Button::with_label("Try again");
                retry.add_css_class("flat");
                retry.add_css_class("inline");
                let weak = Rc::downgrade(self);
                retry.connect_clicked(move |_| {
                    if let Some(app) = weak.upgrade() {
                        {
                            let mut state = app.state.borrow_mut();
                            state.settings.blocks_error = None;
                            app.account_op(&state, AccountOperation::LoadBlocks);
                        }
                        app.render_settings();
                    }
                });
                line.append(&retry);
                dialog.blocked.append(&line);
            }
            dialog.blocked_shown = ids;
        }
        for row in std::iter::successors(dialog.blocked.first_child(), |row| row.next_sibling()) {
            if let Some(button) = row.last_child().and_downcast::<gtk::Button>() {
                button.set_sensitive(!state.block_busy);
            }
        }
    }
}
