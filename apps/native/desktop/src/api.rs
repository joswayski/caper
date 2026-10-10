use crate::attachments::FreshUrl;
use crate::compress::Compression;
use crate::model::{
    Account, Blocks, Channel, ChatSession, DirectConversation, DirectConversations, History,
    Member, Members, Message, NotificationLevel, NotificationOverride, NotificationSettings,
    People, Privacy, ReactionUpdate, Reactors, Space, SpaceDetail, Spaces,
};
use crate::notifications::{Change, Scope};
use crate::uploads::UploadError;
use reqwest::blocking::{Client, Response};
use reqwest::{Method, StatusCode, redirect::Policy};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use url::Url;

#[derive(Clone)]
pub struct Api {
    base: Url,
    client: Client,
    /// Storage and CDN transfers: no credentials, no redirects, and no overall
    /// deadline so large uploads can finish.
    media: Client,
    /// The signed-in chat capability and the account token that minted it. It
    /// is bound to the account session, not a conversation, so navigation
    /// reuses it (like web and mobile) until the server refuses it or the
    /// profile (and so its author name) changes.
    chat_session: Arc<Mutex<Option<(String, ChatSession)>>>,
}

#[derive(Debug)]
pub struct ApiError {
    pub status: Option<StatusCode>,
    pub message: String,
    /// Remaining sign-in code attempts, when the server reports them.
    pub attempts_remaining: Option<u64>,
    /// The server's machine-readable `code`, such as `dm_blocked`.
    pub code: Option<String>,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Api {
    pub fn new(base: &str) -> Result<Self, String> {
        let mut base = Url::parse(base).map_err(|_| "CAPER_API_URL is not a valid URL")?;
        if !base.username().is_empty() || base.password().is_some() {
            return Err("Caper API URLs cannot contain credentials.".into());
        }
        let local = matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
        if base.scheme() != "https" && !(base.scheme() == "http" && local) {
            return Err("Caper requires HTTPS (plain HTTP is allowed only on loopback).".into());
        }
        base.set_path("/");
        base.set_query(None);
        base.set_fragment(None);
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(Policy::none())
            .user_agent(concat!("Caper-Desktop/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| "Could not initialize secure networking")?;
        let media = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(None)
            .redirect(Policy::none())
            .build()
            .map_err(|_| "Could not initialize secure networking")?;
        Ok(Self {
            base,
            client,
            media,
            chat_session: Arc::new(Mutex::new(None)),
        })
    }

    pub fn base(&self) -> &Url {
        &self.base
    }

    pub fn me(&self, token: &str) -> Result<Account, ApiError> {
        self.request(Method::GET, "api/account/me", Some(token), None, None)
    }

    pub fn request_code(&self, email: &str) -> Result<String, ApiError> {
        let value: Value = self.request(
            Method::POST,
            "api/auth/email/request",
            None,
            None,
            Some(json!({"email":email})),
        )?;
        value["challengeId"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| invalid("The sign-in service returned an invalid challenge."))
    }

    pub fn verify_code(&self, challenge: &str, code: &str) -> Result<(Account, String), ApiError> {
        #[derive(serde::Deserialize)]
        struct Verified {
            account: Account,
            token: String,
        }
        let verified: Verified = self.request(
            Method::POST,
            "api/auth/email/verify",
            None,
            None,
            Some(json!({"challengeId":challenge,"code":code,"tokenTransport":"bearer"})),
        )?;
        if verified.token.is_empty() {
            return Err(invalid("The sign-in service returned an invalid session."));
        }
        Ok((verified.account, verified.token))
    }

    pub fn update_profile(
        &self,
        token: &str,
        username: &str,
        display_name: &str,
    ) -> Result<Account, ApiError> {
        let account = self.request(
            Method::POST,
            "api/account/profile",
            Some(token),
            None,
            Some(json!({"username":username,"displayName":display_name})),
        )?;
        // The next conversation mints a session carrying the new author name.
        *self.chat_session.lock().unwrap() = None;
        Ok(account)
    }

    pub fn spaces(&self, token: &str) -> Result<Spaces, ApiError> {
        self.request(Method::GET, "api/spaces", Some(token), None, None)
    }

    pub fn direct_conversations(&self, token: &str) -> Result<DirectConversations, ApiError> {
        self.request(Method::GET, "api/dms", Some(token), None, None)
    }

    pub fn people(&self, token: &str) -> Result<People, ApiError> {
        self.request(Method::GET, "api/people", Some(token), None, None)
    }

    pub fn create_direct(
        &self,
        token: &str,
        username: &str,
    ) -> Result<DirectConversation, ApiError> {
        self.request(
            Method::POST,
            "api/dms",
            Some(token),
            None,
            Some(json!({"username": username})),
        )
    }

    /// Accepts an incoming message request; idempotent.
    pub fn accept_direct(&self, token: &str, id: &str) -> Result<DirectConversation, ApiError> {
        self.request(
            Method::POST,
            &format!("api/dms/{id}/accept"),
            Some(token),
            None,
            None,
        )
    }

    /// Hides an incoming request from you only; the sender is not told.
    pub fn decline_direct(&self, token: &str, id: &str) -> Result<(), ApiError> {
        checked(self.raw(
            Method::POST,
            &format!("api/dms/{id}/decline"),
            Some(token),
            None,
            None,
        )?)
        .map(|_| ())
    }

    pub fn blocks(&self, token: &str) -> Result<Blocks, ApiError> {
        self.request(Method::GET, "api/blocks", Some(token), None, None)
    }

    /// `PUT` (block) or `DELETE` (unblock) `/api/blocks/{account}`; idempotent.
    pub fn set_block(&self, token: &str, account: &str, blocked: bool) -> Result<(), ApiError> {
        let method = if blocked { Method::PUT } else { Method::DELETE };
        checked(self.raw(
            method,
            &format!("api/blocks/{account}"),
            Some(token),
            None,
            None,
        )?)
        .map(|_| ())
    }

    pub fn privacy(&self, token: &str) -> Result<Privacy, ApiError> {
        self.request(Method::GET, "api/account/privacy", Some(token), None, None)
    }

    pub fn save_privacy(&self, token: &str, direct_messages: &str) -> Result<Privacy, ApiError> {
        self.request(
            Method::PUT,
            "api/account/privacy",
            Some(token),
            None,
            Some(json!({"directMessages": direct_messages})),
        )
    }

    pub fn notification_settings(&self, token: &str) -> Result<NotificationSettings, ApiError> {
        self.request(
            Method::GET,
            "api/notifications/settings",
            Some(token),
            None,
            None,
        )
    }

    /// Sets the account level; the answer is the full settings.
    pub fn save_notification_level(
        &self,
        token: &str,
        level: NotificationLevel,
    ) -> Result<NotificationSettings, ApiError> {
        self.request(
            Method::PUT,
            "api/notifications/settings",
            Some(token),
            None,
            Some(json!({"level": level.as_str()})),
        )
    }

    /// `PUT` one field of a space, channel or DM override; `None` resets it.
    pub fn save_notification_override(
        &self,
        token: &str,
        scope: &Scope,
        change: &Change,
    ) -> Result<NotificationOverride, ApiError> {
        let path = match scope {
            Scope::Account => return Err(invalid("Invalid Caper endpoint.")),
            Scope::Space(space) => format!("api/spaces/{space}/notifications"),
            Scope::Channel { space, channel } => {
                format!("api/spaces/{space}/channels/{channel}/notifications")
            }
            Scope::Direct(id) => format!("api/dms/{id}/notifications"),
        };
        let body = match change {
            Change::Level(level) => json!({"level": level.map(NotificationLevel::as_str)}),
            Change::Mute(until) => json!({"mutedUntil": until}),
        };
        self.request(Method::PUT, &path, Some(token), None, Some(body))
    }

    pub fn read_direct(&self, token: &str, id: &str, seq: &str) -> Result<(), ApiError> {
        checked(self.raw(
            Method::POST,
            &format!("api/dms/{id}/read"),
            Some(token),
            None,
            Some(json!({"seq": seq})),
        )?)
        .map(|_| ())
    }

    pub fn general_history(&self, token: Option<&str>) -> Result<History, ApiError> {
        self.request(Method::GET, "api/chat/general", token, None, None)
    }

    pub fn space(&self, token: &str, id: &str) -> Result<SpaceDetail, ApiError> {
        self.request(
            Method::GET,
            &format!("api/spaces/{id}"),
            Some(token),
            None,
            None,
        )
    }

    pub fn history(
        &self,
        token: Option<&str>,
        channel: &str,
        before: Option<&str>,
    ) -> Result<History, ApiError> {
        let path = before.map_or_else(
            || format!("api/chat/channels/{channel}/messages"),
            |cursor| format!("api/chat/channels/{channel}/messages?before={cursor}"),
        );
        self.request(Method::GET, &path, token, None, None)
    }

    pub fn message_context(
        &self,
        token: Option<&str>,
        channel: &str,
        root: Option<&str>,
        anchor: &str,
        newer: bool,
    ) -> Result<crate::model::MessageContext, ApiError> {
        let path = root.map_or_else(
            || format!("api/chat/channels/{channel}/messages"),
            |root| format!("api/chat/channels/{channel}/messages/{root}/thread"),
        );
        let mut url = self.base.join(&path).expect("valid API path");
        url.query_pairs_mut()
            .append_pair(if newer { "after" } else { "around" }, anchor);
        self.request(Method::GET, url.as_str(), token, None, None)
    }

    pub fn chat_session(&self, token: Option<&str>, name: &str) -> Result<ChatSession, ApiError> {
        if let Some(token) = token
            && let Some((owner, session)) = self.chat_session.lock().unwrap().as_ref()
            && owner == token
        {
            return Ok(session.clone());
        }
        let session: ChatSession = self.request(
            Method::POST,
            "api/chat/session",
            token,
            None,
            Some(json!({"name":name})),
        )?;
        if let Some(token) = token
            && !session.author.is_guest
        {
            *self.chat_session.lock().unwrap() = Some((token.to_owned(), session.clone()));
        }
        Ok(session)
    }

    pub fn forward_destinations(
        &self,
        token: &str,
    ) -> Result<crate::model::ForwardDestinations, ApiError> {
        self.request(
            Method::GET,
            "api/chat/forward-destinations",
            Some(token),
            None,
            None,
        )
    }

    pub fn forward(
        &self,
        token: &str,
        chat_token: &str,
        source: &Message,
        destination: &str,
        key: &str,
        text: &str,
    ) -> Result<Message, ApiError> {
        self.request(Method::POST, &format!("api/chat/channels/{destination}/forwards"), Some(token), Some(chat_token),
            Some(json!({"sourceChannelId":source.channel_id,"sourceMessageId":source.id,"clientMessageId":key,"text":text})))
    }

    pub fn forward_conversation(
        &self,
        token: &str,
        wrapper: &Message,
        before: Option<&str>,
    ) -> Result<crate::model::ForwardConversation, ApiError> {
        let path = format!(
            "api/chat/channels/{}/forwards/{}/thread{}",
            wrapper.channel_id,
            wrapper.id,
            before
                .map(|seq| format!("?before={seq}"))
                .unwrap_or_default()
        );
        self.request(Method::GET, &path, Some(token), None, None)
    }

    pub fn typing(
        &self,
        token: Option<&str>,
        chat_token: &str,
        channel: &str,
        typing: bool,
    ) -> Result<(), ApiError> {
        let response = self.raw(
            Method::POST,
            &format!("api/chat/channels/{channel}/typing"),
            token,
            Some(chat_token),
            Some(json!({"typing":typing})),
        )?;
        checked(response).map(|_| ())
    }

    pub fn create_space(&self, token: &str, name: &str) -> Result<Space, ApiError> {
        self.request(
            Method::POST,
            "api/spaces",
            Some(token),
            None,
            Some(json!({"name":name.trim()})),
        )
    }

    pub fn update_space(&self, token: &str, id: &str, name: &str) -> Result<Space, ApiError> {
        self.request(
            Method::PATCH,
            &format!("api/spaces/{id}"),
            Some(token),
            None,
            Some(json!({"name":name.trim()})),
        )
    }

    pub fn delete_space(&self, token: &str, id: &str) -> Result<(), ApiError> {
        checked(self.raw(
            Method::DELETE,
            &format!("api/spaces/{id}"),
            Some(token),
            None,
            None,
        )?)
        .map(|_| ())
    }

    pub fn create_channel(
        &self,
        token: &str,
        space: &str,
        name: &str,
        private: bool,
    ) -> Result<Channel, ApiError> {
        self.request(
            Method::POST,
            &format!("api/spaces/{space}/channels"),
            Some(token),
            None,
            Some(json!({"name":name,"private":private})),
        )
    }

    pub fn update_channel(
        &self,
        token: &str,
        space: &str,
        channel: &str,
        name: &str,
        private: bool,
    ) -> Result<Channel, ApiError> {
        self.request(
            Method::PATCH,
            &format!("api/spaces/{space}/channels/{channel}"),
            Some(token),
            None,
            Some(json!({"name":name,"private":private})),
        )
    }

    pub fn delete_channel(&self, token: &str, space: &str, channel: &str) -> Result<(), ApiError> {
        checked(self.raw(
            Method::DELETE,
            &format!("api/spaces/{space}/channels/{channel}"),
            Some(token),
            None,
            None,
        )?)
        .map(|_| ())
    }

    pub fn join_channel(
        &self,
        token: &str,
        space: &str,
        channel: &str,
    ) -> Result<Channel, ApiError> {
        self.request(
            Method::POST,
            &format!("api/spaces/{space}/channels/{channel}/membership"),
            Some(token),
            None,
            None,
        )
    }

    pub fn leave_channel(&self, token: &str, space: &str, channel: &str) -> Result<(), ApiError> {
        checked(self.raw(
            Method::DELETE,
            &format!("api/spaces/{space}/channels/{channel}/membership"),
            Some(token),
            None,
            None,
        )?)
        .map(|_| ())
    }

    pub fn accept_channel_invitation(
        &self,
        token: &str,
        space: &str,
        channel: &str,
    ) -> Result<Channel, ApiError> {
        self.request(
            Method::POST,
            &format!("api/spaces/{space}/channels/{channel}/invitation"),
            Some(token),
            None,
            None,
        )
    }

    pub fn decline_channel_invitation(
        &self,
        token: &str,
        space: &str,
        channel: &str,
    ) -> Result<(), ApiError> {
        checked(self.raw(
            Method::DELETE,
            &format!("api/spaces/{space}/channels/{channel}/invitation"),
            Some(token),
            None,
            None,
        )?)
        .map(|_| ())
    }

    pub fn members(
        &self,
        token: &str,
        space: &str,
        channel: Option<&str>,
    ) -> Result<Members, ApiError> {
        let path = channel.map_or_else(
            || format!("api/spaces/{space}/members"),
            |channel| format!("api/spaces/{space}/channels/{channel}/members"),
        );
        self.request(Method::GET, &path, Some(token), None, None)
    }

    pub fn add_member(
        &self,
        token: &str,
        space: &str,
        channel: Option<&str>,
        username: &str,
    ) -> Result<Member, ApiError> {
        let path = channel.map_or_else(
            || format!("api/spaces/{space}/members"),
            |channel| format!("api/spaces/{space}/channels/{channel}/members"),
        );
        self.request(
            Method::POST,
            &path,
            Some(token),
            None,
            Some(json!({"username":username})),
        )
    }

    pub fn invitations(&self, token: &str, space: &str) -> Result<Members, ApiError> {
        self.request(
            Method::GET,
            &format!("api/spaces/{space}/invitations"),
            Some(token),
            None,
            None,
        )
    }

    pub fn cancel_invitation(&self, token: &str, space: &str, user: &str) -> Result<(), ApiError> {
        checked(self.raw(
            Method::DELETE,
            &format!("api/spaces/{space}/invitations/{user}"),
            Some(token),
            None,
            None,
        )?)
        .map(|_| ())
    }

    pub fn accept_invitation(&self, token: &str, space: &str) -> Result<Space, ApiError> {
        self.request(
            Method::POST,
            &format!("api/spaces/{space}/invitation"),
            Some(token),
            None,
            None,
        )
    }

    pub fn decline_invitation(&self, token: &str, space: &str) -> Result<(), ApiError> {
        checked(self.raw(
            Method::DELETE,
            &format!("api/spaces/{space}/invitation"),
            Some(token),
            None,
            None,
        )?)
        .map(|_| ())
    }

    pub fn remove_member(
        &self,
        token: &str,
        space: &str,
        channel: Option<&str>,
        member: &str,
    ) -> Result<(), ApiError> {
        let path = channel.map_or_else(
            || format!("api/spaces/{space}/members/{member}"),
            |channel| format!("api/spaces/{space}/channels/{channel}/members/{member}"),
        );
        checked(self.raw(Method::DELETE, &path, Some(token), None, None)?).map(|_| ())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send(
        &self,
        token: Option<&str>,
        chat_token: &str,
        channel: &str,
        client_id: &str,
        text: &str,
        attachment_ids: &[String],
        thread: (Option<&str>, bool),
    ) -> Result<Message, ApiError> {
        let mut body = json!({"clientMessageId":client_id,"text":text});
        // Ids join the idempotency hash; the key is present only with files.
        if !attachment_ids.is_empty() {
            body["attachmentIds"] = json!(attachment_ids);
        }
        if let Some(root) = thread.0 {
            body["threadRootId"] = json!(root);
            body["broadcast"] = json!(thread.1);
        }
        self.request(
            Method::POST,
            &format!("api/chat/channels/{channel}/messages"),
            token,
            Some(chat_token),
            Some(body),
        )
    }

    /// Uploads are optional server configuration (503 without storage). Any
    /// failure hides the attach control; success carries compression settings.
    pub fn asset_usage(&self, token: &str) -> Result<Compression, ApiError> {
        let usage: Value =
            self.request(Method::GET, "api/assets/usage", Some(token), None, None)?;
        Ok(serde_json::from_value(usage["compression"].clone()).unwrap_or_default())
    }

    /// Fresh signed URLs for visible attachments; ids the caller cannot see
    /// are omitted, and malformed entries are skipped.
    pub fn attachment_urls(
        &self,
        token: &str,
        ids: &[String],
    ) -> Result<BTreeMap<String, FreshUrl>, ApiError> {
        let body: Value = self.request(
            Method::POST,
            "api/assets/urls",
            Some(token),
            None,
            Some(json!({"ids": ids})),
        )?;
        Ok(body["urls"]
            .as_object()
            .map(|urls| {
                urls.iter()
                    .filter_map(|(id, value)| {
                        let fresh: FreshUrl = serde_json::from_value(value.clone()).ok()?;
                        let web =
                            |url: &str| url.starts_with("https://") || url.starts_with("http://");
                        // Processing files have only a preview so far.
                        ((fresh.url.is_some() || fresh.preview_url.is_some())
                            && fresh.url.as_deref().is_none_or(web)
                            && fresh.preview_url.as_deref().is_none_or(web))
                        .then(|| (id.clone(), fresh))
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// `POST /api/assets`: reserve quota and receive presigned uploads.
    pub fn create_asset(&self, token: &str, body: Value) -> Result<Value, UploadError> {
        let response = self
            .raw(Method::POST, "api/assets", Some(token), None, Some(body))
            .map_err(|error| UploadError::new(error.message))?;
        if !response.status().is_success() {
            return Err(upload_failure(response));
        }
        response
            .json()
            .map_err(|_| UploadError::new("The upload service returned an invalid response."))
    }

    /// `POST /api/assets/{id}/complete`: the API verifies the stored bytes
    /// (409: not arrived yet; 422: size or signature mismatch).
    pub fn complete_asset(&self, token: &str, id: &str) -> Result<Value, UploadError> {
        let response = self
            .raw(
                Method::POST,
                &format!("api/assets/{id}/complete"),
                Some(token),
                None,
                None,
            )
            .map_err(|error| UploadError::new(error.message))?;
        if !response.status().is_success() {
            return Err(upload_failure(response));
        }
        response
            .json()
            .map_err(|_| UploadError::new("The upload service returned an invalid response."))
    }

    /// PUT exact bytes to a presigned storage URL with exactly the returned
    /// headers. The body sets Content-Length, which the URL signs. No account
    /// credentials are sent.
    pub fn put_presigned(
        &self,
        url: &str,
        headers: &BTreeMap<String, String>,
        body: reqwest::blocking::Body,
    ) -> Result<(), UploadError> {
        let url = media_url(url).ok_or_else(|| {
            UploadError::new("The upload service returned an invalid storage address.")
        })?;
        let mut request = self.media.put(url);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = request
            .body(body)
            .send()
            .map_err(|_| UploadError::new("The upload was interrupted."))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(UploadError::new("Storage refused the upload."))
        }
    }

    /// Download attachment media from a signed CDN URL (no credentials).
    /// Errors carry the HTTP status, if any, for the expired-URL retry.
    /// Only images are fetched (previews and PNG/JPEG/GIF/WebP originals).
    /// reqwest is built without decompression features, so it
    /// sends no `Accept-Encoding` and the CDN never answers with gzip; other
    /// files open in the system browser, which decodes gzip itself.
    /// Stream a media URL into `path` (the viewer's Save; videos can be large).
    pub fn download_media(&self, url: &str, path: &std::path::Path) -> Result<(), Option<u16>> {
        let url = media_url(url).ok_or(None)?;
        let mut response = self.media.get(url).send().map_err(|_| None)?;
        let status = response.status();
        if !status.is_success() {
            return Err(Some(status.as_u16()));
        }
        let mut file = std::fs::File::create(path).map_err(|_| None)?;
        if std::io::copy(&mut response, &mut file).is_err() {
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err(None);
        }
        Ok(())
    }

    pub fn fetch_media(&self, url: &str) -> Result<Vec<u8>, Option<u16>> {
        const MAX_MEDIA_BYTES: u64 = 40 * 1024 * 1024;
        let url = media_url(url).ok_or(None)?;
        let response = self
            .media
            .get(url)
            .timeout(Duration::from_secs(60))
            .send()
            .map_err(|_| None)?;
        let status = response.status();
        if !status.is_success() {
            return Err(Some(status.as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_MEDIA_BYTES)
        {
            return Err(None);
        }
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(
            &mut std::io::Read::take(response, MAX_MEDIA_BYTES + 1),
            &mut bytes,
        )
        .map_err(|_| None)?;
        if bytes.len() as u64 > MAX_MEDIA_BYTES {
            return Err(None);
        }
        Ok(bytes)
    }

    pub fn thread(
        &self,
        token: Option<&str>,
        channel: &str,
        root: &str,
        before: Option<&str>,
    ) -> Result<crate::model::ThreadHistory, ApiError> {
        let path = format!(
            "api/chat/channels/{channel}/messages/{root}/thread{}",
            before
                .map(|cursor| format!("?before={cursor}"))
                .unwrap_or_default()
        );
        self.request(Method::GET, &path, token, None, None)
    }

    pub fn react(
        &self,
        token: Option<&str>,
        chat_token: &str,
        channel: &str,
        message: &str,
        emoji: &str,
        active: bool,
    ) -> Result<ReactionUpdate, ApiError> {
        self.request(
            Method::PUT,
            &format!("api/chat/channels/{channel}/messages/{message}/reactions"),
            token,
            Some(chat_token),
            Some(json!({"emoji":emoji,"active":active})),
        )
    }

    pub fn edit_message(
        &self,
        token: Option<&str>,
        chat_token: &str,
        original: &Message,
        text: &str,
    ) -> Result<Message, ApiError> {
        let message: Message = self.request(
            Method::PUT,
            &format!(
                "api/chat/channels/{}/messages/{}",
                original.channel_id, original.id
            ),
            token,
            Some(chat_token),
            Some(json!({"text":text,"expectedRevision":original.revision})),
        )?;
        Self::edit_snapshot(message, &original.channel_id, &original.id)
    }

    pub fn load_message(
        &self,
        token: Option<&str>,
        channel: &str,
        message: &str,
    ) -> Result<Message, ApiError> {
        let snapshot: Message = self.request(
            Method::GET,
            &format!("api/chat/channels/{channel}/messages/{message}"),
            token,
            None,
            None,
        )?;
        Self::edit_snapshot(snapshot, channel, message)
    }

    fn edit_snapshot(message: Message, channel: &str, id: &str) -> Result<Message, ApiError> {
        if message.id != id || message.channel_id != channel || message.validate().is_err() {
            return Err(ApiError {
                status: None,
                message: "Caper returned an invalid message snapshot.".into(),
                attempts_remaining: None,
                code: None,
            });
        }
        Ok(message)
    }

    pub fn message_versions(
        &self,
        token: Option<&str>,
        channel: &str,
        message: &str,
        before: Option<u32>,
    ) -> Result<crate::model::MessageVersions, ApiError> {
        let path = format!(
            "api/chat/channels/{channel}/messages/{message}/versions{}",
            before
                .map(|revision| format!("?before={revision}"))
                .unwrap_or_default()
        );
        let page: crate::model::MessageVersions =
            self.request(Method::GET, &path, token, None, None)?;
        if !page.valid(message, before) {
            return Err(ApiError {
                status: None,
                message: "Caper returned invalid message history.".into(),
                attempts_remaining: None,
                code: None,
            });
        }
        Ok(page)
    }

    /// Who reacted to one message, with the same read access (and account
    /// token) as `history`.
    pub fn reactors(
        &self,
        token: Option<&str>,
        channel: &str,
        message: &str,
    ) -> Result<Reactors, ApiError> {
        self.request(
            Method::GET,
            &format!("api/chat/channels/{channel}/messages/{message}/reactions"),
            token,
            None,
            None,
        )
    }

    pub fn pin(
        &self,
        token: Option<&str>,
        chat_token: &str,
        channel: &str,
        message: &str,
        active: bool,
    ) -> Result<crate::model::PinUpdate, ApiError> {
        self.request(
            Method::PUT,
            &format!("api/chat/channels/{channel}/messages/{message}/pin"),
            token,
            Some(chat_token),
            Some(json!({"active":active})),
        )
    }

    /// Whether voice is enabled for the General demo (`channel` None, no
    /// credentials) or for one account channel, as web reads it.
    pub fn media_status(&self, token: Option<&str>, channel: Option<&str>) -> bool {
        let path = match channel {
            Some(channel) => format!("api/channels/{channel}/media/status"),
            None => "api/media/status".into(),
        };
        let token = channel.and(token);
        // Web treats any failure or non-success response as not enabled.
        self.request::<Value>(Method::GET, &path, token, None, None)
            .is_ok_and(|status| status["enabled"].as_bool() == Some(true))
    }

    /// Asks the API to create this member's provider session and TURN
    /// credentials before Join (web's `media.prepare`). Failures only mean an
    /// ordinary join.
    pub fn prepare_voice(&self, token: &str, channel: &str) {
        let path = format!("api/channels/{channel}/media/prepare");
        let _ = self.raw(Method::POST, &path, Some(token), None, Some(json!({})));
    }

    pub fn logout(&self, token: &str) -> Result<(), ApiError> {
        *self.chat_session.lock().unwrap() = None;
        let response = self.raw(Method::POST, "api/auth/logout", Some(token), None, None)?;
        checked(response).map(|_| ())
    }

    fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        chat_token: Option<&str>,
        body: Option<Value>,
    ) -> Result<T, ApiError> {
        checked(self.raw(method, path, token, chat_token, body)?)?
            .json()
            .map_err(|_| invalid("Caper returned an invalid response."))
    }

    fn raw(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        chat_token: Option<&str>,
        body: Option<Value>,
    ) -> Result<Response, ApiError> {
        let url = self
            .base
            .join(path)
            .map_err(|_| invalid("Invalid Caper endpoint."))?;
        let mut request = self.client.request(method, url);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if let Some(chat_token) = chat_token {
            request = request.header("x-caper-chat-token", chat_token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().map_err(|_| ApiError {
            status: None,
            message: "Could not reach Caper. Check your connection and try again.".into(),
            attempts_remaining: None,
            code: None,
        })?;
        // Only 401 means the capability itself is invalid; a 403 is a refusal
        // (such as a block) that a new session would not change.
        if response.status() == StatusCode::UNAUTHORIZED
            && let Some(chat_token) = chat_token
        {
            let mut cached = self.chat_session.lock().unwrap();
            if cached
                .as_ref()
                .is_some_and(|(_, session)| session.token == chat_token)
            {
                *cached = None;
            }
        }
        Ok(response)
    }
}

/// Storage and CDN addresses must be HTTPS, or plain HTTP on loopback for
/// local development servers.
pub(crate) fn media_url(url: &str) -> Option<Url> {
    let url = Url::parse(url).ok()?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    (url.username().is_empty()
        && url.password().is_none()
        && (url.scheme() == "https" || (url.scheme() == "http" && local)))
        .then_some(url)
}

/// Web `failure`: storage-full and too-large are explicit; otherwise the
/// server's message.
fn upload_failure(response: Response) -> UploadError {
    let status = response.status();
    let body = response.json::<Value>().ok();
    if body.as_ref().and_then(|body| body["code"].as_str()) == Some("storage_full") {
        return UploadError {
            message: "You’ve used all of your file storage.".into(),
            storage_full: true,
            status: Some(status.as_u16()),
        };
    }
    let mut error = if status == StatusCode::PAYLOAD_TOO_LARGE {
        UploadError::new("This file is too large to upload.")
    } else {
        UploadError::new(
            body.as_ref()
                .and_then(|body| body["error"].as_str())
                .unwrap_or("This file could not be uploaded."),
        )
    };
    error.status = Some(status.as_u16());
    error
}

fn checked(response: Response) -> Result<Response, ApiError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.json::<Value>().ok();
    let code = body
        .as_ref()
        .and_then(|body| body["code"].as_str().map(str::to_owned));
    let message = match code.as_deref() {
        Some("dm_not_accepted") => "This person isn't accepting direct messages.".into(),
        Some("dm_blocked") => "You blocked this person. Unblock them to message them.".into(),
        // Only for display: callers branch on `status` and `code`, never this text.
        _ => body
            .as_ref()
            .and_then(|body| body["error"].as_str().map(friendly_error))
            .unwrap_or_else(|| format!("Caper request failed ({status}).")),
    };
    Err(ApiError {
        status: Some(status),
        message,
        attempts_remaining: body.and_then(|body| body["attemptsRemaining"].as_u64()),
        code,
    })
}

/// Sentences for the API's lowercase error text, copied from web's
/// `friendlyError` (`apps/web/src/spaces/errors.ts`). Keys are the server's
/// exact strings.
const SERVER_ERRORS: &[(&str, &str)] = &[
    (
        "user not found",
        "User not found. Check the username and try again.",
    ),
    (
        "account not found",
        "User not found. Check the username and try again.",
    ),
    ("enter an exact username", "Enter an exact username."),
    (
        "invalid username",
        "Use 3–32 lowercase letters, numbers, or underscores.",
    ),
    (
        "user already in space",
        "This person is already in the space.",
    ),
    (
        "user already in channel",
        "This person already has access to this channel.",
    ),
    (
        "user already invited",
        "This person already has a pending invitation.",
    ),
    (
        "user must join the space first",
        "This person needs to join the space before you can add them to a channel.",
    ),
    (
        "invitation cooldown; try again after 24 hours",
        "This person recently responded to an invitation. You can invite them again after 24 hours.",
    ),
    (
        "too many invitation attempts; try again in 10 minutes",
        "Too many invitations. Try again in 10 minutes.",
    ),
    (
        "pending invitation limit reached",
        "Too many invitations are waiting for a response. Try again later.",
    ),
    (
        "membership limit reached",
        "You’ve reached the limit of spaces you can join. Leave one to join this space.",
    ),
    ("space limit reached", "You’ve reached your space limit."),
    (
        "channel limit reached",
        "This space has reached its channel limit.",
    ),
    (
        "channel name already exists",
        "A channel with that name already exists.",
    ),
    (
        "invalid channel name",
        "Use lowercase letters separated by single dashes.",
    ),
    (
        "invalid space name",
        "Enter a space name up to 80 characters.",
    ),
    (
        "owner cannot be removed",
        "The space owner can’t be removed.",
    ),
    (
        "public channels are self-joined",
        "Anyone in the space can join a public channel without an invitation.",
    ),
    ("resource not found", "That’s no longer available."),
    ("channel not found", "This channel is no longer available."),
    (
        "conversation not found",
        "This conversation is no longer available.",
    ),
    (
        "request not found",
        "This message request is no longer available.",
    ),
    ("you can't block yourself", "You can’t block yourself."),
    (
        "too many blocked accounts",
        "You’ve blocked the maximum number of accounts.",
    ),
    ("complete profile required", "Finish your profile first."),
    (
        "unauthorized",
        "You’re signed out. Sign in again to continue.",
    ),
    (
        "spaces unavailable",
        "Caper is having trouble right now. Try again in a moment.",
    ),
    (
        "messages unavailable",
        "Messages are unavailable right now. Try again in a moment.",
    ),
];

/// Readable text for a server `error`, as web's `friendlyError`: a known
/// sentence, or the text capitalized and punctuated ("a; b" reads "A. B.").
fn friendly_error(message: &str) -> String {
    if message.trim().is_empty() {
        return "That didn’t work. Try again.".into();
    }
    if let Some((_, sentence)) = SERVER_ERRORS.iter().find(|(key, _)| *key == message) {
        return (*sentence).into();
    }
    let text = message
        .trim()
        .split(';')
        .map(str::trim_start)
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(characters).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(". ");
    if text.ends_with(['.', '!', '?', '…']) {
        text
    } else {
        format!("{text}.")
    }
}

fn invalid(message: &str) -> ApiError {
    ApiError {
        status: None,
        message: message.into(),
        attempts_remaining: None,
        code: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Api, friendly_error};
    use std::io::{BufRead, BufReader, Write};

    #[test]
    fn server_errors_read_as_web_sentences() {
        assert_eq!(
            friendly_error("channel name already exists"),
            "A channel with that name already exists."
        );
        assert_eq!(
            friendly_error("user must join the space first"),
            "This person needs to join the space before you can add them to a channel."
        );
        assert_eq!(
            friendly_error("user already in space"),
            "This person is already in the space."
        );
        assert_eq!(
            friendly_error("resource not found"),
            "That’s no longer available."
        );
        assert_eq!(
            friendly_error("invitation cooldown; try again after 24 hours"),
            "This person recently responded to an invitation. You can invite them again after 24 hours."
        );
        // Unknown text is capitalized and punctuated; readable text is kept.
        assert_eq!(
            friendly_error("too many things; try again later"),
            "Too many things. Try again later."
        );
        assert_eq!(friendly_error("message not found"), "Message not found.");
        assert_eq!(
            friendly_error("This channel is no longer accessible."),
            "This channel is no longer accessible."
        );
        assert_eq!(friendly_error("  "), "That didn’t work. Try again.");
    }

    #[test]
    fn reactor_list_uses_history_auth_and_decodes_people() {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api = Api::new(&format!("http://{}", server.local_addr().unwrap())).unwrap();
        let worker = std::thread::spawn(move || {
            for (status, body) in [
                (
                    "200 OK",
                    r#"{"messageId":"m1","reactionSeq":"12","reactions":[{"emoji":"👍","authors":[{"id":"bob","username":"bob","displayName":"Bob B","avatarId":101},{"id":"alice","username":"alice","displayName":null,"avatarId":100}]}]}"#,
                ),
                ("404 Not Found", r#"{"error":"message not found"}"#),
            ] {
                let (stream, _) = server.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(
                    request,
                    "GET /api/chat/channels/c1/messages/m1/reactions HTTP/1.1\r\n"
                );
                let mut authorized = false;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    authorized |= lower == "authorization: bearer account\r\n";
                    assert!(
                        !lower.starts_with("x-caper-chat-token"),
                        "reading needs no chat session"
                    );
                }
                assert!(authorized, "the account token authorizes the read");
                write!(
                    reader.get_mut(),
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let list = api.reactors(Some("account"), "c1", "m1").unwrap();
        assert_eq!(list.message_id, "m1");
        assert_eq!(list.reaction_seq, "12");
        let names: Vec<_> = list.reactions[0]
            .authors
            .iter()
            .map(|author| (author.id.as_str(), author.display_name.as_deref()))
            .collect();
        assert_eq!(names, [("bob", Some("Bob B")), ("alice", None)]);
        assert_eq!(list.reactions[0].authors[1].avatar_id, Some(100));
        let missing = api.reactors(Some("account"), "c1", "m1").unwrap_err();
        assert_eq!(missing.status.map(|status| status.as_u16()), Some(404));
        assert_eq!(missing.message, "Message not found.");
        worker.join().unwrap();
    }

    #[test]
    fn chat_session_is_minted_once_per_sign_in_and_replaced_only_after_401() {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api = Api::new(&format!("http://{}", server.local_addr().unwrap())).unwrap();
        let worker = std::thread::spawn(move || {
            let session = |token: &str| {
                format!(
                    r#"{{"token":"{token}","author":{{"id":"u","name":"User","isGuest":false}}}}"#
                )
            };
            let refused = r#"{"error":"this person isn't accepting direct messages","code":"dm_not_accepted"}"#;
            for (path, status, body) in [
                ("POST /api/chat/session", "200 OK", session("first")),
                (
                    "PUT /api/chat/channels/c/messages/m/reactions",
                    "403 Forbidden",
                    refused.into(),
                ),
                (
                    "PUT /api/chat/channels/c/messages/m/reactions",
                    "401 Unauthorized",
                    r#"{"error":"guest session expired"}"#.into(),
                ),
                ("POST /api/chat/session", "200 OK", session("second")),
                ("POST /api/chat/session", "200 OK", session("other-account")),
            ] {
                let (stream, _) = server.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(request, format!("{path} HTTP/1.1\r\n"));
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length: ")
                    {
                        length = value.trim().parse().unwrap();
                    }
                }
                std::io::Read::read_exact(&mut reader, &mut vec![0; length]).unwrap();
                write!(
                    reader.get_mut(),
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let first = api.chat_session(Some("account"), "User").unwrap();
        assert_eq!(first.token, "first");
        assert_eq!(
            api.chat_session(Some("account"), "User").unwrap().token,
            "first",
            "navigation reuses the account's session"
        );
        let refused = api
            .react(Some("account"), "first", "c", "m", "👍", true)
            .unwrap_err();
        assert_eq!(refused.code.as_deref(), Some("dm_not_accepted"));
        assert_eq!(
            api.chat_session(Some("account"), "User").unwrap().token,
            "first"
        );
        api.react(Some("account"), "first", "c", "m", "👍", true)
            .unwrap_err();
        assert_eq!(
            api.chat_session(Some("account"), "User").unwrap().token,
            "second"
        );
        assert_eq!(
            api.chat_session(Some("new-sign-in"), "User").unwrap().token,
            "other-account",
            "another sign-in never inherits the capability"
        );
        worker.join().unwrap();
    }

    #[test]
    fn request_block_and_privacy_endpoints_and_dm_error_codes() {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api = Api::new(&format!("http://{}", server.local_addr().unwrap())).unwrap();
        let exchanges = [
            (
                "POST /api/dms",
                "403 Forbidden",
                r#"{"error":"raw","code":"dm_not_accepted"}"#,
            ),
            (
                "POST /api/dms",
                "403 Forbidden",
                r#"{"error":"raw","code":"dm_blocked"}"#,
            ),
            (
                "POST /api/dms/dm0000000003/accept",
                "200 OK",
                r#"{"id":"dm0000000003","peer":{"id":"stranger0001","username":"jordan","displayName":"Jordan","avatarId":412},"lastSeq":"1","readSeq":"0","status":"accepted","blocked":false}"#,
            ),
            ("POST /api/dms/dm0000000003/decline", "204 No Content", ""),
            ("PUT /api/blocks/member000001", "204 No Content", ""),
            ("DELETE /api/blocks/member000001", "204 No Content", ""),
            (
                "GET /api/blocks",
                "200 OK",
                r#"{"blocks":[{"id":"member000001","username":"maya","displayName":"Maya","avatarId":null}]}"#,
            ),
            (
                "GET /api/account/privacy",
                "200 OK",
                r#"{"directMessages":"anyone"}"#,
            ),
            (
                "PUT /api/account/privacy",
                "200 OK",
                r#"{"directMessages":"nobody"}"#,
            ),
        ];
        let worker = std::thread::spawn(move || {
            let mut bodies = Vec::new();
            for (expected, status, body) in exchanges {
                let (stream, _) = server.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(request, format!("{expected} HTTP/1.1\r\n"));
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut sent = vec![0; length];
                std::io::Read::read_exact(&mut reader, &mut sent).unwrap();
                bodies.push(String::from_utf8(sent).unwrap());
                write!(
                    reader.get_mut(),
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            bodies
        });
        let refused = api.create_direct("account", "jordan").unwrap_err();
        assert_eq!(
            refused.message,
            "This person isn't accepting direct messages."
        );
        assert_eq!(refused.code.as_deref(), Some("dm_not_accepted"));
        let blocked = api.create_direct("account", "jordan").unwrap_err();
        assert_eq!(
            blocked.message,
            "You blocked this person. Unblock them to message them."
        );
        let accepted = api.accept_direct("account", "dm0000000003").unwrap();
        assert_eq!(accepted.status, crate::model::DirectStatus::Accepted);
        api.decline_direct("account", "dm0000000003").unwrap();
        api.set_block("account", "member000001", true).unwrap();
        api.set_block("account", "member000001", false).unwrap();
        assert_eq!(api.blocks("account").unwrap().blocks[0].username, "maya");
        assert_eq!(api.privacy("account").unwrap().direct_messages, "anyone");
        assert_eq!(
            api.save_privacy("account", "nobody")
                .unwrap()
                .direct_messages,
            "nobody"
        );
        let bodies = worker.join().unwrap();
        assert_eq!(bodies[0], r#"{"username":"jordan"}"#);
        assert_eq!(bodies[8], r#"{"directMessages":"nobody"}"#);
    }
}
