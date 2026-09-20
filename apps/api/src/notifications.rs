use crate::{RuntimeEnvironment, accounts::User};
use reqwest::{Client, Url};
use serde::Serialize;
use std::time::Duration;

const DELIVERY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone)]
pub(crate) struct NotificationsWebhook {
    client: Client,
    url: Option<Url>,
}

#[derive(Serialize)]
#[serde(tag = "event")]
pub(crate) enum NotificationEvent {
    #[serde(rename = "user.created")]
    UserCreated { user: UserCreatedUser },
}

#[derive(Serialize)]
pub(crate) struct UserCreatedUser {
    id: String,
    email: Option<String>,
}

impl NotificationEvent {
    pub(crate) fn user_created(user: &User) -> Self {
        Self::UserCreated {
            user: UserCreatedUser {
                id: user.external_id.clone(),
                email: user.email.clone(),
            },
        }
    }
}

impl NotificationsWebhook {
    pub(crate) fn from_env(environment: &RuntimeEnvironment) -> Self {
        let url = environment
            .get("NOTIFICATIONS_WEBHOOK_URL")
            .filter(|value| !value.trim().is_empty())
            .and_then(|value| match valid_url(&value) {
                Some(url) => Some(url),
                None => {
                    tracing::error!(
                        event_name = "notifications_webhook_disabled",
                        "notifications webhook configuration is invalid"
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

    pub(crate) fn notify(&self, event: NotificationEvent) {
        let Some(url) = self.url.clone() else {
            return;
        };
        let client = self.client.clone();
        tokio::spawn(async move {
            let result = deliver(client, url, event).await;
            match result {
                Ok(response) if response.status().is_success() => {}
                Ok(response) => tracing::warn!(
                    event_name = "notifications_webhook_delivery_failed",
                    status = response.status().as_u16(),
                    "notifications webhook delivery failed"
                ),
                Err(_) => tracing::warn!(
                    event_name = "notifications_webhook_delivery_failed",
                    "notifications webhook delivery failed"
                ),
            }
        });
    }
}

async fn deliver(
    client: Client,
    url: Url,
    event: NotificationEvent,
) -> Result<reqwest::Response, reqwest::Error> {
    client.post(url).json(&event).send().await
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
            NotificationEvent::UserCreated {
                user: UserCreatedUser {
                    id: "public-user-id".into(),
                    email: Some("person@example.com".into()),
                },
            },
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
