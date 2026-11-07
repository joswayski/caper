//! `BENCH`'s scripted scenarios (`caper_client::bench`), inert otherwise:
//! frames counted from the frame clock's paints, the messages scrolled up
//! through their adjustment and characters typed into the composer. A child
//! of `app`.

use super::App;
use caper_client::bench::{Action, Bench};
use gtk::{glib, prelude::*};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

type Shared = Rc<RefCell<Bench>>;

impl App {
    pub(super) fn connect_bench(self: &Rc<Self>, process_start: Instant) {
        let bench = Bench::from_env("caper-linux", process_start);
        if !bench.active() {
            return;
        }
        let bench = Rc::new(RefCell::new(bench));
        let app = Rc::downgrade(self);
        self.ui.window.connect_realize(move |window| {
            let Some(clock) = window.frame_clock() else {
                return;
            };
            let (app, bench) = (app.clone(), bench.clone());
            clock.connect_after_paint(move |_| {
                bench.borrow_mut().painted();
                // The settle starts once a frame has shown the messages.
                let Some(app) = app.upgrade() else {
                    return;
                };
                if app.ui.list.store.n_items() > 0 && bench.borrow_mut().ready() {
                    app.bench_drive(&bench);
                }
            });
        });
    }

    fn bench_drive(self: &Rc<Self>, bench: &Shared) {
        if !bench.borrow().wants_frames() {
            self.bench_check(bench);
            return;
        }
        // A tick callback keeps the frame clock running every refresh.
        let app = Rc::downgrade(self);
        let bench = bench.clone();
        self.ui.window.add_tick_callback(move |_, _| {
            let action = bench.borrow_mut().step();
            match app.upgrade() {
                Some(app) if app.bench_act(action) => glib::ControlFlow::Continue,
                _ => glib::ControlFlow::Break,
            }
        });
    }

    /// Idle draws nothing, so its settle and end come from timers.
    fn bench_check(self: &Rc<Self>, bench: &Shared) {
        let Some(after) = bench.borrow().next_check() else {
            return;
        };
        let app = Rc::downgrade(self);
        let bench = bench.clone();
        glib::timeout_add_local_once(after, move || {
            let action = bench.borrow_mut().step();
            if let Some(app) = app.upgrade()
                && app.bench_act(action)
            {
                app.bench_check(&bench);
            }
        });
    }

    /// Does what the script asks; false once it is over.
    fn bench_act(&self, action: Action) -> bool {
        match action {
            Action::ScrollUp(points) => {
                let adjustment = self.ui.list.scroller.vadjustment();
                adjustment
                    .set_value((adjustment.value() - f64::from(points)).max(adjustment.lower()));
            }
            Action::Type(character) => {
                let view = &self.ui.composer.view;
                if !view.has_focus() {
                    view.grab_focus();
                }
                view.buffer()
                    .insert_at_cursor(character.encode_utf8(&mut [0; 4]));
            }
            Action::Quit => {
                if let Some(application) = self.ui.window.application() {
                    application.quit();
                }
                return false;
            }
            Action::None => {}
        }
        true
    }
}
