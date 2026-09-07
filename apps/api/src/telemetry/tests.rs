use super::*;
use opentelemetry_sdk::logs::InMemoryLogExporter;
use std::{sync::Arc, time::Instant};

fn config(values: &[(&str, &str)]) -> Result<Option<AxiomConfig>, &'static str> {
    AxiomConfig::read(|key| {
        values
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| (*value).to_owned())
    })
}

#[test]
fn logging_is_optional_and_configuration_errors_never_echo_credentials() {
    assert!(config(&[]).unwrap().is_none());
    assert!(config(&[("AXIOM_TOKEN", " ")]).unwrap().is_none());
    let values = [
        ("AXIOM_TOKEN", "xaat-test-only"),
        (
            "AXIOM_ENDPOINT",
            "https://us-east-1.aws.edge.axiom.co/v1/logs",
        ),
    ];
    let c = config(&values).unwrap().unwrap();
    assert_eq!(c.dataset, "caper");
    assert_eq!(c.environment, "development");
    assert!(
        config(&[("AXIOM_TOKEN", "SECRET-invalid")])
            .err()
            .unwrap()
            .contains("API token")
    );
    assert!(
        !config(&[("AXIOM_TOKEN", "SECRET-invalid")])
            .err()
            .unwrap()
            .contains("SECRET")
    );
    assert!(
        config(&[("AXIOM_TOKEN", "xaat-test-only")]).is_err(),
        "region is not guessed"
    );
    for endpoint in [
        "http://us-east-1.aws.edge.axiom.co/v1/logs",
        "https://example.com/v1/logs",
        "https://axiom.co.evil.test/v1/logs",
        "https://user:SECRET@api.axiom.co/v1/logs",
        "https://api.axiom.co/v1/logs?token=SECRET",
        "https://api.axiom.co/v1/traces",
        "https://api.axiom.co:444/v1/logs",
        "https://api.axiom.co/v1/logs#SECRET",
    ] {
        let error = config(&[
            ("AXIOM_TOKEN", "xaat-test-only"),
            ("AXIOM_ENDPOINT", endpoint),
        ])
        .err()
        .unwrap();
        assert!(!error.contains("SECRET"));
    }
    for key in [
        "OTEL_EXPORTER_OTLP_HEADERS",
        "OTEL_EXPORTER_OTLP_LOGS_HEADERS",
    ] {
        let mut v = values.to_vec();
        v.push((key, "authorization=SECRET"));
        assert!(
            config(&v).is_err(),
            "OTel env must not override the chosen token/dataset"
        );
    }
    let mut v = values.to_vec();
    v.push(("AXIOM_DATASET", "bad\nSECRET"));
    assert!(config(&v).is_err());
}

#[test]
fn standalone_logs_and_allowlisted_span_fields_export_without_dependency_noise() {
    let exporter = InMemoryLogExporter::default();
    let provider = SdkLoggerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let subscriber = tracing_subscriber::registry()
        .with(bridge(&provider).with_filter(filter_fn(export_target)));
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "caper_api", event_name = "service_starting", "outside a span");
        let span = tracing::info_span!(target: "caper_api", "request", http_route = "/api/media/join", request_id = "request-1", authorization = "SECRET");
        let _entered = span.enter();
        tracing::warn!(target: "caper_api", operation = "publish", upstream_status = 503u64, "provider unavailable");
        tracing::warn!(target: "caper_api", attempts = 1, session = "SECRET_SESSION", mid = "SECRET_MID", "legacy cleanup warning");
        tracing::error!(target: "reqwest", url = "https://SECRET", "dependency error");
        tracing::error!(target: "opentelemetry_sdk", "exporter error");
    });
    let logs = exporter.get_emitted_logs().unwrap();
    assert_eq!(
        logs.len(),
        2,
        "events outside spans are included; dependency/exporter logs excluded"
    );
    let debug = format!("{logs:?}");
    assert!(debug.contains("outside a span"));
    assert!(debug.contains("/api/media/join"));
    assert!(debug.contains("request-1"));
    assert!(debug.contains("upstream_status"));
    assert!(!debug.contains("SECRET"));
    assert!(!debug.contains("dependency error"));
    assert!(!debug.contains("exporter error"));
    provider.shutdown_with_timeout(SHUTDOWN_TIMEOUT).unwrap();
}

