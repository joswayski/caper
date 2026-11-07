//! Narrow windows (760 px or less), as on the egui desktop and phones:
//! Browse (the space rail and channel sidebar) and the conversation take
//! turns at the full width. Browse in the conversation header opens it;
//! choosing a channel or DM, or Close navigation, goes back. The member
//! list slides over the conversation below its header, over a backdrop
//! that closes it, and an open thread takes the conversation's place. A
//! child of `app`.

use super::App;
use gtk::{gdk, glib, prelude::*};
use std::rc::Rc;

/// The widest window that is narrow.
const NARROW: i32 = 760;
/// The member drawer's width, leaving the conversation 64 px beside it.
const DRAWER: i32 = 280;
/// How long the drawer takes to arrive; it leaves at once.
const ARRIVE_MS: u32 = 200;

impl App {
    pub(super) fn connect_narrow(self: &Rc<Self>) {
        // The surface's size is the window's, even while the wide layout
        // asks for more.
        let app = Rc::downgrade(self);
        self.ui.window.connect_realize(move |window| {
            let Some(surface) = window.surface() else {
                return;
            };
            let app = app.clone();
            surface.connect_layout(move |_, width, _| {
                if let Some(app) = app.upgrade() {
                    app.set_narrow(width <= NARROW);
                }
            });
        });
        let app = Rc::downgrade(self);
        self.ui.browse.connect_clicked(move |_| {
            if let Some(app) = app.upgrade() {
                app.set_browsing(true);
            }
        });
        let app = Rc::downgrade(self);
        self.ui.browse_close.connect_clicked(move |_| {
            if let Some(app) = app.upgrade() {
                app.set_browsing(false);
            }
        });
        let click = gtk::GestureClick::new();
        let app = Rc::downgrade(self);
        click.connect_released(move |_, _, _, _| {
            if let Some(app) = app.upgrade() {
                app.ui.members_toggle.set_active(false);
            }
        });
        self.ui.members_backdrop.add_controller(click);
        // Escape closes the member drawer, then Browse.
        let keys = gtk::EventControllerKey::new();
        let app = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(app) = app.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if key != gdk::Key::Escape || !modifiers.is_empty() || !app.narrow.get() {
                return glib::Propagation::Proceed;
            }
            if app.ui.members_drawer.reveals_child() {
                app.ui.members_toggle.set_active(false);
            } else if app.browsing.get() && app.state.borrow().channel.is_some() {
                app.set_browsing(false);
            } else {
                return glib::Propagation::Proceed;
            }
            glib::Propagation::Stop
        });
        self.ui.window.add_controller(keys);
    }

    /// Whether the window is narrow, already while GTK lays out a size the
    /// surface's `layout` signal hasn't reported yet.
    pub(super) fn is_narrow(&self) -> bool {
        self.narrow.get()
            || self
                .ui
                .window
                .surface()
                .is_some_and(|surface| surface.width() <= NARROW)
    }

    /// Switches layouts as the window crosses 760 px. The member list moves
    /// between its pane and the drawer, and a wide window's choice to show
    /// it comes back with the width; a narrow one starts with it closed.
    fn set_narrow(self: &Rc<Self>, narrow: bool) {
        if self.narrow.replace(narrow) == narrow {
            return;
        }
        let ui = &self.ui;
        if narrow {
            self.wide_members.set(ui.members_toggle.is_active());
            ui.content_split.set_end_child(None::<&gtk::Widget>);
            ui.members_panel.set_size_request(DRAWER, -1);
            ui.members_drawer.set_child(Some(&ui.members_panel));
            ui.members_toggle.set_active(false);
        } else {
            ui.members_drawer.set_child(None::<&gtk::Widget>);
            ui.members_panel.set_size_request(220, -1);
            ui.content_split.set_end_child(Some(&ui.members_panel));
            ui.members_toggle.set_active(self.wide_members.get());
            self.browsing.set(false);
            self.place_sidebar();
            self.place_members();
        }
        self.render();
    }

    /// Opens (`true`) or leaves Browse in a narrow window.
    pub(super) fn set_browsing(self: &Rc<Self>, browsing: bool) {
        if self.browsing.replace(browsing) != browsing {
            self.render();
            // Focus follows: the hidden button can't keep it.
            if browsing {
                self.ui.browse_close.grab_focus();
            } else if self.narrow.get() {
                self.ui.browse.grab_focus();
            }
        }
    }

    /// Which half of a narrow window shows, after `render` has decided the
    /// panels a wide one would show: `shown` is whether a conversation and
    /// a thread are open.
    pub(super) fn render_narrow(&self, (conversation, thread_open): (bool, bool)) {
        let ui = &self.ui;
        let narrow = self.narrow.get();
        // With nothing open there is only Browse.
        let browsing = narrow && (self.browsing.get() || !conversation);
        ui.browse.set_visible(narrow);
        ui.browse_close.set_visible(browsing && conversation);
        ui.rail.set_visible(!narrow || browsing);
        ui.sidebar.set_visible(!narrow || browsing);
        ui.sidebar.set_hexpand(browsing);
        // A narrow thread takes the conversation's place beside it.
        ui.content_split.set_visible(!narrow || !browsing);
        ui.stage.set_visible(!narrow || !thread_open);
        ui.thread_panel.set_visible(thread_open && !browsing);
        ui.thread_panel.set_hexpand(narrow);
        ui.thread_panel
            .set_size_request(if narrow { -1 } else { 300 }, -1);
        if narrow {
            ui.members_panel.set_visible(true);
            let open = ui.members_toggle.is_active() && ui.members_toggle.is_visible();
            // Arrives, and leaves at once.
            ui.members_drawer
                .set_transition_duration(if open { ARRIVE_MS } else { 0 });
            ui.members_drawer.set_reveal_child(open);
            ui.members_backdrop.set_visible(open);
        } else {
            ui.members_backdrop.set_visible(false);
        }
        // A narrow switch can hide the focused widget, and the next key
        // would only find focus again.
        let lost = GtkWindowExt::focus(&ui.window).is_none_or(|focus| !focus.is_visible());
        if narrow && lost {
            if browsing {
                ui.browse_close.grab_focus();
            } else if ui.composer.view.is_visible() {
                ui.composer.view.grab_focus();
            } else {
                ui.browse.grab_focus();
            }
        }
    }
}
