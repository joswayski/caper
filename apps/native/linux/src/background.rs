//! Closing the window minimizes Caper instead of quitting
//! (`caper_client::background`), so a call and notifications carry on. Not
//! hidden: many desktops (GNOME) have no tray to find a hidden app in, while
//! a minimized window stays in the taskbar or dock and Alt+Tab. Quit Caper
//! (the account menu, Ctrl+Q) quits. A child of `app`.

use super::App;
use crate::host::Signal;
use caper_client::background;
use gtk::{glib, prelude::*};
use std::rc::Rc;

impl App {
    /// Whether closing the window keeps Caper running.
    pub(super) fn keep_running(&self) -> bool {
        background::enabled(self.prefs.get::<String>(background::KEY).as_deref())
    }

    pub(super) fn set_keep_running(&self, on: bool) {
        self.prefs.set(background::KEY, on.to_string());
    }

    /// Closing minimizes while Caper keeps running. A close that reaches a
    /// window minimized this way, where its X can't be clicked, is a dock's
    /// or taskbar's Quit, so it quits.
    pub(super) fn follow_close(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.ui.window.connect_close_request(move |window| {
            let Some(app) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            // Closing or quitting: edited drafts reach the server now.
            if let Ok(mut state) = app.state.try_borrow_mut() {
                app.flush_drafts(&mut state);
            }
            if app.quitting.get()
                || !app.keep_running()
                || (app.closed_away.get() && !window.is_active())
            {
                return glib::Propagation::Proceed;
            }
            window.minimize();
            app.closed_away.set(true);
            app.announce_background();
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(self);
        self.ui.window.connect_is_active_notify(move |window| {
            if window.is_active()
                && let Some(app) = weak.upgrade()
            {
                app.closed_away.set(false);
            }
        });
    }

    /// Quit Caper: closes the window (saving its size) and ends the app,
    /// with any other windows it has open.
    pub(super) fn quit_app(&self) {
        self.quitting.set(true);
        self.ui.window.close();
        if let Some(application) = self.ui.window.application() {
            application.quit();
        }
    }

    /// Says where Caper went: every time during a call, otherwise once.
    fn announce_background(&self) {
        let call = background::ongoing(&self.voice.borrow().state.phase).cloned();
        let noticed = self.prefs.get::<bool>(background::NOTICE_KEY) == Some(true);
        let Some((title, body)) = background::notice(call.as_ref(), noticed) else {
            return;
        };
        if call.is_none() {
            self.prefs.set(background::NOTICE_KEY, true);
        }
        let sender = self.wakes.sender.clone();
        background::show(title, body, move || {
            let _ = sender.try_send(Signal::Front);
        });
    }
}
