//! AI provider calls through one OpenAI-compatible gateway (OpenRouter by
//! default). Features ask for a task; which models serve it is configuration,
//! so trying a newer model is an environment change, not a code change.
//! Every attempt is returned as a [`Call`] so callers can record it.
use crate::environment::RuntimeEnvironment;
use base64::Engine;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
/// Tried in order: a later model only runs when an earlier one fails.
const DEFAULT_TRANSCRIBE_MODELS: &str = "microsoft/mai-transcribe-2,elevenlabs/scribe-v2";
const TIMEOUT: Duration = Duration::from_secs(180);
/// Bounds what is kept from a provider body that is not JSON.
const MAX_ERROR_TEXT: usize = 2_000;

#[derive(Clone)]
pub(crate) struct Ai {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    transcribe_models: Vec<String>,
}

/// One provider request, as recorded in `ai_calls`.
#[derive(Debug)]
pub(crate) struct Call {
    pub model: String,
    pub succeeded: bool,
    pub http_status: Option<u16>,
    pub latency_ms: i32,
    /// The request with inline media replaced by a description of it.
    pub input: Value,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub cost_usd: Option<f64>,
}

impl Ai {
    /// Enabled by `OPENROUTER_API_KEY`. `AI_BASE_URL` points at another
    /// OpenAI-compatible gateway; `AI_TRANSCRIBE_MODELS` is a comma-separated
    /// list in fallback order.
    pub(crate) fn from_env(environment: &RuntimeEnvironment) -> Result<Option<Self>, String> {
        let Some(api_key) = environment
            .get("OPENROUTER_API_KEY")
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
        else {
            return Ok(None);
        };
        let base_url = environment
            .get("AI_BASE_URL")
            .map(|v| v.trim().trim_end_matches('/').to_owned())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.into());
        if !base_url.starts_with("https://") && !loopback(&base_url) {
            return Err("AI_BASE_URL must use https".into());
        }
        let transcribe_models =
            models(&environment.get("AI_TRANSCRIBE_MODELS").unwrap_or_default());
        Ok(Some(Self::new(base_url, api_key, transcribe_models)))
    }

    pub(crate) fn new(base_url: String, api_key: String, transcribe_models: Vec<String>) -> Self {
        let transcribe_models = if transcribe_models.is_empty() {
            models(DEFAULT_TRANSCRIBE_MODELS)
        } else {
            transcribe_models
        };
        Self {
            http: reqwest::Client::builder()
                .timeout(TIMEOUT)
                .user_agent("caper-api")
                .build()
                .expect("valid AI HTTP client"),
            base_url,
            api_key,
            transcribe_models,
        }
    }

    /// Speech to text with each configured model in turn until one succeeds.
    /// `source` describes the audio for the call log in place of its bytes.
    pub(crate) async fn transcribe(
        &self,
        audio: &[u8],
        format: &str,
        source: Value,
    ) -> (Option<String>, Vec<Call>) {
        let data = base64::engine::general_purpose::STANDARD.encode(audio);
        let mut calls = Vec::new();
        for model in &self.transcribe_models {
            let body = json!({"model": model, "input_audio": {"data": data, "format": format}});
            let logged = json!({
                "model": model,
                "input_audio": {"format": format, "bytes": audio.len(), "source": source},
            });
            let call = self
                .post("/audio/transcriptions", model, &body, logged)
                .await;
            let text = call
                .succeeded
                .then(|| {
                    call.output
                        .as_ref()?
                        .get("text")?
                        .as_str()
                        .map(str::to_owned)
                })
                .flatten();
            calls.push(call);
            if let Some(text) = text {
                return (Some(text), calls);
            }
            if let Some(last) = calls.last_mut() {
                // A 2xx body without text counts as a failure for fallback.
                if last.succeeded {
                    last.succeeded = false;
                    last.error = Some("response had no text".into());
                }
            }
        }
        (None, calls)
    }

    async fn post(&self, path: &str, model: &str, body: &Value, input: Value) -> Call {
        let started = Instant::now();
        let response = self
            .http
            .post(format!("{}{path}", self.base_url))
            .bearer_auth(&self.api_key)
            .header("X-Title", "Caper")
            .json(body)
            .send()
            .await;
        let mut call = Call {
            model: model.into(),
            succeeded: false,
            http_status: None,
            latency_ms: 0,
            input,
            output: None,
            error: None,
            cost_usd: None,
        };
        match response {
            Err(error) => {
                call.error = Some(if error.is_timeout() {
                    "timed out".into()
                } else {
                    "request failed".into()
                });
            }
            Ok(response) => {
                let status = response.status();
                call.http_status = Some(status.as_u16());
                match response.bytes().await {
                    Err(_) => call.error = Some("response body failed".into()),
                    Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
                        Ok(value) => {
                            call.cost_usd = value
                                .pointer("/usage/cost")
                                .and_then(Value::as_f64)
                                .filter(|v| v.is_finite() && *v >= 0.0);
                            if !status.is_success() {
                                call.error = Some(
                                    value
                                        .pointer("/error/message")
                                        .and_then(Value::as_str)
                                        .unwrap_or("provider error")
                                        .chars()
                                        .take(MAX_ERROR_TEXT)
                                        .collect(),
                                );
                            }
                            call.output = Some(value);
                            call.succeeded = status.is_success();
                        }
                        Err(_) => {
                            call.error = Some(
                                String::from_utf8_lossy(&bytes)
                                    .chars()
                                    .take(MAX_ERROR_TEXT)
                                    .collect(),
                            );
                        }
                    },
                }
            }
        }
        call.latency_ms = i32::try_from(started.elapsed().as_millis()).unwrap_or(i32::MAX);
        call
    }
}

