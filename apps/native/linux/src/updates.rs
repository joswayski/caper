//! Updates and What's new, as on the egui desktop. Settings shows the
//! installed version and its commit, "Check for updates" with its status
//! and "What's new". A release build with `caper-updater` beside it checks
//! in the background (`caper_client::updates`); an update shows "Update
//! available" above the account bar and opens its review: the highlights
//! being installed, a failed install's error with "Send this to the Caper
//! team", and Restart to update (or Download when this copy can't update in
//! place). What's new lists this revision's release highlights
//! (`caper_client::whats_new`), and opens by itself once after an update
//! brings unread ones. A child of `app`.

use super::{App, Page, State, clear};
use crate::images::Icon;
use crate::ui::label;
use caper_client::updates::{self, Available, Status};
use caper_client::whats_new::{self, REVISION, Release};
use caper_client::worker::Command;
use gtk::{glib, prelude::*};
use std::rc::Rc;
use std::time::Instant;

/// A failed install sent to the Caper team from the review.
pub(super) struct Report {
    error: String,
    pub client_id: String,
    pub state: Result<bool, String>,
}

/// The Updates section's widgets in the open Settings.
pub(super) struct UpdatesRow {
    check: gtk::Button,
    status: gtk::Label,
    note: gtk::Label,
}

pub(super) struct WhatsNewWindow {
    window: gtk::Window,
    body: gtk::Box,
    /// What the body shows, so it redraws only when this changes.
    shown: Option<(bool, Option<String>, Vec<u64>)>,
}

/// What the review shows: the update, a failed install's error, its
/// report and whether you are in a call.
type Review = (
    Available,
    Option<String>,
    Option<Result<bool, String>>,
    bool,
);

pub(super) struct UpdateWindow {
    window: gtk::Window,
    body: gtk::Box,
    shown: Option<Review>,
}

fn link(text: &str, url: &str, classes: &[&str]) -> gtk::LinkButton {
    let button = gtk::LinkButton::with_label(url, text);
    for class in classes {
        button.add_css_class(class);
    }
    button.set_halign(gtk::Align::Start);
    button
}

/// "Version 0.1.82" (or the short commit) linking the release's commit.
fn release_label(release: &Release) -> String {
    release.version.as_ref().map_or_else(
        || release.revision[..7].to_owned(),
        |version| format!("Version {version}"),
    )
}

/// The 36 px Caper face heading What's new and the review; decorative.
fn face() -> gtk::Image {
    let face = gtk::Image::new();
    if let Some(texture) = crate::images::caper_face(72) {
        face.set_paintable(Some(&texture));
    }
    face.set_pixel_size(36);
    face.set_halign(gtk::Align::Start);
    face
}

fn highlight(title: &str, body: &str) -> gtk::Box {
    let item = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let heading = label(&format!("• {title}"), &["highlight-title"]);
    heading.set_xalign(0.0);
    heading.set_wrap(true);
    item.append(&heading);
    let text = label(body, &["muted", "highlight-body"]);
    text.set_xalign(0.0);
    text.set_wrap(true);
    item.append(&text);
    item
}

impl State {
    /// The workspace is open and settled: What's new may load and open.
    fn whats_new_ready(&self) -> bool {
        self.page() == Page::Chat && self.token.is_some() && !self.needs_first_space()
    }
}

impl App {
    pub(super) fn start_updates(self: &Rc<Self>) {
        {
            let mut whats_new = self.whats_new.borrow_mut();
            let stored = self.prefs.get::<String>(whats_new.key());
            whats_new.restore(stored.as_deref());
        }
        self.ui.update_dock.connect_clicked({
            let app = Rc::downgrade(self);
            move |_| {
                if let Some(app) = app.upgrade() {
                    app.updates.borrow_mut().show_available();
                    app.render_updates();
                }
            }
        });
        let app = Rc::downgrade(self);
        self.ui.window.connect_is_active_notify(move |_| {
            if let Some(app) = app.upgrade() {
                glib::idle_add_local_once(move || app.poll_whats_new());
            }
        });
        // Highlights may be published after the install: retry each minute
        // while in front, through the same bounded gate.
        let app = Rc::downgrade(self);
        glib::timeout_add_seconds_local(60, move || {
            let Some(app) = app.upgrade() else {
                return glib::ControlFlow::Break;
            };
            app.poll_whats_new();
            glib::ControlFlow::Continue
        });
    }