fn local_config(endpoint: String) -> AxiomConfig {
    // Only tests bypass the production HTTPS/Axiom-host endpoint validation.
    AxiomConfig {
        token: "xaat-test-only".into(),
        dataset: "caper".into(),
        endpoint,
        environment: "test".into(),
        instance: Some("test-pod".into()),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn otlp_batch_reaches_collector_with_expected_headers_and_resource_fields() {
    use axum::{Router, body::Bytes, http::HeaderMap, routing::post};
    let (sent, mut received) = tokio::sync::mpsc::channel(2);
    let router = Router::new().route(
        "/v1/logs",
        post(move |headers: HeaderMap, body: Bytes| {
            let sent = sent.clone();
            async move {
                sent.send((headers, body)).await.unwrap();
                ([("content-type", "application/x-protobuf")], Bytes::new())
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/logs", listener.local_addr().unwrap());
    let server = tokio::spawn(async { axum::serve(listener, router).await.unwrap() });
    let provider = build_provider(local_config(endpoint)).unwrap();
    let subscriber = tracing_subscriber::registry()
        .with(bridge(&provider).with_filter(filter_fn(export_target)));
    tracing::subscriber::with_default(subscriber, || {
        tracing::warn!(target: "caper_api", event_name = "axiom_test", id = "test-error-id", "test provider failure");
    });
    let (headers, body) = tokio::time::timeout(Duration::from_secs(4), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(headers["authorization"], "Bearer xaat-test-only");
    assert_eq!(headers["x-axiom-dataset"], "caper");
    assert_eq!(headers["content-type"], "application/x-protobuf");
    let payload = String::from_utf8_lossy(&body);
    for field in [
        "caper-api",
        "service.name",
        "test-pod",
        "deployment.environment",
        "axiom_test",
        "test-error-id",
        "test provider failure",
    ] {
        assert!(payload.contains(field), "missing {field} from OTLP payload");
    }
    assert!(
        !payload.contains("xaat-test-only"),
        "token only belongs in the auth header"
    );
    tokio::task::spawn_blocking(move || {
        TelemetryGuard {
            provider: Some(provider),
        }
        .shutdown()
    })
    .await
    .unwrap();
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stalled_exporter_does_not_block_log_producers_and_shutdown_is_bounded() {
    use axum::{Router, routing::post};
    let entered = Arc::new(tokio::sync::Notify::new());
    let router = Router::new().route(
        "/v1/logs",
        post({
            let entered = entered.clone();
            move || {
                let entered = entered.clone();
                async move {
                    entered.notify_one();
                    std::future::pending::<axum::http::StatusCode>().await
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/logs", listener.local_addr().unwrap());
    let server = tokio::spawn(async { axum::serve(listener, router).await.unwrap() });
    let provider = build_provider(local_config(endpoint)).unwrap();
    let subscriber = tracing_subscriber::registry()
        .with(bridge(&provider).with_filter(filter_fn(export_target)));
    let dispatch = tracing::Dispatch::new(subscriber);
    tracing::dispatcher::with_default(&dispatch, || {
        for _ in 0..BATCH_SIZE {
            tracing::info!(target: "caper_api", "start export");
        }
    });
    tokio::time::timeout(Duration::from_secs(4), entered.notified())
        .await
        .unwrap();
    let started = Instant::now();
    tracing::dispatcher::with_default(&dispatch, || {
        for _ in 0..QUEUE_SIZE * 3 {
            tracing::info!(target: "caper_api", "queue saturation");
        }
    });
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "a stalled collector must not block request-thread logging"
    );
    let started = Instant::now();
    tokio::time::timeout(
        SHUTDOWN_TIMEOUT + Duration::from_secs(1),
        tokio::task::spawn_blocking(move || {
            TelemetryGuard {
                provider: Some(provider),
            }
            .shutdown();
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(started.elapsed() < SHUTDOWN_TIMEOUT + Duration::from_secs(1));
    server.abort();
}
