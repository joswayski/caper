//! Caper media worker: compresses uploaded attachments with the same rules
//! for every client (see "Uploads and attachments" in `docs/media.md`).

pub mod api;
pub mod av;
pub mod detect;
pub mod event;
pub mod file;
pub mod filename;
pub mod image;
pub mod local;
pub mod process;
pub mod settings;
pub mod storage;
pub mod tools;
pub mod video;
pub mod worker;
