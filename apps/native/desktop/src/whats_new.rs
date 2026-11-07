//! What's new in this client revision (`caper_client::whats_new`), with the
//! acknowledgment saved in eframe storage.

pub use caper_client::whats_new::{PLATFORM, REVISION, Snapshot, WhatsNew};

pub fn restore(state: &mut WhatsNew, storage: &dyn eframe::Storage) {
    let stored = storage.get_string(state.key());
    state.restore(stored.as_deref());
}

pub fn save(state: &WhatsNew, storage: &mut dyn eframe::Storage) {
    if let Some(value) = state.stored() {
        storage.set_string(state.key(), value);
    }
}
