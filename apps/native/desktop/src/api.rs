use crate::model::{
    Account, Channel, ChatSession, History, Member, Members, Message, Space, SpaceDetail, Spaces,
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
        })
    }
}

fn checked(response: Response) -> Result<Response, ApiError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let message = response
        .json::<Value>()
        .ok()
        .and_then(|body| body["error"].as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("Caper request failed ({status})."));
    Err(ApiError {
        status: Some(status),
        message,
    })
}

fn invalid(message: &str) -> ApiError {
    ApiError {
        status: None,
        message: message.into(),
    }
}