    fn load_whats_new(&self, manual: bool) {
        if self.whats_new.borrow_mut().begin(Instant::now(), manual) {
            self.worker.send(Command::WhatsNew);
        }
    }

    /// Loads this revision's highlights once the workspace is open, and
    /// opens unread ones when nothing else is in the way.
    pub(super) fn poll_whats_new(self: &Rc<Self>) {
        if !self.ui.window.is_active() {
            return;
        }
        let open = {
            let Ok(state) = self.state.try_borrow() else {
                return;
            };
            if !state.whats_new_ready() {
                return;
            }
            self.load_whats_new(false);
            let quiet = matches!(
                self.voice.borrow().state.phase,
                caper_client::state::Phase::Idle
            ) && !self.updates.borrow().has_available()
                && self.whats_new_window.borrow().is_none()
                && self.settings.borrow().is_none()
                && self.inbox_window.borrow().is_none()
                && self.profile.borrow().is_none()
                && self.pins.borrow().is_none();
            quiet && self.whats_new.borrow_mut().open_unread()
        };
        if open {
            self.show_whats_new();
        }
    }

    /// Settings' "What's new": the whole history for this revision.
    pub(super) fn open_whats_new(self: &Rc<Self>) {
        self.whats_new.borrow_mut().open_history();
        self.load_whats_new(true);
        self.show_whats_new();
    }

