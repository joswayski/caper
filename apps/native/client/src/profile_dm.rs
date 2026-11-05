//! Sending to a person's DM, or replying from the inbox, with files. Files
//! upload eagerly; only explicit Send writes a message.
use crate::{Host, api::Api, compress::Compression, model, uploads};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, atomic::AtomicBool};

type Uploaded = Result<(String, model::Attachment, std::time::Instant), String>;

/// A file's eager upload: cancelled when the last copy of the file is dropped.
pub struct Upload {
    pub cancel: Arc<AtomicBool>,
    pub result: Arc<OnceLock<Uploaded>>,
}

impl Drop for Upload {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// A file to send: what to upload and what came of it. The UI keeps any
/// preview beside it.
#[derive(Clone)]
pub struct File {
    pub path: PathBuf,
    /// Pasted images belong to the draft, including while a worker reads them.
    pub scratch: Option<Arc<uploads::Scratch>>,
    pub attachment: Option<model::Attachment>,
    uploaded_at: Option<std::time::Instant>,
    pub upload: Option<Arc<Upload>>,
    pub spoiler: bool,
    /// Bytes on disk when staged, for the composer chip.
    pub size: u64,
    /// Whether the file can be cropped, drawn on and so on before sending.
    pub editable: Option<crate::attachment_edit::Kind>,
    /// Applied to the original before preparing and uploading.
    pub edit: Option<crate::attachment_edit::Edit>,
    /// A recorded voice message (`voice_note`), uploaded exactly as prepared.
    pub voice: Option<Arc<uploads::Prepared>>,
}

impl File {
    pub fn picked(path: PathBuf) -> Self {
        Self::staged(path, None)
    }

    /// A voice message sent on its own from the profile card: never staged
    /// in the draft, compressed or edited.
    pub fn voice(prepared: Arc<uploads::Prepared>) -> Self {
        Self {
            size: prepared.size,
            voice: Some(prepared),
            ..Self::staged(PathBuf::new(), None)
        }
    }

    pub fn staged(path: PathBuf, scratch: Option<Arc<uploads::Scratch>>) -> Self {
        Self {
            size: std::fs::metadata(&path).map_or(0, |metadata| metadata.len()),
            editable: crate::attachment_edit::editable(&path),
            path,
            scratch,
            attachment: None,
            uploaded_at: None,
            upload: None,
            spoiler: false,
            edit: None,
            voice: None,
        }
    }

    /// A new edit (`None` returns to the original): any upload of the
    /// previous version no longer counts.
    pub fn set_edit(&mut self, edit: Option<crate::attachment_edit::Edit>) {
        self.upload = None;
        self.attachment = None;
        self.edit = edit;
    }

    pub fn name(&self) -> String {
        if let Some(voice) = &self.voice {
            voice.name.clone()
        } else if self.scratch.is_some() {
            "Pasted image.png".into()
        } else {
            self.path
                .file_name()
                .map_or_else(|| "file".into(), |name| name.to_string_lossy().into_owned())
        }
    }

    /// The server's asset for this file, uploaded eagerly or on a send, so
    /// removing a staged file can cancel it.
    pub fn uploaded_id(&self) -> Option<String> {
        self.attachment
            .as_ref()
            .map(|file| file.id.clone())
            .or_else(
                || match self.upload.as_ref().and_then(|upload| upload.result.get()) {
                    Some(Ok((_, attachment, _))) => Some(attachment.id.clone()),
                    _ => None,
                },
            )
    }

    pub fn upload_status(&self) -> (&str, bool) {
        match self.upload.as_ref().and_then(|upload| upload.result.get()) {
            Some(Err(error)) => (error, true),
            Some(Ok(_)) => ("", false),
            None if self.upload.is_some() => ("Uploading…", false),
            None => ("", false),
        }
    }

