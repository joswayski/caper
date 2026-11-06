//! The GTK window as the background worker's host.
//!
//! The worker calls from its own threads, so every request crosses to the GTK
//! main loop through a channel that `App` drains there.

use caper_client::Host;
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

#[derive(Clone)]
pub struct Wakes(pub async_channel::Sender<Signal>);

impl Host for Wakes {
    fn wake(&self) {
        let _ = self.0.try_send(Signal::Wake);
    }

    fn wake_after(&self, delay: Duration) {
        let sender = self.0.clone();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            let _ = sender.try_send(Signal::Wake);
        });
    }

    fn bring_to_front(&self) {
        let _ = self.0.try_send(Signal::Front);
    }

    fn set_title(&self, title: String) {
        let _ = self.0.try_send(Signal::Title(title));
    }
}
