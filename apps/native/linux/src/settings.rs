//! Settings, as on the egui desktop: Startup (launch at login), Sounds,
//! Notifications ("Notify me about" and voice starting), Privacy (who can
//! start a DM, blocked accounts) and Delete account. Account choices save as
//! soon as they change and revert with an inline error. A child of `app`.

use super::{App, State};
use crate::app::named;
use crate::images::Icon;
use crate::ui::label;
use caper_client::effects::{Effect, Effects};
use caper_client::model::{DELETE_ACCOUNT_URL, DM_PRIVACY, NotificationLevel};
use caper_client::notifications::{self, Change, Scope};
use caper_client::worker::AccountOperation;
use gtk::{glib, prelude::*};
use std::rc::Rc;

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
    pub blocks_error: Option<String>,
}

pub(super) struct SettingsDialog {
    window: gtk::Window,
    startup_error: gtk::Label,
    sounds: gtk::Switch,
    voice_send: gtk::Switch,
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

        section(&root, "Startup", true);
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

        section(&root, "Sounds", false);
        let sounds = switch_row(
            &root,
            "Caper sound effects",
            "Play sounds for messages and app interactions.",
        );
        sounds.set_active(self.sound_effects());

        section(&root, "Voice messages", false);
        let voice_send = switch_row(
            &root,
            "Send voice messages as soon as I stop recording",
            "Off: a stopped recording waits in the composer with your text and files until you press Send.",
        );
        voice_send.set_active(self.voice_send_on_stop());
        let updates = self.updates_section(&root);

        let signed_in = self.state.borrow().account.is_some();
        let mut levels = Vec::new();
        let mut privacy = Vec::new();
        let voice_started = gtk::CheckButton::with_label("When voice starts in a space");
        let notifications_status = label("", &["small"]);
        let privacy_status = label("", &["small"]);
        let blocked = gtk::Box::new(gtk::Orientation::Vertical, 6);
        if signed_in {
            section(&root, "Notifications", false);
            root.append(&label("Notify me about", &["settings-label"]));
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

            section(&root, "Privacy", false);
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
        sounds.connect_active_notify(move |switch| {
            if let Some(app) = weak.upgrade() {
                app.set_sound_effects(switch.is_active());
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
            startup_error,
            sounds,
            voice_send,
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

    pub(super) fn sound_effects(&self) -> bool {
        self.prefs.get::<bool>("sound-effects-v1").unwrap_or(true)
    }

    fn set_sound_effects(&self, enabled: bool) {
        if enabled == self.sound_effects() {
            return;
        }
        self.prefs.set("sound-effects-v1", enabled);
        *self.effects.borrow_mut() = Effects::new(enabled);
        if enabled {
            self.play(Effect::ToggleOn);
        }
    }

    pub(super) fn play(&self, effect: Effect) {
        self.effects.borrow().play(effect);
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
            self.account_op(
                &state,
                AccountOperation::SaveNotifications { scope, change },
            );
        }
        self.render_settings();
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
        dialog.sounds.set_active(self.sound_effects());
        dialog.voice_send.set_active(self.voice_send_on_stop());
        // Notifications: only enabled once loaded.
        let level = settings.notifications.account_level();
        let saving = settings.notifications.saving(&Scope::Account);
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
        let (text, error) = if let Some(error) = settings.notifications.error(&Scope::Account) {
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
