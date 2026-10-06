use crate::model::{
    Account, Channel, ChatSession, DirectConversation, DirectConversations, History, Member,
    Members, Message, People, ReactionUpdate, Reactors, Space, SpaceDetail, Spaces,
};
use reqwest::blocking::{Client, Response};
use reqwest::{Method, StatusCode, redirect::Policy};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::time::Duration;
use url::Url;

#[derive(Clone)]
pub struct Api {
    base: Url,
    client: Client,
}

#[derive(Debug)]
pub struct ApiError {
    pub status: Option<StatusCode>,
    pub message: String,
    /// Remaining sign-in code attempts, when the server reports them.
    pub attempts_remaining: Option<u64>,
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
        Ok(Self { base, client })
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
        self.request(
            Method::POST,
            "api/account/profile",
            Some(token),
            None,
            Some(json!({"username":username,"displayName":display_name})),
        )
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

    pub fn chat_session(&self, token: Option<&str>, name: &str) -> Result<ChatSession, ApiError> {
        self.request(
            Method::POST,
            "api/chat/session",
            token,
            None,
            Some(json!({"name":name})),
        )
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

    pub fn send(
        &self,
        token: Option<&str>,
        chat_token: &str,
        channel: &str,
        client_id: &str,
        text: &str,
    ) -> Result<Message, ApiError> {
        self.request(
            Method::POST,
            &format!("api/chat/channels/{channel}/messages"),
            token,
            Some(chat_token),
            Some(json!({"clientMessageId":client_id,"text":text})),
        )
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
        request.send().map_err(|_| ApiError {
            status: None,
            message: "Could not reach Caper. Check your connection and try again.".into(),
            attempts_remaining: None,
        })
    }
}

fn checked(response: Response) -> Result<Response, ApiError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.json::<Value>().ok();
    let message = body
        .as_ref()
        .and_then(|body| body["error"].as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("Caper request failed ({status})."));
    Err(ApiError {
        status: Some(status),
        message,
        attempts_remaining: body.and_then(|body| body["attemptsRemaining"].as_u64()),
    })
}

fn invalid(message: &str) -> ApiError {
    ApiError {
        status: None,
        message: message.into(),
        attempts_remaining: None,
    }
}

#[cfg(test)]
mod tests {
    use super::Api;
    use std::io::{BufRead, BufReader, Write};

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
        worker.join().unwrap();
    }
}
