//! The background worker (`caper_client::worker`), hosted by the egui window.

pub use caper_client::worker::*;

pub type Worker = caper_client::worker::Worker<crate::host::Repaint>;
