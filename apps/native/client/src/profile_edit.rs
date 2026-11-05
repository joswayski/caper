//! Saving a private account edit's picture.
use crate::{api::Api, pictures, uploads};
use std::sync::{Arc, Mutex};

pub struct AvatarSave {
    pub avatar_id: i32,
    pub asset: Option<String>,
    pub expected_id: Option<i32>,
    pub expected_asset: Option<String>,
    pub picture: Option<Arc<pictures::Prepared>>,
    pub uploaded: Arc<Mutex<Option<String>>>,
}

impl AvatarSave {
    pub fn payload(&self, api: &Api, token: &str) -> Result<serde_json::Value, String> {
        let asset = if let Some(picture) = &self.picture {
            let mut cached = self.uploaded.lock().unwrap();
            if cached.is_none() {
                let (id, _) = uploads::upload_draft_picture(
                    api,
                    token,
                    pictures::reservation(&pictures::Target::Avatar, picture),
                    picture.bytes.clone(),
                )
                .map_err(|error| {
                    pictures::upload_failure(&error, &pictures::Target::Avatar).message
                })?;
                *cached = Some(id);
            }
            cached.clone()
        } else {
            self.asset.clone()
        };
        Ok(serde_json::json!({
            "avatarId": self.avatar_id,
            "avatarAsset": asset,
            "expectedAvatarId": self.expected_id,
            "expectedAvatarAsset": self.expected_asset,
        }))
    }
}
