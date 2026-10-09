//! FCM HTTP v1, data-only messages. The OAuth access token comes from the
//! service account's RS256 JWT assertion and is cached until shortly before
//! it expires.
use super::{Alert, Conversation, Outcome, credentials, retry_after, safe_reason};
use aws_lc_rs::signature::RsaKeyPair;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub(crate) const API: &str = "https://fcm.googleapis.com";
const TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
const TIMEOUT: Duration = Duration::from_secs(10);
/// Refresh this long before Google's stated expiry.
const TOKEN_MARGIN: Duration = Duration::from_secs(5 * 60);

pub(crate) struct Fcm {
    client: reqwest::Client,
    send_url: String,
    token_url: String,
    client_email: String,
    key_id: Option<String>,
    key: RsaKeyPair,
    token: Mutex<Option<(String, Instant)>>,
}

#[derive(Deserialize)]
struct ServiceAccount {
    project_id: String,
    client_email: String,
    private_key: String,
    private_key_id: Option<String>,
    token_uri: Option<String>,
}

#[derive(Deserialize)]
struct AccessToken {
    access_token: String,
    expires_in: u64,
}

impl Fcm {
    /// `api` is Google's host; tests pass a mock server, and point the
    /// service account's `token_uri` at it too.
    pub(crate) fn new(api: &str, service_account: &str) -> Result<Self, &'static str> {
        let account: ServiceAccount = serde_json::from_str(service_account)
            .map_err(|_| "FCM_SERVICE_ACCOUNT_JSON must be a service account JSON key")?;
        if account.project_id.is_empty()
            || !account
                .project_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err("FCM_SERVICE_ACCOUNT_JSON has an invalid project_id");
        }
        if account.client_email.is_empty() {
            return Err("FCM_SERVICE_ACCOUNT_JSON has no client_email");
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(TIMEOUT)
                .build()
                .map_err(|_| "FCM client configuration is invalid")?,
            send_url: format!(
                "{}/v1/projects/{}/messages:send",
                api.trim_end_matches('/'),
                account.project_id
            ),
            token_url: account.token_uri.unwrap_or_else(|| TOKEN_URI.into()),
            client_email: account.client_email,
            key_id: account.private_key_id,
            key: credentials::rsa_key(&account.private_key)?,
            token: Mutex::new(None),
        })
    }

    async fn access_token(&self) -> Result<String, Outcome> {
        // Held across the refresh, so concurrent sends make one token request.
        let mut cached = self.token.lock().await;
        if let Some((token, until)) = cached.as_ref()
            && Instant::now() < *until
        {
            return Ok(token.clone());
        }
        let now = chrono::Utc::now().timestamp();
        let mut header = json!({"alg": "RS256", "typ": "JWT"});
        if let Some(key_id) = &self.key_id {
            header["kid"] = json!(key_id);
        }
        let assertion = credentials::rs256(
            &self.key,
            &header,
            &json!({
                "iss": self.client_email,
                "scope": SCOPE,
                "aud": self.token_url,
                "iat": now,
                "exp": now + 3600,
            }),
        )
        .map_err(|()| Outcome::Abandon("fcm signing failed".into()))?;
        let retry = |error: String, after| Outcome::Retry { error, after };
        let response = self
            .client
            .post(&self.token_url)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
            .map_err(|_| retry("fcm oauth network".into(), None))?;
        let status = response.status();
        if !status.is_success() {
            return Err(retry(
                format!("fcm oauth {}", status.as_u16()),
                retry_after(response.headers()),
            ));
        }
        let token: AccessToken = response
            .json()
            .await
            .map_err(|_| retry("fcm oauth response".into(), None))?;
        let lifetime = Duration::from_secs(token.expires_in)
            .saturating_sub(TOKEN_MARGIN)
            .max(Duration::from_secs(60));
        *cached = Some((token.access_token.clone(), Instant::now() + lifetime));
        Ok(token.access_token)
    }

    pub(crate) async fn send(&self, token: &str, alert: &Alert) -> Outcome {
        let access_token = match self.access_token().await {
            Ok(access_token) => access_token,
            Err(outcome) => return outcome,
        };
        let response = self
            .client
            .post(&self.send_url)
            .bearer_auth(access_token)
            .json(&message(token, alert))
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(_) => {
                return Outcome::Retry {
                    error: "fcm network".into(),
                    after: None,
                };
            }
        };
        let status = response.status().as_u16();
        if response.status().is_success() {
            return Outcome::Delivered;
        }
        if status == 401 {
            // The access token was revoked or expired early: fetch a new one.
            self.token.lock().await.take();
        }
        let after = retry_after(response.headers());
        let body: Value = response.json().await.unwrap_or(Value::Null);
        outcome(status, &body, after)
    }
}

