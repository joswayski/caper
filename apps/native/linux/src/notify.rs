//! Notifications, as on the egui desktop: the account feed shows system
//! notifications (not for the conversation you're reading) and a click opens
//! where they came from; spaces, channels and DMs have Notifications and Mute
//! menus on right-click, and muted ones show a bell-slash. A child of `app`.

use super::{App, State, popup};
use caper_client::model::NotificationLevel;
use caper_client::notifications::{self, Change, Scope};
use caper_client::os_notifications::{Alert, Route};
use caper_client::worker::Command;
use chrono::Local;
use gtk::{gio, glib, prelude::*};
use std::rc::Rc;

/// A scope as a menu action target: `s:space`, `c:space:channel`, `d:id`.
fn encode(scope: &Scope) -> String {
    match scope {
        Scope::Account => "a".into(),
        Scope::Space(space) => format!("s:{space}"),
        Scope::Channel { space, channel } => format!("c:{space}:{channel}"),
        Scope::Direct(id) => format!("d:{id}"),
    }
}

fn decode(value: &str) -> Option<Scope> {
    let mut parts = value.splitn(3, ':');
    Some(match (parts.next()?, parts.next(), parts.next()) {
        ("a", None, None) => Scope::Account,
        ("s", Some(space), None) => Scope::Space(space.into()),
        ("c", Some(space), Some(channel)) => Scope::Channel {
            space: space.into(),
            channel: channel.into(),
        },
        ("d", Some(id), None) => Scope::Direct(id.into()),
        _ => return None,
    })
}

/// A menu choice, `kind|value|scope`, as the change it makes.
fn change(target: &str) -> Option<(Scope, Change)> {
    let mut parts = target.splitn(3, '|');
    let (kind, value, scope) = (parts.next()?, parts.next()?, decode(parts.next()?)?);
    let change = match kind {
        "level" => Change::Level(match value {
            "all" => Some(NotificationLevel::All),
            "mentions" => Some(NotificationLevel::Mentions),
            "nothing" => Some(NotificationLevel::Nothing),
            _ => None,
        }),
        "voice" => Change::VoiceStarted(match value {
            "on" => Some(true),
            "off" => Some(false),
            _ => None,
        }),
        "mute" => Change::Mute(match value {
            "off" => None,
            "forever" => Some(notifications::mute_value(None, chrono::Utc::now())),
            minutes => Some(notifications::mute_value(
                Some(minutes.parse().ok()?),
                chrono::Utc::now(),
            )),
        }),
        _ => return None,
    };
    Some((scope, change))
}

fn level_value(level: Option<NotificationLevel>) -> &'static str {
    match level {
        None => "default",
        Some(NotificationLevel::All) => "all",
        Some(NotificationLevel::Mentions) => "mentions",
        Some(NotificationLevel::Nothing) => "nothing",
    }
}

impl State {
    /// The bell-slash tooltip for a muted scope: its own mute, or "Muted
    /// with the space" for a channel in a muted space.
    pub(super) fn muted_label(&self, scope: &Scope) -> Option<String> {
        let now = chrono::Utc::now();
        let notifications = &self.settings.notifications;
        match notifications.mute(scope, now) {
            Some(mute) => Some(notifications::mute_label(mute, &now.with_timezone(&Local))),
            None => notifications
                .muted(scope, now)
                .then(|| "Muted with the space".into()),
        }
    }
}

impl App {
    /// Installs the `notify.choose` action the scope menus use.
    pub(super) fn install_notification_actions(self: &Rc<Self>) {
        let group = gio::SimpleActionGroup::new();
        let action = gio::SimpleAction::new("choose", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(self);
        action.connect_activate(move |_, parameter| {
            if let (Some(app), Some((scope, change))) = (
                weak.upgrade(),
                parameter
                    .and_then(|value| value.get::<String>())
                    .and_then(|target| change(&target)),
            ) {
                app.change_notifications(scope, change);
                app.state.borrow_mut().layout += 1;
                app.render();
            }
        });
        group.add_action(&action);
        self.ui.window.insert_action_group("notify", Some(&group));
    }

    /// Right-click opens `scope`'s Notifications and Mute menu.
    pub(super) fn scope_menu_on(self: &Rc<Self>, widget: &impl IsA<gtk::Widget>, scope: Scope) {
        let click = gtk::GestureClick::new();
        click.set_button(gtk::gdk::BUTTON_SECONDARY);
        let (weak, anchor) = (Rc::downgrade(self), widget.clone().upcast::<gtk::Widget>());
        click.connect_pressed(move |gesture, _, _, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(app) = weak.upgrade() {
                app.scope_menu(&scope, &anchor);
            }
        });
        widget.add_controller(click);
    }