fn models(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .collect()
}

fn loopback(value: &str) -> bool {
    ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|prefix| value.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::post};

    async fn serve(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{address}")
    }

    #[test]
    fn models_are_trimmed_and_default_when_unset() {
        assert_eq!(models(" a/b , ,c/d,"), vec!["a/b", "c/d"]);
        let ai = Ai::new("http://127.0.0.1:1".into(), "k".into(), Vec::new());
        assert_eq!(
            ai.transcribe_models,
            vec!["microsoft/mai-transcribe-2", "elevenlabs/scribe-v2"]
        );
    }

    #[tokio::test]
    async fn transcription_falls_back_and_logs_every_call_without_audio() {
        let base = serve(Router::new().route(
            "/audio/transcriptions",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["input_audio"]["format"], "flac");
                assert_eq!(body["input_audio"]["data"], "AQID");
                if body["model"] == "first/model" {
                    (
                        axum::http::StatusCode::BAD_GATEWAY,
                        Json(json!({"error": {"code": 502, "message": "upstream failed"}})),
                    )
                } else {
                    (
                        axum::http::StatusCode::OK,
                        Json(json!({"text": "hello there", "usage": {"seconds": 1.5, "cost": 0.0001}})),
                    )
                }
            }),
        ))
        .await;
        let ai = Ai::new(
            base,
            "key".into(),
            vec!["first/model".into(), "second/model".into()],
        );
        let (text, calls) = ai
            .transcribe(&[1, 2, 3], "flac", json!({"asset": "abc"}))
            .await;
        assert_eq!(text.as_deref(), Some("hello there"));
        assert_eq!(calls.len(), 2);
        assert!(!calls[0].succeeded);
        assert_eq!(calls[0].http_status, Some(502));
        assert_eq!(calls[0].error.as_deref(), Some("upstream failed"));
        assert!(calls[1].succeeded);
        assert_eq!(calls[1].cost_usd, Some(0.0001));
        // The log keeps the request shape and source, never the audio itself.
        assert_eq!(calls[1].input["input_audio"]["bytes"], 3);
        assert_eq!(calls[1].input["input_audio"]["source"]["asset"], "abc");
        assert!(calls[1].input["input_audio"].get("data").is_none());
    }

    #[tokio::test]
    async fn a_body_without_text_is_a_failure() {
        let base = serve(Router::new().route(
            "/audio/transcriptions",
            post(|| async { Json(json!({"usage": {}})) }),
        ))
        .await;
        let ai = Ai::new(base, "key".into(), vec!["only/model".into()]);
        let (text, calls) = ai.transcribe(&[0], "flac", Value::Null).await;
        assert!(text.is_none());
        assert!(!calls[0].succeeded);
        assert_eq!(calls[0].error.as_deref(), Some("response had no text"));
    }
}
