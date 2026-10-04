//! Mobile push is deferred until direct APNs/FCM integrations are implemented.
use crate::{AppState, auth::Principal};
use axum::{Extension, Json, Router, routing::get};
use serde_json::json;

/// Keep existing native clients' capability check behind account authentication.
/// An empty platform list hides their notification controls; there is no device
/// registration route, notification queue producer, or delivery worker.
pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/api/push/config", get(config))
}

async fn config(Extension(_principal): Extension<Principal>) -> Json<serde_json::Value> {
    Json(json!({ "platforms": [] }))
}

#[cfg(test)]
mod tests {
    use crate::{AppState, Cloudflare, Config, app};
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tower::ServiceExt;

    #[tokio::test]
    async fn capability_requires_authentication_and_advertises_no_platforms() {
        for (auth_fixture, expected) in [(false, StatusCode::UNAUTHORIZED), (true, StatusCode::OK)]
        {
            let mut config = Config::test(false);
            config.auth_fixture = auth_fixture;
            let app = app(AppState::new(config, Arc::new(Cloudflare::new())));
            let response = app
                .oneshot(
                    Request::builder()
                        .uri("/api/push/config")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
            if auth_fixture {
                let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
                assert_eq!(
                    serde_json::from_slice::<Value>(&bytes).unwrap(),
                    json!({ "platforms": [] }),
                );
            }
        }
    }
}
