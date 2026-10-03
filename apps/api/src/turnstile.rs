use crate::{ApiError, RuntimeEnvironment};
use axum::http::StatusCode;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;

const SITEVERIFY: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";
const ACTION: &str = "login_email";

#[derive(Clone)]
pub(crate) struct Turnstile {
    pub site_key: String,
    secret: String,
    hostnames: Vec<String>,
    client: Client,
    endpoint: String,
}

#[derive(Deserialize)]
struct Verification {
    success: bool,
    hostname: Option<String>,
    action: Option<String>,
    #[serde(default, rename = "error-codes")]
    errors: Vec<String>,
}

impl Turnstile {
    pub fn from_env(environment: &RuntimeEnvironment) -> Result<Option<Self>, String> {
        let setting = |key| {
            environment
                .get(key)
                .filter(|value| !value.trim().is_empty())
        };
        let site_key = setting("TURNSTILE_SITE_KEY");
        let secret = setting("TURNSTILE_SECRET_KEY");
        let hostnames = setting("TURNSTILE_HOSTNAMES");
        if site_key.is_none() && secret.is_none() && hostnames.is_none() {
            return Ok(None);
        }
        let required = |value: Option<String>, key| {
            value.ok_or_else(|| format!("{key} is required when Turnstile is configured"))
        };
        let site_key = required(site_key, "TURNSTILE_SITE_KEY")?;
        let secret = required(secret, "TURNSTILE_SECRET_KEY")?;
        let hostnames: Vec<String> = required(hostnames, "TURNSTILE_HOSTNAMES")?
            .split(',')
            .map(|hostname| hostname.trim().to_ascii_lowercase())
            .collect();
        if hostnames.iter().any(|hostname| {
            hostname.is_empty()
                || !hostname
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        }) {
            return Err("TURNSTILE_HOSTNAMES must contain exact comma-separated hostnames".into());
        }
        // Cloudflare's public testing keys must never disable real bot checks in
        // hosted environments. Local tests use ENVIRONMENT=development or test.
        let test_key = site_key.starts_with("1x000000")
            || site_key.starts_with("2x000000")
            || site_key.starts_with("3x000000")
            || secret.starts_with("1x000000")
            || secret.starts_with("2x000000")
            || secret.starts_with("3x000000");
        if test_key
            && !matches!(
                environment.get("ENVIRONMENT").as_deref(),
                Some("development" | "test")
            )
        {
            return Err("Turnstile testing keys are only allowed in development or test".into());
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "could not initialize Turnstile verification")?;
        Ok(Some(Self {
            site_key,
            secret,
            hostnames,
            client,
            endpoint: SITEVERIFY.into(),
        }))
    }

    pub async fn verify(&self, token: Option<&str>) -> Result<(), ApiError> {
        let token = token.filter(|token| !token.trim().is_empty() && token.len() <= 2048);
        let token = token.ok_or_else(rejected)?;
        // Never log credentials, tokens, request bodies, or provider responses.
        let response = self
            .client
            .post(&self.endpoint)
            .form(&[("secret", self.secret.as_str()), ("response", token)])
            .send()
            .await
            .map_err(|_| unavailable())?
            .error_for_status()
            .map_err(|_| unavailable())?;
        let verification: Verification = response.json().await.map_err(|_| unavailable())?;
        if verification.errors.iter().any(|error| {
            matches!(
                error.as_str(),
                "internal-error" | "invalid-input-secret" | "missing-input-secret"
            )
        }) {
            return Err(unavailable());
        }
        if !verification.success
            || verification.action.as_deref() != Some(ACTION)
            || !verification
                .hostname
                .as_ref()
                .is_some_and(|hostname| self.hostnames.contains(hostname))
        {
            return Err(rejected());
        }
        Ok(())
    }
}

fn rejected() -> ApiError {
    ApiError::new(StatusCode::FORBIDDEN, "browser verification required")
        .with_code("turnstile_required")
}

