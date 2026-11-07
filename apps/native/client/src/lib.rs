//! Caper's UI-free client core, shared by the desktop clients: the HTTP API,
//! the WebSocket gateway, the data model, uploads and compression, voice
//! media, notifications, invites, app links and credentials.

pub mod activity;
pub mod api;
pub mod app_badge;
pub mod app_link;
pub mod attachment_edit;
pub mod attachments;
pub mod avatar_images;
pub mod avif;
pub mod blocking;
pub mod compress;
pub mod credentials;
pub mod daily_icon;
pub mod drafts;
pub mod edits;
pub mod effects;
pub mod embeds;
pub mod emoji;
pub mod feedback;
pub mod ffmpeg;
pub mod formatting;
pub mod forwarding;
pub mod gateway;
pub mod grouping;
pub mod host;
pub mod invites;
pub mod klipy;
/// Link segments in message text, shared by every client (`shared/messages`).
#[path = "../../../../shared/messages/links.rs"]
pub mod links;
pub mod media;
#[path = "../../desktop/voice-spike/src/media_gateway.rs"]
pub mod media_gateway;
pub mod media_proxy;
pub mod mentions;
pub mod metadata;
pub mod model;
pub mod names;
pub mod navigation;
pub mod notifications;
pub mod os_notifications;
pub mod pictures;
pub mod player;
pub mod profile_dm;
pub mod profile_edit;
pub mod reactions;
pub mod sign_in;
pub mod startup;
#[path = "../../desktop/voice-spike/src/state.rs"]
pub mod state;
pub mod updates;
pub mod uploads;
pub mod voice;
pub mod voice_note;
// Voice media, shared with the voice spike (where it is `crate::media`), whose
// controls are built only through `new`.
#[allow(clippy::new_without_default)]
#[path = "../../desktop/voice-spike/src/media.rs"]
pub mod voice_media;
pub mod whats_new;
pub mod worker;

pub use host::{Host, Rgba};

pub fn caper_avatar_index(avatar_id: Option<i32>) -> Option<u16> {
    avatar_id.and_then(|index| u16::try_from(index).ok().filter(|index| *index < 800))
}

/// A space's tile without an icon: its first character, uppercased, as on
/// web, Apple and Android ("C" for a blank name).
pub fn space_initial(name: &str) -> String {
    name.trim()
        .chars()
        .next()
        .unwrap_or('C')
        .to_uppercase()
        .to_string()
}

#[cfg(test)]
mod space_initial_tests {
    #[test]
    fn first_character_uppercased() {
        assert_eq!(super::space_initial("  fixture Studio"), "F");
        assert_eq!(super::space_initial("ßeta"), "SS");
        assert_eq!(super::space_initial("🎲 Games"), "🎲");
        assert_eq!(super::space_initial("   "), "C");
    }
}

#[cfg(test)]
mod avatar_tests {
    use super::caper_avatar_index;
    #[test]
    fn persisted_indices_and_fallback() {
        for index in [0, 31, 32, 255, 256, 799] {
            assert_eq!(caper_avatar_index(Some(index)), Some(index as u16));
        }
        assert_eq!(caper_avatar_index(None), None);
        assert_eq!(caper_avatar_index(Some(-1)), None);
        assert_eq!(caper_avatar_index(Some(800)), None);
    }
}
