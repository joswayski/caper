//! The GTK window as the background worker's host.
//!
//! The worker calls from its own threads, so every request crosses to the GTK
//! main loop through a channel that `App` drains there.

use caper_client::Host;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub enum Signal {
    /// Drain the worker's events.
    Wake,
    Title(String),
    Front,
    /// An uploaded picture finished loading: its frames, one for a still
    /// (`None` on failure).
    Picture {
        asset: String,
        url: String,
        image: Option<caper_client::pictures::Frames>,
        /// What an animation's frames were kept at (`Images::loaded`).
        edge: u32,
    },
    /// Message media wants fresh signed URLs.
    RefreshMedia,
    /// Message media finished loading (`crate::media`).
    Media {
        generation: u64,
        key: String,
        url: String,
        result: Result<Vec<(caper_client::Rgba, std::time::Duration)>, Option<u16>>,
    },
}

/// Every clone shares one pending wake: a burst of worker events queues one
/// `Signal::Wake`, so the app drains and renders once, not once per event.
#[derive(Clone)]
pub struct Wakes {
    pub sender: async_channel::Sender<Signal>,
    pending: Arc<AtomicBool>,
}

impl Wakes {
    pub fn new(sender: async_channel::Sender<Signal>) -> Self {
        Self {
            sender,
            pending: Arc::default(),
        }
    }

    /// The main loop is about to drain: anything sent from now on wakes it
    /// again.
    pub fn draining(&self) {
        self.pending.store(false, Ordering::SeqCst);
    }
}

impl Host for Wakes {
    fn wake(&self) {
        if !self.pending.swap(true, Ordering::SeqCst) {
            let _ = self.sender.try_send(Signal::Wake);
        }
    }

    /// A main-loop timer, not a sleeping thread per request: a call asks
    /// for a wake with every voice report.
    fn wake_after(&self, delay: Duration) {
        if delay.is_zero() {
            return self.wake();
        }
        let wakes = self.clone();
        gtk::glib::timeout_add_once(delay, move || wakes.wake());
    }

    fn bring_to_front(&self) {
        let _ = self.sender.try_send(Signal::Front);
    }

    fn set_title(&self, title: String) {
        let _ = self.sender.try_send(Signal::Title(title));
    }
}
