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
    use serde_json::{Value, json};
    use std::io::{BufRead, BufReader, Read, Write};

    struct Request {
        line: String,
        body: Vec<u8>,
    }

    /// A loopback HTTP/1.1 server answering requests in order and recording
    /// them. Responses are built from the server's own base URL, so presigned
    /// storage URLs can point back at it like a local R2 fake.
    fn serve(
        responses: impl FnOnce(&str) -> Vec<(u16, String)>,
    ) -> (String, std::thread::JoinHandle<Vec<Request>>) {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.local_addr().unwrap());
        let responses = responses(&base);
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (stream, _) = server.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header == "\r\n" {
                        break;
                    }
                    let (name, value) = header.trim_end().split_once(':').unwrap();
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut content = vec![0; length];
                reader.read_exact(&mut content).unwrap();
                write!(
                    reader.get_mut(),
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                requests.push(Request {
                    line: line.trim_end().to_owned(),
                    body: content,
                });
            }
            requests
        });
        (base, handle)
    }

    fn reserved(base: &str) -> String {
        json!({
            "id": "asset0001",
            "kind": "file",
            "upload": {"method": "PUT", "url": format!("{base}/r2/original/asset0001?X-Amz-Signature=s"), "headers": {
                "content-type": "application/octet-stream"
            }},
            "storage": {"used": 23, "limit": 1000}
        })
        .to_string()
    }

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

    #[test]
    fn profile_picture_draft_completes_privately_and_reuses_upload_on_retry() {
        let original =
            json!({"id":"self", "username":"alice", "displayName":"Alice", "avatarId":17});
        let saved = json!({"id":"self", "username":"alice", "displayName":"New name", "avatarId":17, "avatarAsset":"asset0001"});
        let (base, server) = serve(|base| {
            vec![
                (201, reserved(base)),
                (200, String::new()),
                (
                    200,
                    json!({"id":"asset0001", "account":original}).to_string(),
                ),
                (409, r#"{"error":"taken","code":"username_taken"}"#.into()),
                (200, saved.to_string()),
                (200, saved.to_string()),
            ]
        });
        let api = Api::new(&base).unwrap();
        let account = serde_json::from_value(original).unwrap();
        let mut draft = Draft::new(&account);
        draft.picture = Some(Arc::new(pictures::Prepared {
            bytes: b"prepared image bytes".to_vec(),
            content_type: "image/webp",
            width: 8,
            height: 8,
        }));
        let avatar = draft.save().unwrap();
        let payload = avatar.payload(&api, "token").unwrap();
        assert_eq!(
            payload,
            json!({"avatarId":17,"avatarAsset":"asset0001","expectedAvatarId":17,"expectedAvatarAsset":null})
        );
        assert!(
            api.update_profile("token", "alice", "New name", Some(payload.clone()))
                .is_err()
        );
        assert_eq!(draft.uploaded.lock().unwrap().as_deref(), Some("asset0001"));
        assert!(draft.picture.is_some());
        assert!(account.avatar_asset.is_none());
        let retry = draft.save().unwrap().payload(&api, "token").unwrap();
        assert_eq!(retry, payload);
        let result = api
            .update_profile("token", "alice", "New name", Some(retry))
            .unwrap();
        assert_eq!(result.avatar_asset.as_deref(), Some("asset0001"));
        api.update_profile("token", "alice", "Name only", None)
            .unwrap();
        let requests = server.join().unwrap();
        assert_eq!(
            requests
                .iter()
                .map(|request| request.line.as_str())
                .collect::<Vec<_>>(),
            vec![
                "POST /api/assets HTTP/1.1",
                "PUT /r2/original/asset0001?X-Amz-Signature=s HTTP/1.1",
                "POST /api/assets/asset0001/complete?draft=true HTTP/1.1",
                "POST /api/account/profile HTTP/1.1",
                "POST /api/account/profile HTTP/1.1",
                "POST /api/account/profile HTTP/1.1",
            ]
        );
        let body: Value = serde_json::from_slice(&requests[4].body).unwrap();
        assert_eq!(
            body,
            json!({"username":"alice","displayName":"New name","avatar":payload})
        );
        let body: Value = serde_json::from_slice(&requests[5].body).unwrap();
        assert_eq!(body, json!({"username":"alice","displayName":"Name only"}));
    }
}
