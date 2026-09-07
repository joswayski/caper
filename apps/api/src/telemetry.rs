//! Optional Axiom OTLP log export, following the Godis telemetry convention.
//! Export IO runs on the SDK's bounded background worker, never on voice requests.
use std::{collections::HashMap, time::Duration};

use opentelemetry::KeyValue;
use opentelemetry_appender_tracing::layer::{OpenTelemetryTracingBridge, TracingSpanAttributes};
use opentelemetry_otlp::{LogExporter, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::{
    Resource,
    error::OTelSdkResult,
    logs::{
        BatchConfigBuilder, BatchLogProcessor, LogBatch, LogExporter as SdkLogExporter,
        SdkLoggerProvider,
    },
};
use tracing_subscriber::{
    EnvFilter, Layer, filter::filter_fn, layer::SubscriberExt, util::SubscriberInitExt,
};

const EXPORT_TIMEOUT: Duration = Duration::from_secs(2);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
const QUEUE_SIZE: usize = 2048;
const BATCH_SIZE: usize = 128;

// Deliberately not Debug: this contains an ingest credential.
struct AxiomConfig {
    token: String,
    dataset: String,
    endpoint: String,
    environment: String,
    instance: Option<String>,
}
impl AxiomConfig {
    fn read(get: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, &'static str> {
        let Some(token) = get("AXIOM_TOKEN").filter(|v| !v.trim().is_empty()) else {
            return Ok(None);
        };
        if !token.starts_with("xaat-") || reqwest::header::HeaderValue::from_str(&token).is_err() {
            return Err("AXIOM_TOKEN must be an Axiom API token");
        }
        // OTel header env vars override programmatic headers. Refuse ambiguity
        // rather than accidentally ingesting into another dataset/account.
        if get("OTEL_EXPORTER_OTLP_HEADERS").is_some()
            || get("OTEL_EXPORTER_OTLP_LOGS_HEADERS").is_some()
        {
            return Err("use AXIOM_TOKEN/AXIOM_DATASET instead of OTEL exporter header variables");
        }
        let dataset = get("AXIOM_DATASET").unwrap_or_else(|| "caper".into());
        if dataset.trim().is_empty() || reqwest::header::HeaderValue::from_str(&dataset).is_err() {
            return Err("AXIOM_DATASET must be a nonempty dataset name");
        }
        // Region is an operator choice; never infer it from another application's dataset.
        let endpoint =
            get("AXIOM_ENDPOINT").ok_or("AXIOM_ENDPOINT is required when AXIOM_TOKEN is set")?;
        let url = reqwest::Url::parse(&endpoint).map_err(|_| "invalid AXIOM_ENDPOINT")?;
        if url.scheme() != "https"
            || !url
                .host_str()
                .is_some_and(|host| host.ends_with(".axiom.co"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/v1/logs"
            || url.port_or_known_default() != Some(443)
        {
            return Err(
                "AXIOM_ENDPOINT must be an HTTPS Axiom /v1/logs URL without credentials or query parameters",
            );
        }
        Ok(Some(Self {
            token,
            dataset,
            endpoint,
            environment: get("ENVIRONMENT").unwrap_or_else(|| "development".into()),
            instance: get("HOSTNAME").filter(|v| !v.is_empty()),
        }))
    }
}

pub struct TelemetryGuard {
    provider: Option<SdkLoggerProvider>,
}
impl TelemetryGuard {
    pub fn shutdown(self) {
        if let Some(provider) = self.provider
            && provider.shutdown_with_timeout(SHUTDOWN_TIMEOUT).is_err()
        {
            // Do not log into the exporter being stopped, or print exporter errors/headers.
            eprintln!("Axiom log flush did not complete; some events may be lost");
        }
    }
}

pub fn init() -> TelemetryGuard {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("caper_api=info,tower_http=info"));
    let json = std::env::var("LOG_FORMAT").is_ok_and(|v| v.eq_ignore_ascii_case("json"));
    let provider = match AxiomConfig::read(|key| std::env::var(key).ok())
        .and_then(|config| config.map(build_provider).transpose())
    {
        Ok(provider) => provider,
        Err(error) => {
            eprintln!("Axiom logging disabled: {error}");
            None
        }
    };
    let bridge = provider
        .as_ref()
        .map(|provider| bridge(provider).with_filter(filter_fn(export_target)));
    let stdout = if json {
        tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .with_current_span(true)
            .with_span_list(false)
            .boxed()
    } else {
        tracing_subscriber::fmt::layer().boxed()
    };
    tracing_subscriber::registry()
        .with(filter)
        .with(stdout)
        .with(bridge)
        .init();
    tracing::info!(
        event_name = "telemetry_initialized",
        axiom_enabled = provider.is_some(),
        "logging initialized"
    );
    TelemetryGuard { provider }
}

fn bridge(
    provider: &SdkLoggerProvider,
) -> OpenTelemetryTracingBridge<SdkLoggerProvider, opentelemetry_sdk::logs::SdkLogger> {
    OpenTelemetryTracingBridge::builder(provider)
        .with_tracing_span_attributes(TracingSpanAttributes::allowlist([
            "http_method",
            "http_route",
            "request_id",
        ]))
        .build()
}

// Do not export dependency debug logs (URLs, SQL, headers) or exporter-internal
// events that could recursively generate more log traffic.
fn export_target(metadata: &tracing::Metadata<'_>) -> bool {
    // Legacy cleanup warnings include raw provider identifiers. Keep those
    // events local; request spans already use the attribute allowlist above.
    let provider_identifiers = metadata.is_event()
        && (metadata.fields().field("session").is_some()
            || metadata.fields().field("mid").is_some());
    !provider_identifiers
        && (metadata.target() == "caper_api"
            || metadata.target().starts_with("caper_api::")
            || metadata.target() == "tower_http::trace::on_failure")
}

// The SDK's internal debug diagnostics may include response bodies and URLs.
// Keep internal-logs disabled and report failures without formatting its errors.
struct ReportingExporter(LogExporter);
impl std::fmt::Debug for ReportingExporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReportingExporter").finish_non_exhaustive()
    }
}
impl SdkLogExporter for ReportingExporter {
    async fn export(&self, batch: LogBatch<'_>) -> OTelSdkResult {
        let result = self.0.export(batch).await;
        if result.is_err() {
            eprintln!("Axiom log export failed; batch may be lost");
        }
        result
    }
    fn set_resource(&mut self, resource: &Resource) {
        self.0.set_resource(resource);
    }
    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }
}

