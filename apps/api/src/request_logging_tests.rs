use super::*;
use std::io::Write;
use tower::ServiceExt;

#[derive(Clone)]
struct LogWriter(Option<Arc<std::sync::Mutex<Vec<u8>>>>);
impl Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if let Some(buffer) = &self.0 {
            buffer.lock().unwrap().extend_from_slice(bytes);
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn http_logs_include_route_status_and_numeric_timing_but_not_request_secrets() {
    let buffer = Arc::new(std::sync::Mutex::new(Vec::new()));
    let writer = buffer.clone();
    let test_thread = std::thread::current().id();
    // Match production's process-lifetime subscriber. Callsite interest is
    // shared with parallel router tests; capture only this current-thread runtime.
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_writer(move || {
            LogWriter((std::thread::current().id() == test_thread).then(|| writer.clone()))
        })
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap();
    let state = AppState::new(Config::test(false), Arc::new(Cloudflare::new()));
    let request = axum::http::Request::builder()
        .uri("/api/media/status?token=SECRET_QUERY")
        .header("authorization", "Bearer SECRET_HEADER")
        .body(axum::body::Body::from("SECRET_BODY"))
        .unwrap();
    let response = app(state).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let text = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
    assert!(!text.contains("SECRET"));
    let event: Value = text
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|value| value["fields"]["event_name"] == "http_response")
        .unwrap_or_else(|| panic!("missing response event in {text:?}"));
    assert_eq!(event["span"]["http_route"], "/api/media/status");
    assert_eq!(event["span"]["http_method"], "GET");
    assert!(event["span"]["request_id"].as_str().is_some());
    assert_eq!(event["fields"]["status"], 200);
    assert!(event["fields"]["duration_ms"].is_number());
}