fn outcome(status: u16, body: &Value, after: Option<Duration>) -> Outcome {
    let details = body["error"]["details"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let code = details
        .iter()
        .find_map(|detail| detail["errorCode"].as_str())
        .or_else(|| body["error"]["status"].as_str())
        .map(safe_reason)
        .unwrap_or_default();
    // INVALID_ARGUMENT also covers payload mistakes; only a violation on the
    // token field means the token itself is bad.
    let bad_token = details.iter().any(|detail| {
        detail["fieldViolations"]
            .as_array()
            .is_some_and(|fields| fields.iter().any(|f| f["field"] == "message.token"))
    });
    let error = format!("fcm {status} {code}").trim_end().to_owned();
    if matches!(code, "UNREGISTERED" | "SENDER_ID_MISMATCH") || (status == 400 && bad_token) {
        return Outcome::Revoke(error);
    }
    match status {
        401 | 429 | 500..=599 => Outcome::Retry { error, after },
        _ => Outcome::Abandon(error),
    }
}

/// Data-only: the Android app builds the notification itself.
pub(super) fn message(token: &str, alert: &Alert) -> Value {
    let mut data = json!({
        "kind": alert.kind,
        "messageId": alert.message_id,
        "title": alert.title,
        "body": alert.body,
        "sender": alert.sender,
        "senderId": alert.sender_id,
    });
    if let Some(id) = alert.sender_avatar_id {
        data["senderAvatarId"] = json!(id.to_string());
    }
    match &alert.conversation {
        Conversation::Direct { id } => data["conversationId"] = json!(id),
        Conversation::Channel {
            space_id,
            channel_id,
            title,
            ..
        } => {
            data["spaceId"] = json!(space_id);
            data["channelId"] = json!(channel_id);
            data["conversationTitle"] = json!(title);
        }
    }
    json!({
        "message": {
            "token": token,
            "data": data,
            "android": {"priority": "HIGH", "ttl": "86400s", "collapse_key": alert.thread()},
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(status: u16, code: &str, field: Option<&str>) -> Value {
        let mut details = vec![json!({
            "@type": "type.googleapis.com/google.firebase.fcm.v1.FcmError",
            "errorCode": code,
        })];
        if let Some(field) = field {
            details.push(json!({
                "@type": "type.googleapis.com/google.rpc.BadRequest",
                "fieldViolations": [{"field": field, "description": "invalid"}],
            }));
        }
        json!({"error": {"code": status, "status": code, "details": details}})
    }

    #[test]
    fn revokes_only_dead_tokens_and_retries_transient_failures() {
        assert_eq!(
            outcome(404, &error(404, "UNREGISTERED", None), None),
            Outcome::Revoke("fcm 404 UNREGISTERED".into())
        );
        assert!(matches!(
            outcome(403, &error(403, "SENDER_ID_MISMATCH", None), None),
            Outcome::Revoke(_)
        ));
        assert!(matches!(
            outcome(
                400,
                &error(400, "INVALID_ARGUMENT", Some("message.token")),
                None
            ),
            Outcome::Revoke(_)
        ));
        // A payload mistake must not revoke every device.
        assert_eq!(
            outcome(
                400,
                &error(400, "INVALID_ARGUMENT", Some("message.data")),
                None
            ),
            Outcome::Abandon("fcm 400 INVALID_ARGUMENT".into())
        );
        // A bare 404 (a wrong project, say) is not a dead token either.
        assert!(matches!(
            outcome(404, &json!({"error": {"status": "NOT_FOUND"}}), None),
            Outcome::Abandon(_)
        ));
        assert_eq!(
            outcome(
                429,
                &error(429, "QUOTA_EXCEEDED", None),
                Some(Duration::from_secs(30))
            ),
            Outcome::Retry {
                error: "fcm 429 QUOTA_EXCEEDED".into(),
                after: Some(Duration::from_secs(30))
            }
        );
        for status in [401, 500, 503] {
            assert!(matches!(
                outcome(status, &Value::Null, None),
                Outcome::Retry { .. }
            ));
        }
    }
}