    fn scope_menu(self: &Rc<Self>, scope: &Scope, anchor: &gtk::Widget) {
        let state = self.state.borrow();
        let settings = &state.settings.notifications;
        let now = chrono::Utc::now();
        let key = encode(scope);
        let item = |label: &str, kind: &str, value: &str, chosen: bool| {
            let item = gio::MenuItem::new(
                Some(&if chosen {
                    format!("✓  {label}")
                } else {
                    format!("    {label}")
                }),
                None,
            );
            item.set_action_and_target_value(
                Some("notify.choose"),
                Some(&format!("{kind}|{value}|{key}").to_variant()),
            );
            item
        };
        let menu = gio::Menu::new();
        // Owners' and members' management items come first, as on desktop.
        if let Some(admin) = Self::admin_menu(&state, scope) {
            menu.append_section(None, &admin);
        }
        if !settings.ready() || settings.saving(scope) {
            let note = gio::MenuItem::new(
                Some(settings.load_error().unwrap_or(if settings.ready() {
                    "Saving…"
                } else {
                    "Loading…"
                })),
                Some("notify.none"),
            );
            menu.append_item(&note);
            popup(&gtk::PopoverMenu::from_model(Some(&menu)), anchor);
            return;
        }
        let noun = match scope {
            Scope::Space(_) => "space",
            Scope::Channel { .. } => "channel",
            _ => "conversation",
        };
        let level = settings.level(scope);
        let top = gio::Menu::new();
        if let Scope::Direct(_) = scope {
            let off = level == Some(NotificationLevel::Nothing);
            top.append_item(&item(
                if off {
                    "Turn on notifications"
                } else {
                    "Turn off notifications"
                },
                "level",
                if off { "default" } else { "nothing" },
                false,
            ));
        } else {
            let levels = gio::Menu::new();
            let inherited = settings.inherited(scope);
            for choice in [
                None,
                Some(NotificationLevel::All),
                Some(NotificationLevel::Mentions),
                Some(NotificationLevel::Nothing),
            ] {
                let label = choice.map_or_else(
                    || notifications::default_label(inherited),
                    |level| notifications::level_label(level).into(),
                );
                levels.append_item(&item(&label, "level", level_value(choice), level == choice));
            }
            top.append_submenu(Some("Notifications"), &levels);
        }
        if matches!(scope, Scope::Space(_)) {
            let voice = gio::Menu::new();
            let current = settings.voice_started(scope);
            for (value, label, choice) in [
                ("default", "Default (account)", None),
                ("on", "On", Some(true)),
                ("off", "Off", Some(false)),
            ] {
                voice.append_item(&item(label, "voice", value, current == choice));
            }
            top.append_submenu(Some("When voice starts"), &voice);
        }
        menu.append_section(None, &top);
        let mutes = gio::Menu::new();
        if let Some(mute) = settings.mute(scope, now) {
            mutes.append_item(&item(&format!("Unmute {noun}"), "mute", "off", false));
            mutes.append_item(&gio::MenuItem::new(
                Some(&notifications::mute_label(mute, &now.with_timezone(&Local))),
                Some("notify.none"),
            ));
        } else {
            let presets = gio::Menu::new();
            for (label, minutes) in notifications::MUTE_PRESETS {
                let value = minutes.map_or_else(|| "forever".to_owned(), |m| m.to_string());
                presets.append_item(&item(label, "mute", &value, false));
            }
            mutes.append_submenu(Some(&format!("Mute {noun}")), &presets);
        }
        if let Scope::Channel { space, .. } = scope
            && settings.mute(&Scope::Space(space.clone()), now).is_some()
        {
            mutes.append_item(&gio::MenuItem::new(
                Some("Muted with the space"),
                Some("notify.none"),
            ));
        }
        menu.append_section(None, &mutes);
        if let Some(error) = settings.error(scope) {
            let errors = gio::Menu::new();
            errors.append_item(&gio::MenuItem::new(Some(error), Some("notify.none")));
            menu.append_section(None, &errors);
        }
        drop(state);
        popup(&gtk::PopoverMenu::from_model(Some(&menu)), anchor);
    }

