//! Custom profile pictures and space icons (`avatarAsset`/`avatarUrl` on
//! people, `iconAsset`/`iconUrl` on spaces; the shared pictures contract in
//! docs/media.md): what identifies a picture, what an upload changes, and how
//! the API's picture errors read. Preparing, uploading and showing pictures
//! is the UI's.

use crate::uploads::UploadError;

pub const INVALID: &str = "That picture couldn't be used. Try another one.";
pub const UNAVAILABLE: &str = "Picture uploads aren’t available right now.";

/// What a person or space shows instead of the mascot or initial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source<'a> {
    /// The stable cache key; changes whenever the picture changes.
    pub asset: &'a str,
    /// Absent when the server signs no CDN URLs: only a cached picture shows.
    pub url: Option<&'a str>,
}

impl<'a> Source<'a> {
    /// `None` without an asset: no custom picture.
    pub fn new(asset: &'a Option<String>, url: &'a Option<String>) -> Option<Self> {
        asset
            .as_deref()
            .filter(|asset| !asset.is_empty())
            .map(|asset| Self {
                asset,
                url: url.as_deref().filter(|url| !url.is_empty()),
            })
    }
}

/// Which picture an upload or removal changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Avatar,
    /// A space's icon, by space id.
    SpaceIcon(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
    pub width: u32,
    pub height: u32,
}

impl Prepared {
    /// `avatar.<ext>` or `space-icon.<ext>`.
    pub fn filename(&self, target: &Target) -> String {
        let stem = match target {
            Target::Avatar => "avatar",
            Target::SpaceIcon(_) => "space-icon",
        };
        let extension = match self.content_type {
            "image/gif" => "gif",
            "image/webp" => "webp",
            "image/png" => "png",
            _ => "jpg",
        };
        format!("{stem}.{extension}")
    }
}

/// Why a picture change failed, worded for the settings row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    /// The server has no picture storage (`503`): the controls step back.
    pub unavailable: bool,
}

impl Failure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            unavailable: false,
        }
    }
}

/// The API's picture errors as sentences (contract "Uploading a picture").
pub fn failure(status: Option<u16>, code: Option<&str>, message: &str, target: &Target) -> Failure {
    let text = match (code, status) {
        (Some("picture_type"), _) => "Choose a PNG, JPEG, WebP or GIF picture.",
        (Some("picture_too_large"), _) | (_, Some(413)) => {
            "That picture is too large. Try another one."
        }
        (Some("picture_rate_limited"), _) | (_, Some(429)) => {
            "You’ve changed pictures too often today. Try again later."
        }
        (Some("picture_invalid"), _) | (_, Some(422)) => INVALID,
        (_, Some(400)) => "Pictures can be up to 1024 × 1024.",
        (_, Some(404)) => match target {
            Target::Avatar => "That’s no longer available.",
            Target::SpaceIcon(_) => "Only the space owner can change its icon.",
        },
        (_, Some(409)) => "The upload didn’t finish. Try again.",
        (_, Some(503)) => UNAVAILABLE,
        _ => message,
    };
    Failure {
        message: text.into(),
        unavailable: status == Some(503),
    }
}

pub fn upload_failure(error: &UploadError, target: &Target) -> Failure {
    failure(error.status, error.code.as_deref(), &error.message, target)
}

pub fn api_failure(error: &crate::api::ApiError, target: &Target) -> Failure {
    failure(
        error.status.map(|status| status.as_u16()),
        error.code.as_deref(),
        &error.message,
        target,
    )
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviousPicture {
    pub avatar_asset: String,
    pub avatar_url: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub pictures: Vec<PreviousPicture>,
    pub next_cursor: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_errors_map_to_sentences() {
        let space = Target::SpaceIcon("s".into());
        let message = |status, code| failure(status, code, "server text", &space).message;
        assert_eq!(
            message(Some(400), Some("picture_type")),
            "Choose a PNG, JPEG, WebP or GIF picture."
        );
        assert_eq!(
            message(Some(413), Some("picture_too_large")),
            "That picture is too large. Try another one."
        );
        assert_eq!(message(Some(422), Some("picture_invalid")), INVALID);
        assert_eq!(
            message(Some(429), Some("picture_rate_limited")),
            "You’ve changed pictures too often today. Try again later."
        );
        assert_eq!(
            message(Some(404), None),
            "Only the space owner can change its icon."
        );
        assert_eq!(message(Some(500), None), "server text");
        let unavailable = failure(Some(503), None, "x", &Target::Avatar);
        assert!(unavailable.unavailable);
        assert_eq!(unavailable.message, UNAVAILABLE);
        assert!(!failure(Some(422), None, "x", &Target::Avatar).unavailable);
    }
}