    fn show_whats_new(self: &Rc<Self>) {
        if let Some(open) = self.whats_new_window.borrow().as_ref() {
            open.window.present();
            return;
        }
        let window = gtk::Window::builder()
            .transient_for(&self.ui.window)
            .modal(true)
            .title("What’s new")
            .default_width(480)
            .default_height(560)
            .build();
        window.add_css_class("caper");
        window.add_css_class("dialog");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("dialog-body");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let heading = gtk::Box::new(gtk::Orientation::Vertical, 2);
        heading.set_hexpand(true);
        heading.append(&face());
        let title = label("What’s new", &["dialog-title"]);
        title.set_xalign(0.0);
        heading.append(&title);
        let version = updates::version_label();
        match updates::commit_url(REVISION) {
            Some(url) => heading.append(&link(&version, &url, &["release-link"])),
            None => {
                let text = label(&version, &["muted", "small"]);
                text.set_xalign(0.0);
                heading.append(&text);
            }
        }
        top.append(&heading);
        let close: gtk::Button = super::named("GtkButton", "Close What’s new");
        close.set_child(Some(&self.images.icon_image(Icon::Close, "#b9bcbe", 14)));
        close.add_css_class("flat");
        close.set_valign(gtk::Align::Start);
        top.append(&close);
        root.append(&top);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vexpand(true)
                .child(&body)
                .build(),
        );
        let done = gtk::Button::with_label("Done");
        done.add_css_class("primary");
        done.set_halign(gtk::Align::End);
        root.append(&done);
        window.set_child(Some(&root));
        for button in [&close, &done] {
            let weak = window.downgrade();
            button.connect_clicked(move |_| {
                if let Some(window) = weak.upgrade() {
                    window.close();
                }
            });
        }
        let app = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(app) = app.upgrade() {
                app.whats_new_window.borrow_mut().take();
                // Only what was shown counts as read.
                let mut whats_new = app.whats_new.borrow_mut();
                whats_new.dismiss();
                if let Some(value) = whats_new.stored() {
                    app.prefs.set(whats_new.key(), value);
                }
            }
            glib::Propagation::Proceed
        });
        *self.whats_new_window.borrow_mut() = Some(WhatsNewWindow {
            window: window.clone(),
            body,
            shown: None,
        });
        self.render_whats_new();
        window.present();
        done.grab_focus();
    }

    fn render_whats_new(self: &Rc<Self>) {
        let Ok(mut open) = self.whats_new_window.try_borrow_mut() else {
            return;
        };
        let Some(open) = open.as_mut() else {
            return;
        };
        let Ok(mut whats_new) = self.whats_new.try_borrow_mut() else {
            return;
        };
        let loading = whats_new.loading;
        let error = whats_new.error.clone();
        let releases = if loading || error.is_some() {
            Vec::new()
        } else {
            whats_new.displayed()
        };
        drop(whats_new);
        let shown = (
            loading,
            error.clone(),
            releases.iter().map(|release| release.sequence).collect(),
        );
        if open.shown.as_ref() == Some(&shown) {
            return;
        }
        open.shown = Some(shown);
        clear(&open.body);
        if loading {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let spinner = gtk::Spinner::new();
            spinner.start();
            line.append(&spinner);
            line.append(&label("Loading release highlights…", &[]));
            open.body.append(&line);
            return;
        }
        if let Some(error) = error {
            let text = label(&error, &["error"]);
            text.set_xalign(0.0);
            text.set_wrap(true);
            open.body.append(&text);
            let retry = gtk::Button::with_label("Retry");
            retry.set_halign(gtk::Align::Start);
            retry.set_margin_top(8);
            let app = Rc::downgrade(self);
            retry.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    app.load_whats_new(true);
                    app.render_whats_new();
                }
            });
            open.body.append(&retry);
            return;
        }
        if releases.is_empty() {
            let text = label("No release highlights for this version yet.", &["muted"]);
            text.set_xalign(0.0);
            open.body.append(&text);
            return;
        }
        let mut previous_day = None;
        for release in releases {
            let day = chrono::DateTime::parse_from_rfc3339(&release.published_at)
                .ok()
                .map(|published| published.date_naive());
            if day != previous_day {
                let date = label(
                    &day.map(|day| day.format("%B %-d, %Y").to_string())
                        .unwrap_or_default(),
                    &["settings-section"],
                );
                date.set_xalign(0.0);
                if previous_day.is_some() {
                    date.set_margin_top(20);
                }
                open.body.append(&date);
                previous_day = day;
            }
            let version = link(
                &release_label(&release),
                &updates::commit_url(&release.revision).unwrap_or_default(),
                &["release-link"],
            );
            version.set_margin_top(12);
            open.body.append(&version);
            for item in &release.highlights {
                let item = highlight(&item.title, &item.body);
                item.set_margin_top(10);
                open.body.append(&item);
            }
            let rule = gtk::Separator::new(gtk::Orientation::Horizontal);
            rule.set_margin_top(16);
            open.body.append(&rule);
        }
    }

    /// The Updates section of Settings, after Sounds.
    pub(super) fn updates_section(self: &Rc<Self>, root: &gtk::Box) -> UpdatesRow {
        let rule = gtk::Separator::new(gtk::Orientation::Horizontal);
        rule.add_css_class("settings-rule");
        root.append(&rule);
        root.append(&label("Updates", &["settings-section"]));
        let installed = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        installed.append(&label(&updates::installed_version(), &["muted"]));
        installed.append(&link(
            &REVISION[..7.min(REVISION.len())],
            &format!("https://github.com/joswayski/caper/commit/{REVISION}"),
            &["release-link"],
        ));
        root.append(&installed);
        let check = gtk::Button::with_label("Check for updates");
        check.set_halign(gtk::Align::Start);
        root.append(&check);
        // Always shown, so a fast check never moves the controls.
        let status = label("", &[]);
        status.set_xalign(0.0);
        status.set_ellipsize(gtk::pango::EllipsizeMode::End);
        root.append(&status);
        let note = label("", &["muted", "small"]);
        note.set_xalign(0.0);
        root.append(&note);
        let whats_new = gtk::Button::with_label("What’s new");
        whats_new.set_halign(gtk::Align::Start);
        whats_new.set_margin_top(4);
        root.append(&whats_new);
        let app = Rc::downgrade(self);
        check.connect_clicked(move |_| {
            if let Some(app) = app.upgrade() {
                app.updates.borrow_mut().check_now();
                app.render_updates();
            }
        });
        let app = Rc::downgrade(self);
        whats_new.connect_clicked(move |_| {
            if let Some(app) = app.upgrade() {
                app.open_whats_new();
            }
        });
        UpdatesRow {
            check,
            status,
            note,
        }
    }

    /// Settings' check status, the dock's "Update available", the review
    /// and What's new, from the latest state.
    pub(super) fn render_updates(self: &Rc<Self>) {
        let (status, can_check, known, review) = {
            let updates = self.updates.borrow();
            (
                updates.status(),
                updates.can_check(),
                updates.known_available(),
                updates.available(),
            )
        };
        if let Ok(dialog) = self.settings.try_borrow()
            && let Some(dialog) = dialog.as_ref()
        {
            let row = &dialog.updates;
            row.check
                .set_sensitive(can_check && status != Status::Checking);
            let (message, error) = match &status {
                Status::UpToDate => ("No newer published release is available.".into(), false),
                Status::Available => ("An update is available to review.".into(), false),
                Status::Deferred if known.is_some() => {
                    ("An update is available to review.".into(), false)
                }
                Status::Deferred => ("No new updates. Will check again shortly.".into(), false),
                Status::Failed(error) => (format!("Could not check for updates: {error}"), true),
                Status::Checking => ("Checking for updates…".into(), false),
                Status::Idle => ("Updates are checked automatically.".into(), false),
            };
            row.status.set_label(&message);
            row.status.set_tooltip_text(Some(&message));
            if error {
                row.status.add_css_class("error");
            } else {
                row.status.remove_css_class("error");
            }
            row.status.set_opacity(if error { 1.0 } else { 0.7 });
            row.note.set_label(if !can_check && status == Status::Idle {
                "Update checks are available in packaged release builds."
            } else {
                "Checks automatically every minute."
            });
        }
        self.ui.update_dock.set_visible(known.is_some());
        if let Some(update) = &known {
            self.ui.update_dock.set_tooltip_text(Some(&format!(
                "Caper {} is available. Review the update before restarting.",
                update.version
            )));
        }
        match review {
            Some(update) => self.show_update(update),
            None => {
                if let Some(open) = self.update_window.borrow_mut().take() {
                    open.window.destroy();
                }
            }
        }
        self.render_whats_new();
    }

    fn show_update(self: &Rc<Self>, update: Available) {
        if self.update_window.borrow().is_none() {
            let window = gtk::Window::builder()
                .transient_for(&self.ui.window)
                .modal(true)
                .title("Update")
                .default_width(452)
                .resizable(false)
                .build();
            window.add_css_class("caper");
            window.add_css_class("dialog");
            let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
            body.add_css_class("dialog-body");
            window.set_child(Some(&body));
            let app = Rc::downgrade(self);
            window.connect_close_request(move |_| {
                if let Some(app) = app.upgrade() {
                    app.update_window.borrow_mut().take();
                    app.updates.borrow_mut().dismiss();
                    let app = app.clone();
                    glib::idle_add_local_once(move || app.render_updates());
                }
                glib::Propagation::Proceed
            });
            *self.update_window.borrow_mut() = Some(UpdateWindow {
                window: window.clone(),
                body,
                shown: None,
            });
            window.present();
        }
        let error = self
            .updates
            .borrow()
            .error
            .clone()
            .or(update.last_error.clone());
        let report = self
            .state
            .borrow()
            .update_report
            .as_ref()
            .and_then(|report| {
                (Some(&report.error) == error.as_ref()).then(|| report.state.clone())
            });
        let in_call = !matches!(
            self.voice.borrow().state.phase,
            caper_client::state::Phase::Idle
        );
        let mut open = self.update_window.borrow_mut();
        let Some(open) = open.as_mut() else {
            return;
        };
        let shown = (update.clone(), error.clone(), report.clone(), in_call);
        if open.shown.as_ref() == Some(&shown) {
            return;
        }
        open.shown = Some(shown);
        let body = &open.body;
        clear(body);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let face = face();
        face.set_hexpand(true);
        top.append(&face);
        let close: gtk::Button = super::named("GtkButton", "Close update review");
        close.set_child(Some(&self.images.icon_image(Icon::Close, "#b9bcbe", 14)));
        close.add_css_class("flat");
        close.set_valign(gtk::Align::Start);
        top.append(&close);
        body.append(&top);
        let title = label("A fresh Caper is ready", &["update-title"]);
        title.set_xalign(0.0);
        body.append(&title);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let version = format!("Version {}", update.version);
        match update.revision.as_deref().and_then(updates::commit_url) {
            Some(url) => line.append(&link(&version, &url, &["release-link"])),
            None => line.append(&label(&version, &["muted", "small"])),
        }
        if let Some(date) = update
            .released
            .as_deref()
            .and_then(|date| chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
        {
            line.append(&label(
                &format!(" · {}", date.format("%B %-d, %Y")),
                &["muted", "small"],
            ));
        }
        body.append(&line);
        if let Some(error) = &error {
            let heading = label("The last update didn’t finish", &["error", "author"]);
            heading.set_xalign(0.0);
            heading.set_margin_top(8);
            body.append(&heading);
            if update.can_apply {
                let sentence = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                sentence.append(&label("Try again using the button below, or ", &["muted"]));
                sentence.append(&link(
                    "use the installer",
                    updates::DOWNLOAD_URL,
                    &["inline-link"],
                ));
                sentence.append(&label(".", &["muted"]));
                body.append(&sentence);
            } else {
                let text = label("Download the installer with the button below.", &["muted"]);
                text.set_xalign(0.0);
                body.append(&text);
            }
            let detail = label(error, &["muted", "small"]);
            detail.set_xalign(0.0);
            detail.set_wrap(true);
            detail.set_selectable(true);
            body.append(
                &gtk::ScrolledWindow::builder()
                    .hscrollbar_policy(gtk::PolicyType::Never)
                    .propagate_natural_height(true)
                    .max_content_height(48)
                    .child(&detail)
                    .build(),
            );
            let can_report = self.state.borrow().account.as_ref().is_some_and(|account| {
                account.username.is_some() && self.state.borrow().token.is_some()
            });
            if can_report {
                match &report {
                    Some(Ok(true)) => {
                        let sent = label(
                            "Sent. The Caper team will reply in your notes.",
                            &["muted", "small"],
                        );
                        sent.set_xalign(0.0);
                        body.append(&sent);
                    }
                    Some(Ok(false)) => {
                        let sending = gtk::Button::with_label("Sending to the Caper team…");
                        sending.set_sensitive(false);
                        sending.set_halign(gtk::Align::Start);
                        body.append(&sending);
                    }
                    failed => {
                        if let Some(Err(message)) = failed {
                            let text = label(message, &["error", "small"]);
                            text.set_xalign(0.0);
                            body.append(&text);
                        }
                        let send = gtk::Button::with_label("Send this to the Caper team");
                        send.set_halign(gtk::Align::Start);
                        let (app, error, target) =
                            (Rc::downgrade(self), error.clone(), update.version.clone());
                        send.connect_clicked(move |_| {
                            if let Some(app) = app.upgrade() {
                                app.send_update_report(error.clone(), &target);
                            }
                        });
                        body.append(&send);
                    }
                }
            }
        }
        let rule = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        rule.set_margin_top(8);
        rule.append(&label("WHAT’S NEW", &["update-rule"]));
        let line_rule = gtk::Separator::new(gtk::Orientation::Horizontal);
        line_rule.set_hexpand(true);
        line_rule.set_valign(gtk::Align::Center);
        rule.append(&line_rule);
        body.append(&rule);
        let notes = gtk::Box::new(gtk::Orientation::Vertical, 14);
        if !update.history_complete {
            let text = label(
                "Earlier release notes aren’t available.",
                &["muted", "small"],
            );
            text.set_xalign(0.0);
            notes.append(&text);
        }
        match &update.highlights {
            Some(items) if items.is_empty() => {
                let text = label("Behind-the-scenes fixes this time.", &["muted"]);
                text.set_xalign(0.0);
                notes.append(&text);
            }
            Some(items) => {
                for item in items {
                    notes.append(&highlight(&item.title, &item.body));
                }
            }
            // The highlights could not be read: the updater's plain notes
            // and the signed changelog, unversioned.
            None => {
                let lines = std::iter::once(update.notes.as_str()).chain(
                    update
                        .changelog
                        .iter()
                        .flat_map(|entry| entry.notes.lines())
                        .filter(|line| !line.trim().is_empty()),
                );
                for line in lines {
                    let text = label(line, &["muted"]);
                    text.set_xalign(0.0);
                    text.set_wrap(true);
                    notes.append(&text);
                }
            }
        }
        body.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .propagate_natural_height(true)
                .max_content_height(300)
                .child(&notes)
                .build(),
        );
        body.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        if !update.can_apply {
            let text = label(
                "Download the installer to update this copy manually.",
                &["muted", "small"],
            );
            text.set_xalign(0.0);
            body.append(&text);
        }
        if let Some(size) = update.download_label().filter(|_| update.can_apply) {
            let text = label(&size, &["muted", "small"]);
            text.set_xalign(0.0);
            body.append(&text);
        }
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.set_halign(gtk::Align::End);
        actions.set_margin_top(4);
        let later = gtk::Button::with_label("Later");
        actions.append(&later);
        let primary = gtk::Button::with_label(match (update.can_apply, in_call) {
            (false, _) => "Download",
            (true, true) => "Restart and leave call",
            (true, false) => "Restart to update",
        });
        primary.add_css_class("primary");
        actions.append(&primary);
        body.append(&actions);
        for button in [&close, &later] {
            let weak = open.window.downgrade();
            button.connect_clicked(move |_| {
                if let Some(window) = weak.upgrade() {
                    window.close();
                }
            });
        }
        let (app, sequence, can_apply) = (Rc::downgrade(self), update.sequence, update.can_apply);
        primary.connect_clicked(move |_| {
            let Some(app) = app.upgrade() else { return };
            if !can_apply {
                gtk::show_uri(Some(&app.ui.window), updates::DOWNLOAD_URL, 0);
                return;
            }
            let applied = app.updates.borrow_mut().apply();
            if applied {
                // The new build will not repeat these highlights.
                if let Some(sequence) = sequence {
                    let mut whats_new = app.whats_new.borrow_mut();
                    whats_new.mark_read(sequence);
                    if let Some(value) = whats_new.stored() {
                        app.prefs.set(whats_new.key(), value);
                    }
                }
                app.voice.borrow_mut().leave();
                if let Some(application) = app.ui.window.application() {
                    application.quit();
                }
            } else {
                app.render_updates();
            }
        });
        later.grab_focus();
    }

    /// Posts the failed install to the Caper team from your notes.
    fn send_update_report(self: &Rc<Self>, error: String, target_version: &str) {
        {
            let mut state = self.state.borrow_mut();
            let (Some(token), Some(account)) = (state.token.clone(), state.account.clone()) else {
                return;
            };
            let Some(username) = account.username.clone() else {
                return;
            };
            let text = format!(
                "@feedback My Caper update didn’t finish.\n\n{error}\n\n{} → Caper {target_version} on Linux",
                updates::installed_version().trim_start_matches("Installed: "),
            );
            let client_id = uuid::Uuid::new_v4().to_string();
            state.update_report = Some(Report {
                error,
                client_id: client_id.clone(),
                state: Ok(false),
            });
            self.worker.send(Command::SendProfile {
                epoch: state.epoch,
                person: account.id.clone(),
                token,
                send: Box::new(caper_client::profile_dm::Send {
                    person: account.id.clone(),
                    username,
                    name: account.display_name.clone().unwrap_or_default(),
                    client_id,
                    text,
                    files: Vec::new(),
                    settings: state.uploads.clone(),
                    cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    retrying: false,
                    open_only: false,
                    voice_only: false,
                    direct: None,
                }),
            });
        }
        self.render_updates();
    }

    /// `WhatsNew` answered.
    pub(super) fn whats_new_loaded(&self, result: Result<whats_new::Snapshot, String>) {
        let mut whats_new = self.whats_new.borrow_mut();
        whats_new.finish(result);
        if let Some(value) = whats_new.stored() {
            self.prefs.set(whats_new.key(), value);
        }
    }
}
