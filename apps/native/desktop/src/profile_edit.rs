//! Private account edits. Preparing a picture never contacts the server.
use crate::{api::Api, model::Account, pictures, uploads};
use eframe::egui;
use std::sync::{Arc, Mutex, mpsc};

type Preparation = Result<(pictures::Prepared, pictures::Frames), String>;

pub struct Draft {
    pub baseline: Account,
    pub preview: Account,
    pub picture: Option<Arc<pictures::Prepared>>,
    pub uploaded: Arc<Mutex<Option<String>>>,
    pub preparing: Option<mpsc::Receiver<Preparation>>,
    pub confirm_discard: bool,
}

impl Draft {
    pub fn new(account: &Account) -> Self {
        Self {
            baseline: account.clone(),
            preview: account.clone(),
            picture: None,
            uploaded: Arc::default(),
            preparing: None,
            confirm_discard: false,
        }
    }

    pub fn avatar_dirty(&self) -> bool {
        self.picture.is_some()
            || self.preview.avatar_id != self.baseline.avatar_id
            || self.preview.avatar_asset != self.baseline.avatar_asset
    }

    pub fn dirty(&self, username: &str, name: &str) -> bool {
        self.preparing.is_some()
            || self.avatar_dirty()
            || username.trim() != self.baseline.username.as_deref().unwrap_or_default()
            || name.trim() != self.baseline.display_name.as_deref().unwrap_or_default()
    }

    pub fn select(&mut self, asset: Option<String>, url: Option<String>) {
        self.preview.avatar_asset = asset;
        self.preview.avatar_url = url;
        self.picture = None;
        self.uploaded = Arc::default();
    }

    pub fn regenerate(&mut self) {
        // UUID entropy is local, and a nonzero offset guarantees a new mascot.
        let offset = (uuid::Uuid::new_v4().as_u128() % 799) as i32 + 1;
        self.preview.avatar_id = Some((self.preview.avatar_id.unwrap_or(0) + offset) % 800);
        self.select(None, None);
    }

    pub fn prepare(
        &mut self,
        context: &egui::Context,
        path: std::path::PathBuf,
        edit: Option<crate::attachment_edit::Edit>,
        scratch: Option<Arc<uploads::Scratch>>,
    ) {
        let (sender, receiver) = mpsc::channel();
        let context = context.clone();
        let animate = !crate::motion::reduced();
        std::thread::spawn(move || {
            use std::io::Read;
            let _keep = scratch;
            let result = (|| {
                let file =
                    std::fs::File::open(path).map_err(|_| pictures::UNREADABLE.to_string())?;
                let mut bytes = Vec::new();
                file.take(pictures::MAX_SOURCE_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| pictures::UNREADABLE.to_string())?;
                if bytes.len() as u64 > pictures::MAX_SOURCE_BYTES {
                    return Err("That file is too large to use as a picture.".into());
                }
                let picture = pictures::prepare_edited(bytes, edit.as_ref())?;
                let frames = pictures::decode(&picture.bytes, animate)
                    .ok_or_else(|| pictures::UNREADABLE.to_string())?;
                Ok((picture, frames))
            })();
            let _ = sender.send(result);
            context.request_repaint();
        });
        self.preparing = Some(receiver);
    }

    pub fn poll(&mut self) -> Option<Result<(String, pictures::Frames), String>> {
        let result = match self.preparing.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(_) => Err(pictures::UNREADABLE.into()),
        };
        self.preparing = None;
        Some(result.map(|(picture, frames)| {
            let key = format!("profile-draft-{}", uuid::Uuid::new_v4());
            self.select(Some(key.clone()), None);
            self.picture = Some(Arc::new(picture));
            (key, frames)
        }))
    }

    pub fn save(&self) -> Option<AvatarSave> {
        self.avatar_dirty().then(|| AvatarSave {
            avatar_id: self.preview.avatar_id.unwrap_or(0),
            asset: self.preview.avatar_asset.clone(),
            expected_id: self.baseline.avatar_id,
            expected_asset: self.baseline.avatar_asset.clone(),
            picture: self.picture.clone(),
            uploaded: self.uploaded.clone(),
        })
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_are_private_and_removal_can_return_to_clean() {
        let account: Account = serde_json::from_value(serde_json::json!({
            "id":"self", "username":"alice", "displayName":"Alice", "avatarId":799
        }))
        .unwrap();
        let mut draft = Draft::new(&account);
        assert!(!draft.dirty("alice", "Alice"));
        assert!(draft.save().is_none());
        draft.select(
            Some("old-photo".into()),
            Some("https://example.test/photo".into()),
        );
        assert!(draft.avatar_dirty());
        assert!(account.avatar_asset.is_none());
        draft.select(None, None);
        assert!(!draft.avatar_dirty());
        assert!(draft.dirty("alice", "New name"));
        for _ in 0..1000 {
            let previous = draft.preview.avatar_id;
            draft.regenerate();
            assert_ne!(draft.preview.avatar_id, previous);
            assert!((0..800).contains(&draft.preview.avatar_id.unwrap()));
        }
        assert_eq!(account.avatar_id, Some(799));
        assert_eq!(draft.baseline.avatar_id, Some(799));
    }
}