fn unavailable() -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "browser verification unavailable",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Form, Router, routing::post};
    use serde_json::{Value, json};
    use std::{collections::HashMap, sync::Arc};
    use tokio::sync::Mutex;

    fn settings() -> RuntimeEnvironment {
        RuntimeEnvironment::from_values_for_test([
            ("TURNSTILE_SITE_KEY", "site-key"),
            ("TURNSTILE_SECRET_KEY", "test-only-secret"),
            ("TURNSTILE_HOSTNAMES", "caper.chat,staging.caper.chat"),
        ])
    }

    #[test]
    fn configuration_is_all_or_nothing_and_testing_keys_cannot_reach_production() {
        assert!(
            Turnstile::from_env(&RuntimeEnvironment::from_values_for_test([]))
                .unwrap()
                .is_none()
        );
        assert!(
            Turnstile::from_env(&RuntimeEnvironment::from_values_for_test([(
                "TURNSTILE_SITE_KEY",
                "site-key"
            ),]))
            .is_err()
        );
        assert!(
            Turnstile::from_env(&RuntimeEnvironment::from_values_for_test([
                ("TURNSTILE_SITE_KEY", "site-key"),
                ("TURNSTILE_SECRET_KEY", "secret"),
                ("TURNSTILE_HOSTNAMES", "*.caper.chat"),
            ]))
            .is_err()
        );
        for environment in ["production", "staging", "development"] {
            let result = Turnstile::from_env(&RuntimeEnvironment::from_values_for_test([
                ("TURNSTILE_SITE_KEY", "1x00000000000000000000AA"),
                (
                    "TURNSTILE_SECRET_KEY",
                    "1x0000000000000000000000000000000AA",
                ),
                ("TURNSTILE_HOSTNAMES", "localhost"),
                ("ENVIRONMENT", environment),
            ]));
            assert_eq!(result.is_ok(), environment == "development");
        }
    }

    #[tokio::test]
    async fn verification_checks_provider_success_action_hostname_and_replay() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let received = calls.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new().route("/", post(move |Form(body): Form<HashMap<String, String>>| {
            let received = received.clone();
            async move {
                assert_eq!(body["secret"], "test-only-secret");
                let token = body["response"].clone();
                let mut calls = received.lock().await;
                let replay = calls.contains(&token);
                calls.push(token.clone());
                let response = if replay {
                    json!({"success": false, "error-codes": ["timeout-or-duplicate"]})
                } else {
                    match token.as_str() {
                        "valid" => json!({"success": true, "hostname": "caper.chat", "action": ACTION}),
                        "wrong-action" => json!({"success": true, "hostname": "caper.chat", "action": "contact"}),
                        "wrong-host" => json!({"success": true, "hostname": "caper.chat.attacker.net", "action": ACTION}),
                        "missing-host" => json!({"success": true, "action": ACTION}),
                        "rejected" => json!({"success": false}),
                        "provider-error" => json!({"success": false, "error-codes": ["internal-error"]}),
                        "malformed" => json!({"hostname": "caper.chat"}),
                        "http-error" => return (StatusCode::BAD_GATEWAY, axum::Json(Value::Null)),
                        _ => panic!("unexpected test token"),
                    }
                };
                (StatusCode::OK, axum::Json(response))
            }
        }));
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut verifier = Turnstile::from_env(&settings()).unwrap().unwrap();
        verifier.endpoint = endpoint;
        for token in [None, Some(""), Some("   "), Some("a".repeat(2049).as_str())] {
            assert_eq!(
                verifier.verify(token).await.unwrap_err().status,
                StatusCode::FORBIDDEN
            );
        }
        assert!(calls.lock().await.is_empty());
        verifier.verify(Some("valid")).await.unwrap();
        for token in [
            "valid",
            "wrong-action",
            "wrong-host",
            "missing-host",
            "rejected",
        ] {
            let error = verifier.verify(Some(token)).await.unwrap_err();
            assert_eq!(error.status, StatusCode::FORBIDDEN, "{token}");
        }
        for token in ["provider-error", "malformed", "http-error"] {
            assert_eq!(
                verifier.verify(Some(token)).await.unwrap_err().status,
                StatusCode::SERVICE_UNAVAILABLE
            );
        }
        server.abort();
    }
}
