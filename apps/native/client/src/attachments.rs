//! Message attachments: refreshed signed URLs, and the edge locally shown
//! images are decoded at.

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FreshUrl {
    /// Absent while the file is still processing.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub preview_url: Option<String>,
}

/// Locally shown images fit the largest inline frame at 2x.
pub const THUMBNAIL_EDGE: u32 = 720;
