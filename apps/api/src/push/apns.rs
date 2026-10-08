//! APNs over HTTP/2 with token authentication: an ES256 JWT signed with the
//! team's `.p8` key, reused for about 40 minutes (Apple allows 20–60).
use super::{Alert, Conversation, Outcome, credentials, retry_after, safe_reason};
use aws_lc_rs::signature::EcdsaKeyPair;
use serde_json::{Value, json};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

pub(crate) const PRODUCTION: &str = "https://api.push.apple.com";
pub(crate) const SANDBOX: &str = "https://api.sandbox.push.apple.com";
pub(crate) const DEFAULT_TOPIC: &str = "chat.caper.ios";
const TOKEN_LIFETIME: Duration = Duration::from_secs(40 * 60);
const TIMEOUT: Duration = Duration::from_secs(10);
const EXPIRATION_SECONDS: i64 = 24 * 60 * 60;

pub(crate) struct Apns {
    client: reqwest::Client,
    base: String,
    topic: String,
    team_id: String,
    key_id: String,
    key: EcdsaKeyPair,
    token: Mutex<Option<(String, Instant)>>,
}

fn identifier(value: &str) -> bool {
    (1..=32).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_alphanumeric())
}

impl Apns {
    /// `base` is the production or sandbox host; tests pass a mock server.
    pub(crate) fn new(
        base: &str,
        topic: &str,
        team_id: &str,
        key_id: &str,
        private_key: &str,
    ) -> Result<Self, &'static str> {
        let (team_id, key_id, topic) = (team_id.trim(), key_id.trim(), topic.trim());
        if !identifier(team_id) {
            return Err("APNS_TEAM_ID must be the 10-character team ID");
        }
        if !identifier(key_id) {
            return Err("the APNs key ID must be the 10-character key ID");
        }
        if topic.is_empty() || !topic.bytes().all(|b| b.is_ascii_graphic()) {
            return Err("APNS_TOPIC must be a bundle ID");
        }
        Ok(Self {
            // APNs speaks only HTTP/2; over TLS this offers just `h2` via ALPN.
            client: reqwest::Client::builder()
                .http2_prior_knowledge()
                .timeout(TIMEOUT)
                .build()
                .map_err(|_| "APNs client configuration is invalid")?,
            base: base.trim_end_matches('/').to_owned(),
            topic: topic.to_owned(),
            team_id: team_id.to_owned(),
            key_id: key_id.to_owned(),
            key: credentials::ecdsa_key(private_key)?,
            token: Mutex::new(None),
        })
    }

    fn bearer(&self) -> Result<String, ()> {
        let mut cached = self.token.lock().unwrap();
        if let Some((token, issued)) = cached.as_ref()
            && issued.elapsed() < TOKEN_LIFETIME
        {
            return Ok(token.clone());
        }
        let token = credentials::es256(
            &self.key,
            &json!({"alg": "ES256", "kid": self.key_id}),
            &json!({"iss": self.team_id, "iat": chrono::Utc::now().timestamp()}),
        )?;
        *cached = Some((token.clone(), Instant::now()));
        Ok(token)
    }

    pub(crate) async fn send(&self, token: &str, alert: &Alert) -> Outcome {
        let Ok(bearer) = self.bearer() else {
            return Outcome::Abandon("apns signing failed".into());
        };
        let response = self
            .client
            .post(format!("{}/3/device/{token}", self.base))
            .header("authorization", format!("bearer {bearer}"))
            .header("apns-topic", &self.topic)
            .header("apns-push-type", "alert")
            .header("apns-priority", "10")
            .header(
                "apns-expiration",
                (chrono::Utc::now().timestamp() + EXPIRATION_SECONDS).to_string(),
            )
            .json(&payload(alert))
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(_) => {
                return Outcome::Retry {
                    error: "apns network".into(),
                    after: None,
                };
            }
        };
        let status = response.status().as_u16();
        if response.status().is_success() {
            return Outcome::Delivered;
        }
        let after = retry_after(response.headers());
        let body: Value = response.json().await.unwrap_or(Value::Null);
        let reason = safe_reason(body["reason"].as_str().unwrap_or_default());
        if status == 403 && reason == "ExpiredProviderToken" {
            self.token.lock().unwrap().take();
        }
        outcome(status, reason, after)
    }
}

fn outcome(status: u16, reason: &str, after: Option<Duration>) -> Outcome {
    let error = format!("apns {status} {reason}").trim_end().to_owned();
    match (status, reason) {
        // 410 Unregistered or ExpiredToken; the others are dead or foreign tokens.
        (410, _) | (400, "BadDeviceToken" | "DeviceTokenNotForTopic") => Outcome::Revoke(error),
        (403, "ExpiredProviderToken" | "InvalidProviderToken") | (429, _) | (500..=599, _) => {
            Outcome::Retry { error, after }
        }
        _ => Outcome::Abandon(error),
    }
}

/// The alert the server sends, plus what clients need to open the right place.
pub(super) fn payload(alert: &Alert) -> Value {
    let mut payload = json!({
        "aps": {
            "alert": {"title": alert.title, "body": alert.body},
            "sound": "default",
            "thread-id": alert.thread(),
        },
        "kind": alert.kind,
        "messageId": alert.message_id,
    });
    match &alert.conversation {
        Conversation::Direct { id } => payload["conversationId"] = json!(id),
        Conversation::Channel {
            space_id,
            channel_id,
            ..
        } => {
            payload["spaceId"] = json!(space_id);
            payload["channelId"] = json!(channel_id);
        }
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_dead_tokens_retries_and_permanent_failures() {
        for (status, reason) in [
            (410, "Unregistered"),
            (410, "ExpiredToken"),
            (400, "BadDeviceToken"),
            (400, "DeviceTokenNotForTopic"),
        ] {
            assert!(matches!(outcome(status, reason, None), Outcome::Revoke(_)));
        }
        for (status, reason) in [
            (429, "TooManyRequests"),
            (500, "InternalServerError"),
            (503, "ServiceUnavailable"),
            (403, "ExpiredProviderToken"),
        ] {
            assert!(matches!(
                outcome(status, reason, None),
                Outcome::Retry { .. }
            ));
        }
        assert_eq!(
            outcome(413, "PayloadTooLarge", None),
            Outcome::Abandon("apns 413 PayloadTooLarge".into())
        );
        assert_eq!(
            outcome(400, "BadTopic", None),
            Outcome::Abandon("apns 400 BadTopic".into())
        );
    }

    #[test]
    fn rejects_malformed_identifiers() {
        let (key, _) = credentials::testing::p8();
        assert!(Apns::new(PRODUCTION, DEFAULT_TOPIC, "TEAM123456", "KEY1234567", &key).is_ok());
        assert!(Apns::new(PRODUCTION, DEFAULT_TOPIC, "team id", "KEY1234567", &key).is_err());
        assert!(Apns::new(PRODUCTION, DEFAULT_TOPIC, "TEAM123456", "", &key).is_err());
        assert!(Apns::new(PRODUCTION, " ", "TEAM123456", "KEY1234567", &key).is_err());
    }
}
