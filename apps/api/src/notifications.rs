use crate::{RuntimeEnvironment, accounts::User};
use reqwest::{Client, Url};
use serde::Serialize;
use std::time::Duration;

const DELIVERY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone)]
pub(crate) struct UserCreatedWebhook {
    client: Client,
    url: Option<Url>,
}

#[derive(Serialize)]
struct UserCreatedEvent<'a> {
    event: &'static str,
    user: UserCreatedUser<'a>,
}

#[derive(Serialize)]
struct UserCreatedUser<'a> {
    id: &'a str,
    email: Option<&'a str>,
}

impl UserCreatedWebhook {
    pub(crate) fn from_env(environment: &RuntimeEnvironment) -> Self {
        let url = environment
            .get("USER_CREATED_WEBHOOK_URL")
            .filter(|value| !value.trim().is_empty())
            .and_then(|value| match valid_url(&value) {
                Some(url) => Some(url),
                None => {
                    tracing::error!(
                        event_name = "user_created_webhook_disabled",
                        "user-created webhook configuration is invalid"
                    );
                    None
                }
            });
        Self {
            client: Client::builder()
                .timeout(DELIVERY_TIMEOUT)
                .build()
                .expect("reqwest client configuration is valid"),
            url,
        }
    }

    pub(crate) fn notify(&self, user: &User) {
        let Some(url) = self.url.clone() else {
            return;
        };
        let client = self.client.clone();
        let id = user.external_id.clone();
        let email = user.email.clone();
        tokio::spawn(async move {
            let result = deliver(client, url, id, email).await;
            match result {
                Ok(response) if response.status().is_success() => {}
                Ok(response) => tracing::warn!(
                    event_name = "user_created_webhook_delivery_failed",
                    status = response.status().as_u16(),
                    "user-created webhook delivery failed"
                ),
                Err(_) => tracing::warn!(
                    event_name = "user_created_webhook_delivery_failed",
                    "user-created webhook delivery failed"
                ),
            }
        });
    }
}

async fn deliver(
    client: Client,
    url: Url,
    id: String,
    email: Option<String>,
) -> Result<reqwest::Response, reqwest::Error> {
    client
        .post(url)
        .json(&UserCreatedEvent {
            event: "user.created",
            user: UserCreatedUser {
                id: &id,
                email: email.as_deref(),
            },
        })
        .send()
        .await
}

fn valid_url(value: &str) -> Option<Url> {
    let url = Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none())
    .then_some(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::post};
    use serde_json::Value;
    use std::sync::{Arc, Mutex};
    use tokio::sync::oneshot;

    #[test]
    fn accepts_only_safe_https_webhook_urls() {
        assert!(valid_url("https://example.com/hooks/user-created").is_some());
        for value in [
            "http://example.com/hooks/user-created",
            "https://user:password@example.com/hooks/user-created",
            "https://example.com/hooks/user-created?token=secret",
            "https://example.com/hooks/user-created#secret",
        ] {
            assert!(valid_url(value).is_none(), "{value} should be rejected");
        }
    }

    #[tokio::test]
    async fn posts_the_new_user_event_without_profile_data() {
        let (sent, received) = oneshot::channel();
        let sent = Arc::new(Mutex::new(Some(sent)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/",
                    post(move |Json(body): Json<Value>| {
                        let sent = sent.clone();
                        async move {
                            if let Some(sent) = sent.lock().unwrap().take() {
                                let _ = sent.send(body);
                            }
                            axum::http::StatusCode::NO_CONTENT
                        }
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let response = deliver(
            Client::new(),
            Url::parse(&format!("http://{address}/")).unwrap(),
            "public-user-id".into(),
            Some("person@example.com".into()),
        )
        .await
        .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
        assert_eq!(
            received.await.unwrap(),
            serde_json::json!({
                "event": "user.created",
                "user": {"id": "public-user-id", "email": "person@example.com"}
            })
        );
    }
}