    /// Uploads the file to the person's DM (or the inbox row's channel)
    /// without sending; an explicit Send reuses it. The file owns its upload:
    /// removing or editing it invalidates the result. `host` wakes as the
    /// upload starts and when it ends.
    pub fn upload_eagerly<H: Host>(
        &mut self,
        api: &Api,
        token: &str,
        (person, username, target): (&str, &str, Option<crate::activity::Item>),
        settings: &Compression,
        host: &H,
    ) {
        let cancel = Arc::new(AtomicBool::new(false));
        let result = Arc::new(OnceLock::new());
        let mut send = Send {
            person: person.into(),
            username: username.into(),
            name: String::new(),
            client_id: String::new(),
            text: String::new(),
            files: vec![self.clone()],
            settings: Some(settings.clone()),
            cancel: cancel.clone(),
            retrying: false,
            open_only: false,
            voice_only: false,
            direct: None,
        };
        self.upload = Some(Arc::new(Upload {
            cancel,
            result: result.clone(),
        }));
        host.wake();
        let (api, token, host) = (api.clone(), token.to_owned(), host.clone());
        std::thread::spawn(move || {
            let destination = if let Some(item) = &target {
                send.run_activity(&api, &token, &|_| {}, item, None, true)
                    .map(|()| item.channel().unwrap().to_owned())
            } else {
                send.run_inner(&api, &token, &|_| {}, true)
                    .map(|direct| direct.id)
            };
            let uploaded = destination
                .map(|channel| {
                    (
                        channel,
                        send.files[0].attachment.clone().expect("uploaded file"),
                        std::time::Instant::now(),
                    )
                })
                .map_err(|error| error.message);
            let _ = result.set(uploaded);
            host.wake();
        });
    }
}

pub struct Send {
    pub person: String,
    pub username: String,
    pub name: String,
    pub client_id: String,
    pub text: String,
    pub files: Vec<File>,
    pub settings: Option<Compression>,
    pub cancel: Arc<AtomicBool>,
    pub retrying: bool,
    /// Explicit navigation resolves the DM but never uploads or writes a message.
    pub open_only: bool,
    /// A voice message sent alone as soon as recording stopped (the "send
    /// at once" setting): it keeps the card's draft instead of clearing it.
    pub voice_only: bool,
    pub direct: Option<model::DirectConversation>,
}

#[derive(Debug)]
pub struct Failure {
    pub message: String,
    pub uncertain: bool,
}

impl Send {
    /// Rechecks destination safety on Send, waits for eager uploads, and
    /// reuses uploaded IDs after a failed write.
    pub fn run(
        &mut self,
        api: &Api,
        token: &str,
        status: &dyn Fn(String),
    ) -> Result<model::DirectConversation, Failure> {
        self.run_inner(api, token, status, false)
    }

    fn run_inner(
        &mut self,
        api: &Api,
        token: &str,
        status: &dyn Fn(String),
        upload_only: bool,
    ) -> Result<model::DirectConversation, Failure> {
        let cancelled = || {
            if self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                Err(Failure {
                    message: "Send cancelled.".into(),
                    uncertain: self.retrying,
                })
            } else {
                Ok(())
            }
        };
        cancelled()?;
        // No pre-POST failure can disprove an earlier uncertain write.
        let retrying = self.retrying;
        let lookup_failure = |error: crate::api::ApiError| Failure {
            uncertain: retrying,
            message: error.to_string(),
        };
        self.direct = api
            .direct_conversations(token)
            .map_err(lookup_failure)?
            .conversations
            .into_iter()
            .find(|direct| direct.peer.id == self.person);
        cancelled()?;
        let direct = match self.direct.clone() {
            Some(direct) => direct,
            None => api
                .create_direct(token, &self.username)
                .map_err(lookup_failure)?,
        };
        if direct.peer.id != self.person {
            return Err(Failure {
                message: "This person's username changed. Reopen their profile before sending."
                    .into(),
                uncertain: retrying,
            });
        }
        self.direct = Some(direct.clone());
        if self.open_only {
            return Ok(direct);
        }
        if direct.status == model::DirectStatus::Incoming {
            return Err(Failure {
                message: "Open conversation and accept this request before sending.".into(),
                uncertain: retrying,
            });
        }
        if direct.blocked {
            return Err(Failure {
                message: "Unblock this person to message them.".into(),
                uncertain: retrying,
            });
        }
        self.deliver(api, token, status, (&direct.id, None, None), upload_only)?;
        Ok(direct)
    }