fn build_provider(config: AxiomConfig) -> Result<SdkLoggerProvider, &'static str> {
    let headers = HashMap::from([
        ("authorization".into(), format!("Bearer {}", config.token)),
        ("x-axiom-dataset".into(), config.dataset),
    ]);
    let exporter = LogExporter::builder()
        .with_http()
        .with_endpoint(config.endpoint)
        .with_headers(headers)
        .with_timeout(EXPORT_TIMEOUT)
        .build()
        .map_err(|_| "could not initialize OTLP log exporter")?;
    let processor = BatchLogProcessor::builder(ReportingExporter(exporter))
        .with_batch_config(
            BatchConfigBuilder::default()
                .with_max_queue_size(QUEUE_SIZE)
                .with_max_export_batch_size(BATCH_SIZE)
                .with_scheduled_delay(Duration::from_secs(1))
                .build(),
        )
        .build();
    let mut attributes = vec![
        KeyValue::new("deployment.environment", config.environment),
        KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
    ];
    if let Some(instance) = config.instance {
        attributes.push(KeyValue::new("service.instance.id", instance));
    }
    Ok(SdkLoggerProvider::builder()
        .with_resource(
            Resource::builder_empty()
                .with_service_name("caper-api")
                .with_attributes(attributes)
                .build(),
        )
        .with_log_processor(processor)
        .build())
}

#[cfg(test)]
mod tests;