    /// The account feed: system notifications and live profile changes.
    pub(super) fn start_notifications(&self, state: &State) {
        if let Some(token) = state.token.clone() {
            self.worker.send(Command::StartNotifications {
                epoch: state.epoch,
                token,
            });
        }
    }

    /// A notification was clicked: go where it came from.
    pub(super) fn open_alert(self: &Rc<Self>, alert: Alert) {
        self.ui.window.present();
        self.close_forward();
        self.close_settings();
        let route = {
            let state = self.state.borrow();
            if state.token.is_none() || state.needs_profile() {
                return;
            }
            let known = alert.conversation_id.as_ref().map_or_else(
                || {
                    alert
                        .space_id
                        .as_ref()
                        .is_some_and(|space| state.spaces.iter().any(|item| &item.id == space))
                },
                |id| state.directs.iter().any(|direct| &direct.id == id),
            );
            let team = state
                .account
                .as_ref()
                .is_some_and(|account| account.feedback_team);
            alert.route(team, known)
        };
        match route {
            Some(Route::Direct(id)) => {
                self.open_direct(id);
                let state = self.state.borrow();
                self.load_directs(&state);
            }
            Some(Route::Channel { space, channel }) => {
                {
                    let mut state = self.state.borrow_mut();
                    if state.channel.as_deref() != Some(&channel) {
                        self.navigate(&mut state, Some(space), Some(channel));
                    }
                }
                self.render();
            }
            // Invitations open their space once accepted elsewhere; the
            // Caper team's report visits aren't on Linux yet.
            Some(Route::Invitation { space, .. }) => {
                let known = self
                    .state
                    .borrow()
                    .spaces
                    .iter()
                    .any(|item| item.id == space);
                if known {
                    self.select_space(space);
                }
            }
            Some(Route::Visit(_)) | None => {}
        }
    }

    /// The window reads the open conversation only while it has focus, so
    /// a notification for it is suppressed then and shown otherwise.
    pub(super) fn renew_reading(&self) {
        let state = self.state.borrow();
        self.worker.showing(
            state
                .channel
                .as_deref()
                .filter(|_| self.ui.window.is_active() && state.page == Some(super::Page::Chat)),
        );
    }

    /// Redraws the sidebar when the next timed mute ends.
    pub(super) fn schedule_mute_expiry(self: &Rc<Self>) {
        if self.mute_timer.get() {
            return;
        }
        let Some(delay) = self
            .state
            .borrow()
            .settings
            .notifications
            .next_expiry(chrono::Utc::now())
        else {
            return;
        };
        self.mute_timer.set(true);
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(delay + std::time::Duration::from_millis(250), move || {
            if let Some(app) = weak.upgrade() {
                app.mute_timer.set(false);
                app.state.borrow_mut().layout += 1;
                app.render();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_targets_round_trip_their_scope_and_change() {
        for scope in [
            Scope::Space("space1".into()),
            Scope::Channel {
                space: "space1".into(),
                channel: "chan1".into(),
            },
            Scope::Direct("dm1".into()),
        ] {
            let (back, change) = change(&format!("level|mentions|{}", encode(&scope))).unwrap();
            assert_eq!(back, scope);
            assert_eq!(change, Change::Level(Some(NotificationLevel::Mentions)));
        }
        assert_eq!(change("level|default|s:x").unwrap().1, Change::Level(None));
        assert_eq!(change("mute|off|d:x").unwrap().1, Change::Mute(None));
        assert_eq!(
            change("mute|forever|d:x").unwrap().1,
            Change::Mute(Some("forever".into()))
        );
        let Change::Mute(Some(until)) = change("mute|60|c:s:c").unwrap().1 else {
            panic!("a timed mute")
        };
        let until = chrono::DateTime::parse_from_rfc3339(&until).unwrap();
        let minutes = (until.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_minutes();
        assert!((58..=60).contains(&minutes));
        assert_eq!(
            change("voice|off|s:x").unwrap().1,
            Change::VoiceStarted(Some(false))
        );
        assert!(change("mute|soon|d:x").is_none());
        assert!(change("level|all|x:y").is_none());
    }
}