    pub fn run_activity(
        &mut self,
        api: &Api,
        token: &str,
        status: &dyn Fn(String),
        item: &crate::activity::Item,
        gif: Option<&crate::klipy::Gif>,
        upload_only: bool,
    ) -> Result<(), Failure> {
        if self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Failure {
                message: "Send cancelled.".into(),
                uncertain: self.retrying,
            });
        }
        let channel = item.destination(api, token).map_err(|message| Failure {
            message,
            uncertain: self.retrying,
        })?;
        self.deliver(api, token, status, (&channel, Some(item), gif), upload_only)
    }

    fn deliver(
        &mut self,
        api: &Api,
        token: &str,
        status: &dyn Fn(String),
        (channel, item, gif): (
            &str,
            Option<&crate::activity::Item>,
            Option<&crate::klipy::Gif>,
        ),
        upload_only: bool,
    ) -> Result<(), Failure> {
        let retrying = self.retrying;
        let cancelled = || {
            if self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                Err(Failure {
                    message: "Send cancelled.".into(),
                    uncertain: retrying,
                })
            } else {
                Ok(())
            }
        };
        let lookup_failure = |error: crate::api::ApiError| Failure {
            uncertain: retrying,
            message: error.to_string(),
        };
        cancelled()?;
        let session = if upload_only {
            None
        } else {
            Some(
                api.chat_session(Some(token), &self.name)
                    .map_err(lookup_failure)?,
            )
        };
        for (index, file) in self.files.iter_mut().enumerate() {
            cancelled()?;
            if let Some(upload) = file.upload.take() {
                while upload.result.get().is_none() {
                    cancelled()?;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                if let Some(Ok((destination, attachment, completed))) = upload.result.get()
                    && destination == channel
                {
                    file.attachment = Some(attachment.clone());
                    file.uploaded_at = Some(*completed);
                }
                // A failed eager attempt is retried by explicit Send, never every frame.
            }
            if !retrying
                && file
                    .uploaded_at
                    .is_some_and(|at| at.elapsed().as_secs() >= 23 * 60 * 60)
            {
                if let Some(attachment) = file.attachment.take() {
                    api.cancel_asset(token, &attachment.id);
                }
                file.uploaded_at = None;
            }
            if file.attachment.is_some() {
                continue;
            }
            let settings = self.settings.as_ref().ok_or_else(|| Failure {
                message: "File uploads are not ready. Try again shortly.".into(),
                uncertain: retrying,
            })?;
            status(if file.voice.is_some() {
                "Uploading voice message…".into()
            } else {
                format!("Preparing and uploading {} ({})…", file.name(), index + 1)
            });
            let prepared = match (&file.voice, &file.edit) {
                (Some(voice), _) => Ok(voice.as_ref().clone()),
                (None, Some(edit)) => uploads::prepare_edited_path(
                    &file.path,
                    &file.name(),
                    edit,
                    settings,
                    &|_| {},
                    &self.cancel,
                ),
                (None, None) => {
                    uploads::prepare_path_with(&file.path, settings, &|_| {}, &self.cancel)
                }
            };
            let mut prepared = prepared.map_err(|message| Failure {
                message,
                uncertain: retrying,
            })?;
            if file.scratch.is_some() {
                prepared.name =
                    crate::compress::renamed("Pasted image.png", &prepared.content_type);
            }
            file.attachment = Some(
                uploads::upload(
                    api,
                    token,
                    channel,
                    prepared,
                    Arc::new(|_| {}),
                    self.cancel.clone(),
                )
                .map_err(|error| Failure {
                    message: error.message,
                    uncertain: retrying,
                })?,
            );
            file.uploaded_at = Some(std::time::Instant::now());
        }
        cancelled()?;
        if upload_only {
            return Ok(());
        }
        let session = session.expect("explicit send session");
        status("Sending…".into());
        let ids = self
            .files
            .iter()
            .filter_map(|file| file.attachment.as_ref().map(|a| a.id.clone()))
            .collect::<Vec<_>>();
        let spoiler_ids = self
            .files
            .iter()
            .filter(|file| file.spoiler)
            .filter_map(|file| file.attachment.as_ref().map(|a| a.id.clone()))
            .collect::<Vec<_>>();
        let message = api
            .send(
                Some(token),
                &session.token,
                channel,
                &self.client_id,
                &self.text,
                &ids,
                &spoiler_ids,
                (item.and_then(|item| item.thread_root_id.as_deref()), false),
                item.and_then(|item| item.quote_id()),
                gif,
                false,
            )
            .map_err(|error| Failure {
                // A definitive 4xx refuses the write. Transport/5xx responses
                // may have lost the acknowledgement after committing it.
                uncertain: error.status.is_none_or(|status| status.is_server_error()),
                message: error.to_string(),
            })?;
        if message.channel_id != channel
            || message.client_message_id != self.client_id
            || message.author.id != session.author.id
            || message.thread_root_id.as_deref()
                != item.and_then(|item| item.thread_root_id.as_deref())
            || message.reply_to.as_ref().map(|quote| quote.id.as_str())
                != item.and_then(|item| item.quote_id())
        {
            return Err(Failure {
                message: "Could not confirm message delivery. Retry sends the same message safely."
                    .into(),
                uncertain: true,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::io::{BufRead, BufReader, Read, Write};

    const DIRECT: &str = r#"{"id":"dm0000000002","peer":{"id":"alex-id","username":"alex","displayName":"Alex"},"lastSeq":"0","readSeq":"0","status":"outgoing"}"#;

    fn operation(files: Vec<File>) -> Send {
        Send {
            person: "alex-id".into(),
            username: "alex".into(),
            name: "Owner".into(),
            client_id: uuid::Uuid::new_v4().to_string(),
            text: "Receipt 🎉".into(),
            files,
            settings: Some(Compression::default()),
            cancel: Arc::new(AtomicBool::new(false)),
            retrying: false,
            open_only: false,
            voice_only: false,
            direct: None,
        }
    }

    struct Request {
        line: String,
        body: Vec<u8>,
    }

    /// A loopback API/storage server records the public HTTP contract.
    fn server(
        replies: impl FnOnce(&str) -> Vec<(u16, String)> + std::marker::Send + 'static,
    ) -> (Api, std::thread::JoinHandle<Vec<Request>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let api = Api::new(&base).unwrap();
        let thread = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for (status, reply) in replies(&base) {
                let (stream, _) = listener.accept().unwrap();
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
                    if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:")
                    {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                seen.push(Request {
                    line: line.trim_end().into(),
                    body,
                });
                write!(reader.get_mut(), "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
            }
            seen
        });
        (api, thread)
    }

    #[test]
    fn inbox_reply_targets_original_channel_and_retries_quote_or_thread_exactly() {
        for thread in [None, Some("Thread000000001")] {
            let item: crate::activity::Item = serde_json::from_value(json!({
                "id":"opaque:inbox", "kind":"mention", "actorName":"Other actor", "body":"Original",
                "createdAt":"2026-11-04T12:00:00Z", "unread":true, "channelId":"channel00001",
                "messageId":"Message00000001", "threadRootId":thread,
            }))
            .unwrap();
            let mut operation = operation(Vec::new());
            let client_id = operation.client_id.clone();
            let (api, requests) = server(move |_| {
                let mut sent = json!({"id":"Message00000002","channelId":"channel00001","seq":"2",
                    "createdAt":"2026-11-04T12:00:00Z","clientMessageId":client_id,
                    "author":{"id":"owner","name":"Owner","isGuest":false},
                    "content":{"version":1,"type":"text","text":"Receipt 🎉"}});
                if let Some(thread) = thread {
                    sent["threadRootId"] = json!(thread);
                } else {
                    sent["replyTo"] = json!({"id":"Message00000001","author":{"id":"other","name":"Other","isGuest":false},
                    "text":"Original","hasAttachments":false});
                }
                vec![
                    (200, json!({"token":"chat","author":{"id":"owner","name":"Owner","isGuest":false}}).to_string()),
                    (503, json!({"error":"Acknowledgement lost"}).to_string()),
                    (200, sent.to_string()),
                ]
            });
            assert!(
                operation
                    .run_activity(&api, "account", &|_| {}, &item, None, false)
                    .unwrap_err()
                    .uncertain
            );
            operation.retrying = true;
            operation
                .run_activity(&api, "account", &|_| {}, &item, None, false)
                .unwrap();
            let seen = requests.join().unwrap();
            assert_eq!(
                seen.len(),
                3,
                "no DM lookup/creation, navigation or read requests"
            );
            assert_eq!(
                seen[1].line,
                "POST /api/chat/channels/channel00001/messages HTTP/1.1"
            );
            assert_eq!(seen[1].body, seen[2].body);
            let body: Value = serde_json::from_slice(&seen[1].body).unwrap();
            assert_eq!(body["threadRootId"], json!(thread));
            assert_eq!(
                body["replyToId"],
                json!(thread.is_none().then_some("Message00000001"))
            );
        }
    }

    #[test]
    fn inbox_missing_dm_is_never_created_by_actor() {
        let item: crate::activity::Item = serde_json::from_value(json!({
            "id":"opaque:inbox", "kind":"reaction", "actorName":"Other", "body":"Original",
            "createdAt":"2026-11-04T12:00:00Z", "unread":true, "conversationId":"dm0000000002", "messageId":"Message00000001"
        })).unwrap();
        let (api, requests) = server(|_| vec![(200, json!({"conversations":[]}).to_string())]);
        assert!(
            operation(Vec::new())
                .run_activity(&api, "account", &|_| {}, &item, None, false)
                .is_err()
        );
        let seen = requests.join().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].line, "GET /api/dms HTTP/1.1");
    }

    #[test]
    fn profile_send_uploads_once_and_uncertain_retry_keeps_the_exact_payload() {
        let scratch = uploads::Scratch::new("txt");
        std::fs::write(scratch.path(), b"receipt").unwrap();
        let mut operation = operation(vec![File::picked(scratch.path().to_owned())]);
        operation.files[0].spoiler = true;
        let client_id = operation.client_id.clone();
        let (api, requests) = server(move |base| {
            vec![
            (200, json!({"conversations":[]}).to_string()),
            (201, DIRECT.into()),
            (200, json!({"token":"chat","author":{"id":"owner","name":"Owner","isGuest":false}}).to_string()),
            (201, json!({"id":"asset1","upload":{"method":"PUT","url":format!("{base}/storage/asset1"),"headers":{}}}).to_string()),
            (200, String::new()),
            (200, json!({"id":"asset1","kind":"file","contentType":"text/plain","name":"receipt.txt","size":7}).to_string()),
            (503, json!({"error":"Acknowledgement lost"}).to_string()),
            (503, json!({"error":"Temporary lookup failure"}).to_string()),
            (200, format!("{{\"conversations\":[{DIRECT}]}}")),
            (200, json!({"id":"m1","channelId":"dm0000000002","seq":"1","createdAt":"2026-10-11T00:00:00Z","clientMessageId":client_id,"author":{"id":"owner","name":"Owner","isGuest":false},"content":{"version":1,"type":"text","text":"Receipt 🎉"}}).to_string()),
        ]
        });
        let error = operation.run(&api, "account", &|_| {}).unwrap_err();
        assert!(error.uncertain);
        assert_eq!(operation.files[0].attachment.as_ref().unwrap().id, "asset1");
        operation.retrying = true;
        assert!(
            operation
                .run(&api, "account", &|_| {})
                .unwrap_err()
                .uncertain
        );
        assert_eq!(
            operation.run(&api, "account", &|_| {}).unwrap().id,
            "dm0000000002"
        );
        let seen = requests.join().unwrap();
        assert_eq!(seen[4].body, b"receipt");
        let reservation: Value = serde_json::from_slice(&seen[3].body).unwrap();
        assert_eq!(reservation["channelId"], "dm0000000002");
        assert_eq!(reservation["byteSize"], 7);
        assert_eq!(
            seen[6].body, seen[9].body,
            "retry must not change text, IDs or attachments"
        );
        let sent: Value = serde_json::from_slice(&seen[9].body).unwrap();
        assert_eq!(
            sent,
            json!({"clientMessageId":operation.client_id,"text":"Receipt 🎉","attachmentIds":["asset1"],"spoilerAttachmentIds":["asset1"]})
        );
        assert_eq!(
            seen.iter()
                .filter(|request| request.line == "POST /api/assets HTTP/1.1")
                .count(),
            1
        );
    }

    #[test]
    fn edited_files_upload_the_edit_and_unappliable_edits_send_nothing() {
        use crate::attachment_edit::{Crop, Edit};
        let picture = uploads::Scratch::new("png");
        image::RgbaImage::from_fn(64, 32, |x, _| {
            image::Rgba(if x < 32 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            })
        })
        .save(picture.path())
        .unwrap();
        let left = Edit {
            crop: Crop {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
            ..Edit::default()
        };
        let mut file = File::picked(picture.path().to_owned());
        assert_eq!(file.editable, Some(crate::attachment_edit::Kind::Image));
        file.set_edit(Some(left.clone()));
        let mut edited = operation(vec![file]);
        let client_id = edited.client_id.clone();
        let (api, requests) = server(move |base| {
            vec![
                (200, format!("{{\"conversations\":[{DIRECT}]}}")),
                (200, json!({"token":"chat","author":{"id":"owner","name":"Owner","isGuest":false}}).to_string()),
                (201, json!({"id":"asset1","upload":{"method":"PUT","url":format!("{base}/storage/asset1"),"headers":{}}}).to_string()),
                (200, String::new()),
                (200, json!({"id":"asset1","kind":"image","contentType":"image/png","name":"left.png","size":1}).to_string()),
                (200, json!({"id":"m1","channelId":"dm0000000002","seq":"1","createdAt":"2026-10-11T00:00:00Z","clientMessageId":client_id,"author":{"id":"owner","name":"Owner","isGuest":false},"content":{"version":1,"type":"text","text":"Receipt 🎉"}}).to_string()),
            ]
        });
        edited.run(&api, "account", &|_| {}).unwrap();
        let seen = requests.join().unwrap();
        let reservation: Value = serde_json::from_slice(&seen[2].body).unwrap();
        assert_eq!(
            (
                reservation["width"].as_u64(),
                reservation["height"].as_u64()
            ),
            (Some(32), Some(32))
        );
        let stored = image::load_from_memory(&seen[3].body).unwrap().to_rgba8();
        assert_eq!(stored.dimensions(), (32, 32));
        assert!(
            stored.pixels().all(|pixel| pixel.0 == [255, 0, 0, 255]),
            "only the kept half"
        );
        // An edit on something that cannot take it fails before any reservation.
        let notes = uploads::Scratch::new("txt");
        std::fs::write(notes.path(), b"notes").unwrap();
        let mut file = File::picked(notes.path().to_owned());
        assert_eq!(file.editable, None);
        file.set_edit(Some(left));
        let mut refused = operation(vec![file]);
        let (api, requests) = server(|_| {
            vec![
                (200, format!("{{\"conversations\":[{DIRECT}]}}")),
                (
                    200,
                    json!({"token":"chat","author":{"id":"owner","name":"Owner","isGuest":false}})
                        .to_string(),
                ),
            ]
        });
        let error = refused.run(&api, "account", &|_| {}).unwrap_err();
        assert_eq!(error.message, crate::attachment_edit::FAILED);
        assert_eq!(
            requests.join().unwrap().len(),
            2,
            "nothing is reserved or sent"
        );
    }

    #[test]
    fn incoming_and_blocked_directs_refuse_before_creation_upload_or_send() {
        for (status, blocked, expected) in [
            (
                "incoming",
                false,
                "Open conversation and accept this request before sending.",
            ),
            ("accepted", true, "Unblock this person to message them."),
        ] {
            for (retrying, upload_only) in [(false, false), (true, false), (false, true)] {
                let (api, requests) = server(move |_| {
                    vec![(200, json!({"conversations":[{
                "id":"dm0000000002","peer":{"id":"alex-id","username":"alex","displayName":"Alex"},"lastSeq":"0","readSeq":"0","status":status,"blocked":blocked
            }]}).to_string())]
                });
                let mut operation = operation(vec![File::picked(PathBuf::from("not-read.txt"))]);
                operation.retrying = retrying;
                let error = operation
                    .run_inner(&api, "account", &|_| {}, upload_only)
                    .unwrap_err();
                assert_eq!(error.message, expected);
                assert_eq!(error.uncertain, retrying);
                assert_eq!(operation.text, "Receipt 🎉");
                assert_eq!(operation.files.len(), 1);
                assert!(operation.files[0].attachment.is_none());
                assert_eq!(requests.join().unwrap()[0].line, "GET /api/dms HTTP/1.1");
            }
        }
    }

    #[test]
    fn eager_upload_refuses_changed_recipient_and_cancelled_drafts() {
        let (api, requests) = server(|_| {
            vec![
                (200, json!({"conversations":[]}).to_string()),
                (201, DIRECT.replace("alex-id", "other-id")),
            ]
        });
        let mut send = operation(vec![File::picked("not-read.txt".into())]);
        let error = send.run_inner(&api, "account", &|_| {}, true).unwrap_err();
        assert!(error.message.contains("username changed"));
        assert_eq!(requests.join().unwrap().len(), 2);
        send.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            send.run_inner(&api, "account", &|_| {}, true)
                .unwrap_err()
                .message,
            "Send cancelled."
        );
    }

    #[test]
    fn mismatched_confirmations_and_later_lookup_refusals_keep_retry_payload() {
        for field in ["channelId", "clientMessageId", "author"] {
            let mut operation = operation(Vec::new());
            let client_id = operation.client_id.clone();
            let (api, requests) = server(move |_| {
                let confirmed = json!({
                    "id":"m1","channelId":"dm0000000002","seq":"1",
                    "createdAt":"2026-10-11T00:00:00Z","clientMessageId":client_id,
                    "author":{"id":"owner","name":"Owner","isGuest":false},
                    "content":{"version":1,"type":"text","text":"Receipt 🎉"}
                });
                let mut wrong = confirmed.clone();
                match field {
                    "channelId" => wrong["channelId"] = json!("dm0000000003"),
                    "clientMessageId" => {
                        wrong["clientMessageId"] = json!(uuid::Uuid::new_v4().to_string())
                    }
                    "author" => wrong["author"]["id"] = json!("someone-else"),
                    _ => unreachable!(),
                }
                vec![
                    (200, format!("{{\"conversations\":[{DIRECT}]}}")),
                    (200, json!({"token":"chat","author":{"id":"owner","name":"Owner","isGuest":false}}).to_string()),
                    (200, wrong.to_string()),
                    (401, json!({"error":"Sign in again"}).to_string()),
                    (200, format!("{{\"conversations\":[{DIRECT}]}}")),
                    (200, confirmed.to_string()),
                ]
            });
            let error = operation.run(&api, "account", &|_| {}).unwrap_err();
            assert!(error.uncertain, "wrong {field} is not confirmation");
            operation.retrying = true;
            let error = operation.run(&api, "account", &|_| {}).unwrap_err();
            assert!(error.uncertain, "lookup 401 cannot disprove the prior POST");
            assert_eq!(
                operation.run(&api, "account", &|_| {}).unwrap().id,
                "dm0000000002"
            );
            let seen = requests.join().unwrap();
            assert_eq!(seen[2].body, seen[5].body);
            let sent: Value = serde_json::from_slice(&seen[5].body).unwrap();
            assert_eq!(sent["clientMessageId"], operation.client_id);
        }
    }
}
