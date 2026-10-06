//! Worker callbacks to the API (`/api/internal/media/{id}/…`).

use std::time::Duration;

use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::process::{EarlyPreview, Finish};

const ATTEMPTS: u32 = 3;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("api request failed: {0}")]
    Network(String),
    #[error("api returned {0}")]
    Status(StatusCode),
    #[error("api response was invalid: {0}")]
    Invalid(String),
}

/// `200` body of `POST …/start`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartResponse {
    pub id: String,
    pub upload_byte_size: i64,
    #[serde(default)]
    pub declared_content_type: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub settings: Value,
}

#[derive(Debug)]
pub enum Start {
    Job(StartResponse),
    /// `409`: deleted, already finished or unknown — nothing to do.
    Gone,
}

#[derive(Clone)]
pub struct Api {
    client: reqwest::Client,
    origin: String,
    secret: String,
}

impl Api {
    pub fn new(origin: &str, secret: String) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .user_agent("caper-media-worker")
            .build()?;
        Ok(Self {
            client,
            origin: origin.trim_end_matches('/').to_owned(),
            secret,
        })
    }

    fn url(&self, id: &str, action: &str) -> String {
        format!("{}/api/internal/media/{id}/{action}", self.origin)
    }

    /// POSTs with small bounded retries for network errors, 429 and 5xx.
    /// Returns the final status and body.
    async fn post(
        &self,
        id: &str,
        action: &str,
        body: Option<&Value>,
        attempts: u32,
    ) -> Result<(StatusCode, Vec<u8>), ApiError> {
        let mut last = ApiError::Network("not attempted".into());
        for attempt in 0..attempts {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(500 * 3u64.pow(attempt - 1))).await;
            }
            let mut request = self
                .client
                .post(self.url(id, action))
                .bearer_auth(&self.secret);
            if let Some(body) = body {
                request = request.json(body);
            }
            match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    let retry = status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS;
                    if retry && attempt + 1 < attempts {
                        last = ApiError::Status(status);
                        continue;
                    }
                    let bytes = response
                        .bytes()
                        .await
                        .map_err(|e| ApiError::Network(e.without_url().to_string()))?;
                    return Ok((status, bytes.to_vec()));
                }
                // Never log the URL (it carries the asset id only, but keep
                // the habit of logging no URLs from the worker).
                Err(error) => last = ApiError::Network(error.without_url().to_string()),
            }
        }
        Err(last)
    }

    async fn expect_ok(&self, id: &str, action: &str, body: &Value) -> Result<(), ApiError> {
        let (status, _) = self.post(id, action, Some(body), ATTEMPTS).await?;
        // 409 on a callback means the asset moved on (deleted/finished):
        // there is nothing left to report.
        if status.is_success() || status == StatusCode::CONFLICT {
            Ok(())
        } else {
            Err(ApiError::Status(status))
        }
    }

    pub async fn start(&self, id: &str) -> Result<Start, ApiError> {
        let (status, body) = self.post(id, "start", None, ATTEMPTS).await?;
        if status == StatusCode::CONFLICT {
            return Ok(Start::Gone);
        }
        if !status.is_success() {
            return Err(ApiError::Status(status));
        }
        serde_json::from_slice(&body)
            .map(Start::Job)
            .map_err(|e| ApiError::Invalid(e.to_string()))
    }

    /// Best effort, single attempt: progress is ephemeral.
    pub async fn progress(&self, id: &str, percent: u8) {
        if let Err(error) = self
            .post(id, "progress", Some(&json!({ "percent": percent })), 1)
            .await
        {
            tracing::debug!(error = %error, "progress callback failed");
        }
    }

    pub async fn preview(&self, id: &str, preview: &EarlyPreview) -> Result<(), ApiError> {
        let body = serde_json::to_value(preview).map_err(|e| ApiError::Invalid(e.to_string()))?;
        self.expect_ok(id, "preview", &body).await
    }

    pub async fn finish(&self, id: &str, finish: &Finish) -> Result<(), ApiError> {
        let body = serde_json::to_value(finish).map_err(|e| ApiError::Invalid(e.to_string()))?;
        self.expect_ok(id, "finish", &body).await
    }

    pub async fn fail(&self, id: &str, reason: &str) -> Result<(), ApiError> {
        self.expect_ok(id, "fail", &json!({ "reason": reason }))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_response_parses() {
        let body = br#"{"id":"abc","uploadByteSize":123,"declaredContentType":"image/png","filename":"a.png","settings":{"videoCrf":22}}"#;
        let start: StartResponse = serde_json::from_slice(body).unwrap();
        assert_eq!(start.upload_byte_size, 123);
        assert_eq!(start.filename, "a.png");
        assert_eq!(
            crate::settings::Settings::from_value(&start.settings).video_crf,
            22
        );
    }

    #[test]
    fn urls() {
        let api = Api::new("https://api.caper.chat/", "s".repeat(32)).unwrap();
        assert_eq!(
            api.url("abc", "start"),
            "https://api.caper.chat/api/internal/media/abc/start"
        );
    }

    /// Minimal HTTP/1.1 server answering each request with the next canned
    /// response and recording `(request line, authorization, body)`.
    async fn server(
        responses: Vec<(u16, &'static str)>,
    ) -> (
        String,
        tokio::task::JoinHandle<Vec<(String, String, String)>>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            let mut seen = Vec::new();
            for (status, body) in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let (head, body_start) = loop {
                    let n = socket.read(&mut chunk).await.unwrap();
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break (String::from_utf8_lossy(&buf[..i]).into_owned(), i + 4);
                    }
                };
                let header = |name: &str| {
                    head.lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            k.eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
                        })
                        .unwrap_or_default()
                };
                let length: usize = header("content-length").parse().unwrap_or(0);
                while buf.len() < body_start + length {
                    let n = socket.read(&mut chunk).await.unwrap();
                    buf.extend_from_slice(&chunk[..n]);
                }
                let request_line = head.lines().next().unwrap_or_default().to_owned();
                let request_body =
                    String::from_utf8_lossy(&buf[body_start..body_start + length]).into_owned();
                seen.push((request_line, header("authorization"), request_body));
                let response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.ok();
            }
            seen
        });
        (origin, handle)
    }

    #[tokio::test]
    async fn start_retries_server_errors_then_parses_the_job() {
        let job = r#"{"id":"abc","uploadByteSize":5,"declaredContentType":"text/plain","filename":"a.txt","settings":{}}"#;
        let (origin, handle) = server(vec![(503, ""), (200, job)]).await;
        let api = Api::new(&origin, "s".repeat(32)).unwrap();
        let Start::Job(start) = api.start("abc").await.unwrap() else {
            panic!("expected a job");
        };
        assert_eq!(start.upload_byte_size, 5);
        let seen = handle.await.unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[1].0, "POST /api/internal/media/abc/start HTTP/1.1");
        assert_eq!(seen[1].1, format!("Bearer {}", "s".repeat(32)));
    }

    #[tokio::test]
    async fn conflict_means_gone_and_client_errors_are_not_retried() {
        let (origin, handle) = server(vec![(409, ""), (422, ""), (409, "")]).await;
        let api = Api::new(&origin, "s".repeat(32)).unwrap();
        assert!(matches!(api.start("abc").await.unwrap(), Start::Gone));
        assert!(matches!(
            api.fail("abc", "size mismatch").await,
            Err(ApiError::Status(StatusCode::UNPROCESSABLE_ENTITY))
        ));
        // A finished/deleted asset answers callbacks with 409: nothing to do.
        api.fail("abc", "size mismatch").await.unwrap();
        let seen = handle.await.unwrap();
        assert_eq!(seen[1].0, "POST /api/internal/media/abc/fail HTTP/1.1");
        assert_eq!(seen[1].2, r#"{"reason":"size mismatch"}"#);
    }

    #[tokio::test]
    async fn exhausted_retries_report_the_last_status() {
        let (origin, handle) = server(vec![(500, ""), (502, ""), (503, "")]).await;
        let api = Api::new(&origin, "s".repeat(32)).unwrap();
        assert!(matches!(
            api.start("abc").await,
            Err(ApiError::Status(StatusCode::SERVICE_UNAVAILABLE))
        ));
        assert_eq!(handle.await.unwrap().len(), 3);
    }
}
