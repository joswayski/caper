//! Uploaded files. Clients upload originals straight to a private S3 bucket;
//! the media worker (`apps/media-worker`) compresses each one into R2 and
//! reports back through the worker routes. The API reserves quota, presigns
//! one exact upload, tracks processing, updates messages as files finish, and
//! signs short-lived delivery URLs that the CDN Worker checks.
use crate::{
    ApiError, AppState, RuntimeEnvironment, auth::Principal, auth::random_id, chat,
    spaces::channel_access,
};
use aws_credential_types::provider::{ProvideCredentials, SharedCredentialsProvider};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{collections::HashMap, fmt::Write as _, sync::Arc, time::Duration};
use subtle::ConstantTimeEq;
use tokio::sync::{Notify, OnceCell};

const DEFAULT_QUOTA_BYTES: i64 = 10 * 1024 * 1024 * 1024;
const DEFAULT_MAX_UPLOAD_BYTES: i64 = 2 * 1024 * 1024 * 1024;
const PREVIEW_MAX_BYTES: i64 = 512 * 1024;
const UPLOAD_URL_SECONDS: u64 = 15 * 60;
const CREATES_PER_MINUTE: i64 = 30;
const MAX_PENDING: i64 = 20;
pub(crate) const MAX_PER_MESSAGE: usize = 10;
const MAX_DIMENSION: i32 = 32_768;
const MAX_DURATION_MS: i32 = 24 * 60 * 60 * 1000;
const PURGE_INTERVAL: Duration = Duration::from_secs(60);
/// Deleted files stay readable to moderators for a day before R2 removal.
const PURGE_DELAY_HOURS: i32 = 24;
/// A confirmed upload the worker has not touched for this long has failed.
/// Lambda runs for at most 15 minutes and reports progress while encoding.
const WORKER_SILENCE_MINUTES: i32 = 30;
const DAY: i64 = 24 * 60 * 60;

type HmacSha256 = Hmac<Sha256>;

fn unavailable() -> ApiError {
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "uploads unavailable")
}
fn database_error(_: sqlx::Error) -> ApiError {
    unavailable()
}
fn invalid(message: &'static str) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, message)
}
fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "file not found")
}
fn finished() -> ApiError {
    ApiError::new(StatusCode::CONFLICT, "file is not being processed")
}

pub(crate) fn incoming_key(id: &str) -> String {
    format!("incoming/{id}")
}
pub(crate) fn original_key(id: &str) -> String {
    format!("original/{id}")
}
pub(crate) fn preview_key(id: &str) -> String {
    format!("preview/{id}")
}

/// Types clients may render inline. Everything else is a download.
/// The CDN Worker keeps the same allowlist independently.
pub(crate) fn kind(content_type: &str) -> &'static str {
    match content_type {
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif" => "image",
        "video/mp4" | "video/webm" | "video/quicktime" => "video",
        "audio/mpeg" | "audio/mp4" | "audio/x-m4a" | "audio/aac" | "audio/ogg" | "audio/wav"
        | "audio/x-wav" | "audio/webm" | "audio/flac" => "audio",
        _ => "file",
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}
fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}
fn uri_encode(value: &str, encode_slash: bool) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b'/' if !encode_slash => out.push('/'),
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// One set of request-signing credentials. `token` is the session token that
/// temporary (workload identity) credentials carry.
pub(crate) struct Signing<'a> {
    pub access_key: &'a str,
    pub secret_key: &'a str,
    pub token: Option<&'a str>,
    pub region: &'a str,
}

/// AWS Signature Version 4 query presigning (S3 dialect, unsigned payload).
/// `headers` are signed in addition to `host`, so the client must send them
/// byte-for-byte; storage rejects any other Content-Type or Content-Length.
fn presign_v4(
    method: &str,
    scheme_host: &str,
    path: &str,
    signing: &Signing<'_>,
    now: DateTime<Utc>,
    expires: u64,
    headers: &[(&str, &str)],
) -> String {
    let host = scheme_host
        .split_once("://")
        .map_or(scheme_host, |(_, host)| host);
    let date = now.format("%Y%m%d").to_string();
    let stamp = now.format("%Y%m%dT%H%M%SZ").to_string();
    let scope = format!("{date}/{}/s3/aws4_request", signing.region);
    let mut signed: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .chain([("host".to_owned(), host.to_owned())])
        .collect();
    signed.sort();
    let signed_names = signed
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let mut query = [
        ("X-Amz-Algorithm", "AWS4-HMAC-SHA256".to_owned()),
        (
            "X-Amz-Credential",
            format!("{}/{scope}", signing.access_key),
        ),
        ("X-Amz-Date", stamp.clone()),
        ("X-Amz-Expires", expires.to_string()),
        ("X-Amz-SignedHeaders", signed_names.clone()),
    ]
    .into_iter()
    .chain(
        signing
            .token
            .map(|token| ("X-Amz-Security-Token", token.to_owned())),
    )
    .map(|(name, value)| format!("{}={}", uri_encode(name, true), uri_encode(&value, true)))
    .collect::<Vec<_>>();
    query.sort();
    let query = query.join("&");
    let canonical_headers: String = signed
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect();
    let path = uri_encode(path, false);
    let canonical =
        format!("{method}\n{path}\n{query}\n{canonical_headers}\n{signed_names}\nUNSIGNED-PAYLOAD");
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{stamp}\n{scope}\n{}",
        hex(&Sha256::digest(canonical.as_bytes()))
    );
    let key = [signing.region.as_bytes(), b"s3", b"aws4_request"]
        .iter()
        .fold(
            hmac(
                format!("AWS4{}", signing.secret_key).as_bytes(),
                date.as_bytes(),
            ),
            |key, part| hmac(&key, part),
        );
    let signature = hex(&hmac(&key, to_sign.as_bytes()));
    format!("{scheme_host}{path}?{query}&X-Amz-Signature={signature}")
}

fn loopback_http(value: &str) -> bool {
    let rest = value.strip_prefix("http://").unwrap_or_default();
    let host = rest.split(['/', ':']).next().unwrap_or_default();
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("static HTTP client configuration")
}

/// The R2 bucket that holds processed files. The worker writes it; the API
/// only deletes from it when purging.
#[derive(Clone)]
pub(crate) struct R2 {
    endpoint: String,
    bucket: String,
    access_key: String,
    secret_key: String,
    http: reqwest::Client,
}

impl R2 {
    fn from_env(environment: &RuntimeEnvironment) -> Result<Option<Self>, String> {
        let values = [
            "R2_ACCOUNT_ID",
            "R2_BUCKET",
            "R2_ACCESS_KEY_ID",
            "R2_SECRET_ACCESS_KEY",
        ]
        .map(|key| environment.get(key).filter(|v| !v.trim().is_empty()));
        match values {
            [None, None, None, None] => Ok(None),
            [Some(account), Some(bucket), Some(access_key), Some(secret_key)] => {
                if account.len() != 32 || !account.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("R2_ACCOUNT_ID must be a 32-character Cloudflare account ID".into());
                }
                if bucket.is_empty() || !bucket.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                    return Err("invalid R2_BUCKET".into());
                }
                // Local development only: an S3-compatible fake on loopback.
                let endpoint = match environment.get("R2_ENDPOINT").filter(|v| !v.trim().is_empty()) {
                    None => format!("https://{account}.r2.cloudflarestorage.com"),
                    Some(endpoint) if loopback_http(&endpoint) => endpoint.trim_end_matches('/').to_owned(),
                    Some(_) => return Err("R2_ENDPOINT may only point at a loopback development server".into()),
                };
                Ok(Some(Self::new(endpoint, bucket, access_key, secret_key)))
            }
            _ => Err("R2_ACCOUNT_ID, R2_BUCKET, R2_ACCESS_KEY_ID and R2_SECRET_ACCESS_KEY must be set together".into()),
        }
    }

    pub(crate) fn new(
        endpoint: String,
        bucket: String,
        access_key: String,
        secret_key: String,
    ) -> Self {
        Self {
            endpoint,
            bucket,
            access_key,
            secret_key,
            http: http_client(),
        }
    }

    fn presign(&self, method: &str, key: &str, expires: u64) -> String {
        presign_v4(
            method,
            &self.endpoint,
            &format!("/{}/{key}", self.bucket),
            &Signing {
                access_key: &self.access_key,
                secret_key: &self.secret_key,
                token: None,
                region: "auto",
            },
            Utc::now(),
            expires,
            &[],
        )
    }

    async fn delete(&self, key: &str) -> Result<(), ()> {
        let response = self
            .http
            .delete(self.presign("DELETE", key, 60))
            .send()
            .await
            .map_err(|_| ())?;
        (response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND)
            .then_some(())
            .ok_or(())
    }
}

#[derive(Clone)]
enum Credentials {
    /// Tests and loopback development.
    Static {
        access_key: String,
        secret_key: String,
    },
    /// The default AWS chain (workload identity in the cluster), resolved on
    /// first use and refreshed by the provider's own cache.
    Chain(Arc<OnceCell<SharedCredentialsProvider>>),
}

/// The private S3 bucket that receives originals. Its `ObjectCreated` events
/// feed the media worker through SQS.
#[derive(Clone)]
pub(crate) struct Incoming {
    scheme_host: String,
    path_prefix: String,
    region: String,
    credentials: Credentials,
    http: reqwest::Client,
}

impl Incoming {
    fn from_env(environment: &RuntimeEnvironment) -> Result<Option<Self>, String> {
        let Some(bucket) = environment
            .get("MEDIA_UPLOAD_BUCKET")
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
        else {
            return Ok(None);
        };
        if !(3..=63).contains(&bucket.len())
            || !bucket
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err("invalid MEDIA_UPLOAD_BUCKET".into());
        }
        let region = ["MEDIA_UPLOAD_REGION", "AWS_REGION", "AWS_DEFAULT_REGION"]
            .iter()
            .find_map(|key| environment.get(key).filter(|v| !v.trim().is_empty()))
            .ok_or("MEDIA_UPLOAD_REGION or AWS_REGION is required with MEDIA_UPLOAD_BUCKET")?;
        if !region
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err("invalid MEDIA_UPLOAD_REGION".into());
        }
        let chain = Credentials::Chain(Arc::default());
        Ok(Some(
            match environment
                .get("MEDIA_UPLOAD_ENDPOINT")
                .filter(|v| !v.trim().is_empty())
            {
                None => Self {
                    scheme_host: format!("https://{bucket}.s3.{region}.amazonaws.com"),
                    path_prefix: String::new(),
                    region,
                    credentials: chain,
                    http: http_client(),
                },
                // Local development only: an S3-compatible fake on loopback.
                // Loopback fakes do not check signatures; never reaches AWS.
                Some(endpoint) if loopback_http(&endpoint) => Self {
                    region,
                    ..Self::local(
                        endpoint.trim_end_matches('/').to_owned(),
                        bucket,
                        "local".into(),
                        "local".into(),
                    )
                },
                Some(_) => {
                    return Err(
                        "MEDIA_UPLOAD_ENDPOINT may only point at a loopback development server"
                            .into(),
                    );
                }
            },
        ))
    }

    /// Path-style bucket on a loopback endpoint with fixed credentials.
    pub(crate) fn local(
        endpoint: String,
        bucket: String,
        access_key: String,
        secret_key: String,
    ) -> Self {
        Self {
            scheme_host: endpoint,
            path_prefix: format!("/{bucket}"),
            region: "us-east-1".into(),
            credentials: Credentials::Static {
                access_key,
                secret_key,
            },
            http: http_client(),
        }
    }

    async fn presign(
        &self,
        method: &str,
        key: &str,
        expires: u64,
        headers: &[(&str, &str)],
    ) -> Result<String, ApiError> {
        let path = format!("{}/{key}", self.path_prefix);
        let sign = |access_key: &str, secret_key: &str, token: Option<&str>| {
            presign_v4(
                method,
                &self.scheme_host,
                &path,
                &Signing {
                    access_key,
                    secret_key,
                    token,
                    region: &self.region,
                },
                Utc::now(),
                expires,
                headers,
            )
        };
        match &self.credentials {
            Credentials::Static {
                access_key,
                secret_key,
            } => Ok(sign(access_key, secret_key, None)),
            Credentials::Chain(cell) => {
                let provider = cell
                    .get_or_try_init(|| async {
                        aws_config::load_defaults(aws_config::BehaviorVersion::latest())
                            .await
                            .credentials_provider()
                            .ok_or(())
                    })
                    .await
                    .map_err(|()| unavailable())?;
                let credentials = provider
                    .provide_credentials()
                    .await
                    .map_err(|_| unavailable())?;
                Ok(sign(
                    credentials.access_key_id(),
                    credentials.secret_access_key(),
                    credentials.session_token(),
                ))
            }
        }
    }

    /// Stored size, or `None` when the object does not exist.
    async fn size(&self, key: &str) -> Result<Option<i64>, ApiError> {
        let response = self
            .http
            .head(self.presign("HEAD", key, 60, &[]).await?)
            .send()
            .await
            .map_err(|_| unavailable())?;
        match response.status() {
            reqwest::StatusCode::NOT_FOUND => Ok(None),
            status if status.is_success() => Ok(response
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())),
            _ => Err(unavailable()),
        }
    }
}

/// Delivery URLs for the CDN Worker. Expiry snaps to UTC day boundaries, so a
/// URL stays identical (and browser-cacheable) for a day and lives 24–48 hours.
#[derive(Clone)]
pub(crate) struct CdnSigner {
    origin: String,
    secret: Vec<u8>,
}

impl CdnSigner {
    pub(crate) fn from_env(environment: &RuntimeEnvironment) -> Result<Option<Arc<Self>>, String> {
        match (
            environment.get("ASSET_CDN_ORIGIN"),
            environment.get("ASSET_CDN_SIGNING_SECRET"),
        ) {
            (None, None) => Ok(None),
            (Some(origin), Some(secret)) => {
                let origin = origin.trim().trim_end_matches('/').to_owned();
                let local = origin.starts_with("http://localhost")
                    || origin.starts_with("http://127.0.0.1");
                if !(origin.starts_with("https://") || local) || origin.contains('?') {
                    return Err("ASSET_CDN_ORIGIN must be an https origin".into());
                }
                if secret.len() < 32 {
                    return Err(
                        "ASSET_CDN_SIGNING_SECRET must contain at least 32 characters".into(),
                    );
                }
                Ok(Some(Arc::new(Self::new(origin, secret.into_bytes()))))
            }
            _ => Err("ASSET_CDN_ORIGIN and ASSET_CDN_SIGNING_SECRET must be set together".into()),
        }
    }

    pub(crate) fn new(origin: String, secret: Vec<u8>) -> Self {
        Self { origin, secret }
    }

    pub(crate) fn url(&self, key: &str, now: i64) -> String {
        let expires = (now.div_euclid(DAY) + 2) * DAY;
        let signature =
            URL_SAFE_NO_PAD.encode(hmac(&self.secret, format!("{key}\n{expires}").as_bytes()));
        format!("{}/{key}?exp={expires}&sig={signature}", self.origin)
    }
}

/// Adds fresh delivery URLs to a message, a `message.created` event or a
/// `message.attachments` event. URLs are never stored: every response and
/// socket frame signs at the moment it leaves.
pub(crate) fn sign_attachments(mut payload: Value, signer: Option<&CdnSigner>) -> Value {
    let Some(signer) = signer else {
        return payload;
    };
    let now = Utc::now().timestamp();
    let pointer = if payload.get("type").and_then(Value::as_str) == Some("message.attachments") {
        "/attachments"
    } else if payload.get("message").is_some() {
        "/message/content/attachments"
    } else {
        "/content/attachments"
    };
    if let Some(attachments) = payload.pointer_mut(pointer).and_then(Value::as_array_mut) {
        for attachment in attachments.iter_mut().filter_map(Value::as_object_mut) {
            sign_one(attachment, signer, now);
        }
    }
    payload
}

fn sign_one(attachment: &mut Map<String, Value>, signer: &CdnSigner, now: i64) {
    let Some(id) = attachment
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return;
    };
    if attachment.get("unavailable") == Some(&Value::Bool(true)) {
        return;
    }
    // Payloads written before processing existed carry no status: ready.
    let status = attachment
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("ready")
        .to_owned();
    if status == "ready" {
        attachment.insert("url".into(), json!(signer.url(&original_key(&id), now)));
    }
    if status != "failed" && attachment.get("preview").is_some_and(Value::is_object) {
        attachment.insert(
            "previewUrl".into(),
            json!(signer.url(&preview_key(&id), now)),
        );
    }
}

/// Compression settings the worker applies, sent with each job so operators
/// tune them with API configuration alone. `profile` selects a set (one today;
/// room for per-plan tiers).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaSettings {
    pub image_avif_quality: u8,
    pub image_avif_speed: u8,
    /// Keep lossless WebP when it is at most this many times the AVIF size.
    pub image_lossless_ratio: f64,
    /// Keep the original image unless re-encoding saves at least this much.
    pub image_min_savings_percent: u8,
    /// Longest still edge in pixels; 0 keeps full resolution.
    pub image_max_edge: u32,
    pub preview_edge: u32,
    pub video_crf: u8,
    pub video_preset: String,
    pub video_long_seconds: u32,
    pub video_long_preset: String,
    /// Short-edge cap in pixels; 0 keeps the resolution.
    pub video_max_height: u32,
    pub audio_kbps: u32,
    pub file_min_savings_percent: u8,
}

impl Default for MediaSettings {
    fn default() -> Self {
        Self {
            image_avif_quality: 90,
            image_avif_speed: 6,
            image_lossless_ratio: 1.3,
            image_min_savings_percent: 15,
            image_max_edge: 0,
            preview_edge: 640,
            video_crf: 20,
            video_preset: "slow".into(),
            video_long_seconds: 300,
            video_long_preset: "veryfast".into(),
            video_max_height: 1080,
            audio_kbps: 128,
            file_min_savings_percent: 10,
        }
    }
}

const PRESETS: [&str; 9] = [
    "ultrafast",
    "superfast",
    "veryfast",
    "faster",
    "fast",
    "medium",
    "slow",
    "slower",
    "veryslow",
];

fn setting<T: std::str::FromStr + PartialOrd>(
    environment: &RuntimeEnvironment,
    name: &str,
    default: T,
    range: std::ops::RangeInclusive<T>,
) -> Result<T, String> {
    match environment.get(name).filter(|v| !v.trim().is_empty()) {
        None => Ok(default),
        Some(value) => value
            .trim()
            .parse::<T>()
            .ok()
            .filter(|v| range.contains(v))
            .ok_or_else(|| format!("{name} is out of range")),
    }
}

fn preset(environment: &RuntimeEnvironment, name: &str, default: &str) -> Result<String, String> {
    match environment.get(name).filter(|v| !v.trim().is_empty()) {
        None => Ok(default.to_owned()),
        Some(value) if PRESETS.contains(&value.trim()) => Ok(value.trim().to_owned()),
        Some(_) => Err(format!("{name} must be an x264 preset")),
    }
}

impl MediaSettings {
    pub(crate) fn from_env(environment: &RuntimeEnvironment) -> Result<Self, String> {
        let d = Self::default();
        Ok(Self {
            image_avif_quality: setting(
                environment,
                "MEDIA_IMAGE_AVIF_QUALITY",
                d.image_avif_quality,
                1..=100,
            )?,
            image_avif_speed: setting(
                environment,
                "MEDIA_IMAGE_AVIF_SPEED",
                d.image_avif_speed,
                0..=10,
            )?,
            image_lossless_ratio: setting(
                environment,
                "MEDIA_IMAGE_LOSSLESS_RATIO",
                d.image_lossless_ratio,
                0.0..=100.0,
            )?,
            image_min_savings_percent: setting(
                environment,
                "MEDIA_IMAGE_MIN_SAVINGS_PERCENT",
                d.image_min_savings_percent,
                0..=99,
            )?,
            image_max_edge: setting(
                environment,
                "MEDIA_IMAGE_MAX_EDGE",
                d.image_max_edge,
                0..=65_535,
            )?,
            preview_edge: setting(environment, "MEDIA_PREVIEW_EDGE", d.preview_edge, 64..=2048)?,
            video_crf: setting(environment, "MEDIA_VIDEO_CRF", d.video_crf, 0..=51)?,
            video_preset: preset(environment, "MEDIA_VIDEO_PRESET", &d.video_preset)?,
            video_long_seconds: setting(
                environment,
                "MEDIA_VIDEO_LONG_SECONDS",
                d.video_long_seconds,
                1..=86_400,
            )?,
            video_long_preset: preset(
                environment,
                "MEDIA_VIDEO_LONG_PRESET",
                &d.video_long_preset,
            )?,
            video_max_height: setting(
                environment,
                "MEDIA_VIDEO_MAX_HEIGHT",
                d.video_max_height,
                0..=4320,
            )?,
            audio_kbps: setting(environment, "MEDIA_AUDIO_KBPS", d.audio_kbps, 32..=512)?,
            file_min_savings_percent: setting(
                environment,
                "MEDIA_FILE_MIN_SAVINGS_PERCENT",
                d.file_min_savings_percent,
                0..=99,
            )?,
        })
    }
}

#[derive(Clone)]
pub(crate) struct Assets {
    pool: PgPool,
    r2: R2,
    incoming: Incoming,
    quota_bytes: i64,
    max_upload_bytes: i64,
    settings: MediaSettings,
    worker_secret: Arc<str>,
}

impl Assets {
    /// Uploads turn on with `MEDIA_UPLOAD_BUCKET`, which then requires R2 (for
    /// purging) and the worker secret. Without it the routes return 503.
    pub(crate) fn from_env(
        pool: Option<&PgPool>,
        environment: &RuntimeEnvironment,
    ) -> Result<Option<Self>, String> {
        let r2 = R2::from_env(environment)?;
        let Some(incoming) = Incoming::from_env(environment)? else {
            return Ok(None);
        };
        let r2 = r2.ok_or("MEDIA_UPLOAD_BUCKET requires the R2_* settings")?;
        let pool = pool.ok_or("uploads require DATABASE_URL")?.clone();
        let worker_secret = environment
            .get("MEDIA_WORKER_SECRET")
            .filter(|v| v.len() >= 32)
            .ok_or("MEDIA_WORKER_SECRET must contain at least 32 characters")?;
        let positive = |name: &str, default: i64| match environment.get(name) {
            None => Ok(default),
            Some(value) => value
                .trim()
                .parse::<i64>()
                .ok()
                .filter(|v| *v > 0)
                .ok_or(format!("{name} must be a positive integer")),
        };
        let mut assets = Self::new(pool, r2, incoming, worker_secret);
        assets.quota_bytes = positive("ASSET_QUOTA_BYTES", DEFAULT_QUOTA_BYTES)?;
        // A single S3 PUT accepts at most 5 GiB.
        assets.max_upload_bytes = positive("MEDIA_MAX_UPLOAD_BYTES", DEFAULT_MAX_UPLOAD_BYTES)?
            .min(5 * 1024 * 1024 * 1024);
        assets.settings = MediaSettings::from_env(environment)?;
        Ok(Some(assets))
    }

    pub(crate) fn new(pool: PgPool, r2: R2, incoming: Incoming, worker_secret: String) -> Self {
        Self {
            pool,
            r2,
            incoming,
            quota_bytes: DEFAULT_QUOTA_BYTES,
            max_upload_bytes: DEFAULT_MAX_UPLOAD_BYTES,
            settings: MediaSettings::default(),
            worker_secret: worker_secret.into(),
        }
    }
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/assets", post(create))
        .route("/api/assets/usage", get(usage))
        .route("/api/assets/urls", post(urls))
        .route("/api/assets/{asset}/complete", post(complete))
}

/// Media worker callbacks, authenticated with `MEDIA_WORKER_SECRET` instead of
/// an account session.
pub(crate) fn worker_routes() -> Router<AppState> {
    Router::new()
        .route("/api/internal/media/{asset}/start", post(worker_start))
        .route(
            "/api/internal/media/{asset}/progress",
            post(worker_progress),
        )
        .route("/api/internal/media/{asset}/preview", post(worker_preview))
        .route("/api/internal/media/{asset}/finish", post(worker_finish))
        .route("/api/internal/media/{asset}/fail", post(worker_fail))
}

fn enabled(state: &AppState) -> Result<&Assets, ApiError> {
    state.assets.as_ref().ok_or_else(unavailable)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateInput {
    channel_id: String,
    filename: String,
    content_type: String,
    byte_size: i64,
}

fn filename(value: &str) -> Result<String, ApiError> {
    let name = value.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.chars().count() > 255
        || name.chars().any(char::is_control)
    {
        return Err(invalid("invalid file name"));
    }
    Ok(name.to_owned())
}

fn content_type(value: &str) -> Result<String, ApiError> {
    let value = value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if value.is_empty() {
        return Ok("application/octet-stream".into());
    }
    let token = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&b))
    };
    match value.split_once('/') {
        Some((top, sub)) if value.len() <= 127 && token(top) && token(sub) => Ok(value),
        _ => Err(invalid("invalid content type")),
    }
}

fn optional_range(value: Option<i32>, max: i32, min: i32) -> Result<Option<i32>, ApiError> {
    match value {
        Some(v) if !(min..=max).contains(&v) => Err(invalid("invalid media dimensions")),
        v => Ok(v),
    }
}

#[derive(sqlx::FromRow)]
struct Row {
    external_id: String,
    status: String,
    kind: String,
    content_type: String,
    filename: String,
    byte_size: i64,
    animated: bool,
    width: Option<i32>,
    height: Option<i32>,
    duration_ms: Option<i32>,
    preview_content_type: Option<String>,
    uploaded_at: Option<DateTime<Utc>>,
}
const COLUMNS: &str = "external_id,status,kind,content_type,filename,byte_size,animated,width,height,duration_ms,preview_content_type,uploaded_at";

/// The attachment shape embedded in message content (without URLs). While a
/// file is processing, `kind`, `contentType`, `name` and `size` describe the
/// upload.
fn describe(row: &Row) -> Value {
    let mut value = json!({
        "id": row.external_id,
        "kind": row.kind,
        "contentType": row.content_type,
        "name": row.filename,
        "size": row.byte_size,
        "status": if row.status == "uploading" { "processing" } else { row.status.as_str() },
    });
    for (name, v) in [
        ("width", row.width),
        ("height", row.height),
        ("durationMs", row.duration_ms),
    ] {
        if let Some(v) = v {
            value[name] = json!(v);
        }
    }
    // `finish` is authoritative: a video that ended up stored as a plain file
    // keeps its early poster object (purged with the row) but shows none.
    if row.preview_content_type.is_some()
        && row.status != "failed"
        && matches!(row.kind.as_str(), "image" | "video")
    {
        value["preview"] = json!({});
    }
    if row.animated {
        value["animated"] = json!(true);
    }
    value
}

async fn stored_bytes(
    connection: &mut sqlx::PgConnection,
    user: i64,
) -> Result<(i64, i64, i64), ApiError> {
    sqlx::query_as(
        "SELECT COALESCE(sum(byte_size + COALESCE(preview_byte_size,0)) FILTER (WHERE purged_at IS NULL),0)::bigint,
                count(*) FILTER (WHERE created_at > now() - interval '1 minute'),
                count(*) FILTER (WHERE uploaded_at IS NULL AND deleted_at IS NULL)
         FROM public.assets WHERE owner_id=$1",
    )
    .bind(user)
    .fetch_one(connection)
    .await
    .map_err(database_error)
}

async fn create(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<CreateInput>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let assets = enabled(&state)?;
    let name = filename(&input.filename)?;
    let content_type = content_type(&input.content_type)?;
    let kind = kind(&content_type);
    if input.byte_size < 1 {
        return Err(invalid("invalid file size"));
    }
    if input.byte_size > assets.max_upload_bytes {
        return Err(
            ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "file is too large")
                .with_code("file_too_large"),
        );
    }
    let access = channel_access(&assets.pool, &input.channel_id, Some(principal.user.id)).await?;

    let id = random_id(16);
    let mut tx = assets.pool.begin().await.map_err(database_error)?;
    // Serialize one person's reservations so parallel uploads cannot overshoot.
    sqlx::query("SELECT 1 FROM public.users WHERE id=$1 AND deleted_at IS NULL FOR UPDATE")
        .bind(principal.user.id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?
        .ok_or_else(not_found)?;
    let (used, recent, pending) = stored_bytes(&mut tx, principal.user.id).await?;
    if recent >= CREATES_PER_MINUTE || pending >= MAX_PENDING {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "uploading too quickly; try again shortly",
        ));
    }
    // The original's size is held until the worker reports the stored size.
    if used + input.byte_size > assets.quota_bytes {
        return Err(
            ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "storage limit reached")
                .with_code("storage_full"),
        );
    }
    sqlx::query(
        "INSERT INTO public.assets (external_id,owner_id,purpose,channel_id,declared_content_type,upload_byte_size,kind,content_type,filename,byte_size)
         VALUES ($1,$2,'attachment',$3,$4,$5,$6,$4,$7,$5)",
    )
    .bind(&id)
    .bind(principal.user.id)
    .bind(access.id)
    .bind(&content_type)
    .bind(input.byte_size)
    .bind(kind)
    .bind(&name)
    .execute(&mut *tx)
    .await
    .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;

    let length = input.byte_size.to_string();
    let url = assets
        .incoming
        .presign(
            "PUT",
            &incoming_key(&id),
            UPLOAD_URL_SECONDS,
            &[
                ("content-type", content_type.as_str()),
                ("content-length", length.as_str()),
            ],
        )
        .await?;
    // Clients send only the listed headers; HTTP stacks set Content-Length.
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "kind": kind,
            "upload": {"method": "PUT", "url": url, "headers": {"content-type": content_type}},
            "storage": {"used": used + input.byte_size, "limit": assets.quota_bytes},
        })),
    ))
}

async fn complete(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(asset): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let assets = enabled(&state)?;
    let (row, upload_size): (Row, i64) = {
        let row: Option<Row> = sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM public.assets WHERE external_id=$1 AND owner_id=$2 AND (deleted_at IS NULL OR status='failed')"
        ))
        .bind(&asset)
        .bind(principal.user.id)
        .fetch_optional(&assets.pool)
        .await
        .map_err(database_error)?;
        let row = row.ok_or_else(not_found)?;
        let size: i64 =
            sqlx::query_scalar("SELECT upload_byte_size FROM public.assets WHERE external_id=$1")
                .bind(&asset)
                .fetch_one(&assets.pool)
                .await
                .map_err(database_error)?;
        (row, size)
    };
    if row.status == "failed" {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "file could not be processed",
        ));
    }
    // The worker may already have picked the original up (and deleted it).
    if row.uploaded_at.is_some() {
        return Ok(Json(describe(&row)));
    }
    match assets.incoming.size(&incoming_key(&asset)).await? {
        None => Err(ApiError::new(StatusCode::CONFLICT, "upload not finished")),
        Some(size) if size != upload_size => {
            sqlx::query(
                "UPDATE public.assets SET status='failed', failure='size mismatch', deleted_at=now() WHERE external_id=$1 AND deleted_at IS NULL",
            )
            .bind(&asset)
            .execute(&assets.pool)
            .await
            .map_err(database_error)?;
            Err(ApiError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "file does not match its declared size",
            ))
        }
        Some(_) => {
            sqlx::query("UPDATE public.assets SET uploaded_at=now() WHERE external_id=$1 AND uploaded_at IS NULL AND deleted_at IS NULL")
                .bind(&asset)
                .execute(&assets.pool)
                .await
                .map_err(database_error)?;
            Ok(Json(describe(&row)))
        }
    }
}

async fn usage(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Value>, ApiError> {
    let assets = enabled(&state)?;
    let mut connection = assets.pool.acquire().await.map_err(database_error)?;
    let (used, _, _) = stored_bytes(&mut connection, principal.user.id).await?;
    Ok(Json(
        json!({"used": used, "limit": assets.quota_bytes, "maxUploadBytes": assets.max_upload_bytes}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UrlsInput {
    ids: Vec<String>,
}

/// Fresh URLs for attachments the caller can still see, for long-open clients
/// whose earlier URLs expired (Discord exposes the same kind of refresh).
async fn urls(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<UrlsInput>,
) -> Result<Json<Value>, ApiError> {
    let pool = state.database.as_ref().ok_or_else(unavailable)?;
    let signer = state
        .chat
        .as_ref()
        .and_then(|chat| chat.cdn.clone())
        .ok_or_else(unavailable)?;
    if input.ids.is_empty() || input.ids.len() > 100 {
        return Err(invalid("request 1–100 files"));
    }
    let rows: Vec<(String, String, bool, bool)> = sqlx::query_as(
        "SELECT a.external_id, c.external_id, a.status='ready', a.preview_content_type IS NOT NULL
         FROM public.assets a JOIN public.channels c ON c.id=a.channel_id
         WHERE a.external_id = ANY($1) AND a.message_id IS NOT NULL AND a.deleted_at IS NULL",
    )
    .bind(&input.ids)
    .fetch_all(pool)
    .await
    .map_err(database_error)?;
    let mut allowed: HashMap<String, bool> = HashMap::new();
    let now = Utc::now().timestamp();
    let mut out = Map::new();
    for (asset, channel, ready, preview) in rows {
        let ok = match allowed.get(&channel) {
            Some(ok) => *ok,
            None => {
                let ok = channel_access(pool, &channel, Some(principal.user.id))
                    .await
                    .is_ok();
                allowed.insert(channel, ok);
                ok
            }
        };
        if ok {
            let mut urls = json!({});
            if ready {
                urls["url"] = json!(signer.url(&original_key(&asset), now));
            }
            if preview {
                urls["previewUrl"] = json!(signer.url(&preview_key(&asset), now));
            }
            out.insert(asset, urls);
        }
    }
    Ok(Json(json!({"urls": out})))
}

/// Locks and links uploaded, unattached files to a new message in one channel.
/// Processing need not have finished. Returns attachment descriptions in the
/// requested order.
pub(crate) async fn attach(
    tx: &mut sqlx::PgConnection,
    ids: &[String],
    owner: i64,
    channel_id: i64,
) -> Result<Vec<Value>, ApiError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM public.assets
         WHERE external_id = ANY($1) AND owner_id=$2 AND channel_id=$3 AND purpose='attachment'
           AND uploaded_at IS NOT NULL AND status <> 'failed' AND message_id IS NULL AND deleted_at IS NULL
         FOR UPDATE"
    ))
    .bind(ids)
    .bind(owner)
    .bind(channel_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|_| crate::chat::unavailable())?;
    let by_id: HashMap<&str, &Row> = rows.iter().map(|r| (r.external_id.as_str(), r)).collect();
    ids.iter()
        .map(|id| by_id.get(id.as_str()).map(|row| describe(row)))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::CONFLICT,
                "attachment is not uploaded or already sent",
            )
        })
}

pub(crate) async fn link(
    tx: &mut sqlx::PgConnection,
    ids: &[String],
    message_id: i64,
) -> Result<(), ApiError> {
    for (position, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE public.assets SET message_id=$2, position=$3 WHERE external_id=$1")
            .bind(id)
            .bind(message_id)
            .bind(i16::try_from(position).unwrap_or(i16::MAX))
            .execute(&mut *tx)
            .await
            .map_err(|_| crate::chat::unavailable())?;
    }
    Ok(())
}

/// Marks history attachments whose files were deleted, so clients show a
/// placeholder instead of a broken link before the objects are purged. Files
/// that failed processing already say so through `status`.
pub(crate) async fn mark_deleted(pool: &PgPool, messages: &mut [Value]) -> Result<(), ApiError> {
    let ids: Vec<String> = messages
        .iter()
        .filter_map(|m| m.pointer("/content/attachments").and_then(Value::as_array))
        .flatten()
        .filter_map(|a| a.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let gone: Vec<String> = sqlx::query_scalar(
        "SELECT external_id FROM public.assets WHERE external_id = ANY($1) AND deleted_at IS NOT NULL AND status <> 'failed'",
    )
    .bind(&ids)
    .fetch_all(pool)
    .await
    .map_err(|_| crate::chat::unavailable())?;
    if gone.is_empty() {
        return Ok(());
    }
    for attachment in messages
        .iter_mut()
        .filter_map(|m| {
            m.pointer_mut("/content/attachments")
                .and_then(Value::as_array_mut)
        })
        .flatten()
        .filter_map(Value::as_object_mut)
    {
        if attachment
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| gone.iter().any(|g| g == id))
        {
            attachment.insert("unavailable".into(), Value::Bool(true));
        }
    }
    Ok(())
}

fn worker<'a>(state: &'a AppState, headers: &HeaderMap) -> Result<&'a Assets, ApiError> {
    let assets = enabled(state)?;
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    if bool::from(token.as_bytes().ct_eq(assets.worker_secret.as_bytes())) {
        Ok(assets)
    } else {
        Err(ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized"))
    }
}

/// Claims a file for processing and returns what the worker needs. Repeated
/// claims (SQS redelivery) are allowed until the file finishes.
async fn worker_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(asset): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let assets = worker(&state, &headers)?;
    let row: Option<(i64, String, String, String)> = sqlx::query_as(
        "UPDATE public.assets SET status='processing', uploaded_at=COALESCE(uploaded_at,now()), worker_seen_at=now()
         WHERE external_id=$1 AND deleted_at IS NULL AND status IN ('uploading','processing')
         RETURNING upload_byte_size, declared_content_type, filename, profile",
    )
    .bind(&asset)
    .fetch_optional(&assets.pool)
    .await
    .map_err(database_error)?;
    let (size, declared, name, _profile) = row.ok_or_else(finished)?;
    Ok(Json(json!({
        "id": asset,
        "uploadByteSize": size,
        "declaredContentType": declared,
        "filename": name,
        "settings": assets.settings,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgressInput {
    percent: u8,
}

/// Ephemeral progress for files already in a message, published like typing:
/// no sequence, no storage, droppable.
async fn worker_progress(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(asset): Path<String>,
    Json(input): Json<ProgressInput>,
) -> Result<StatusCode, ApiError> {
    let assets = worker(&state, &headers)?;
    if input.percent > 100 {
        return Err(invalid("invalid progress"));
    }
    let linked: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "UPDATE public.assets a SET worker_seen_at=now()
         FROM public.channels c
         WHERE a.external_id=$1 AND a.status='processing' AND a.deleted_at IS NULL AND c.id=a.channel_id
         RETURNING (SELECT external_id FROM public.messages WHERE id=a.message_id), c.external_id",
    )
    .bind(&asset)
    .fetch_optional(&assets.pool)
    .await
    .map_err(database_error)?;
    let Some((message, channel)) = linked else {
        return Err(finished());
    };
    if let (Some(message), Some(channel), Some(chat)) = (message, channel, state.chat.as_ref()) {
        let event = json!({"type":"attachment.progress","channelId":channel,"messageId":message,"attachmentId":asset,"percent":input.percent});
        // Best effort, like typing: a lost update is replaced by the next one.
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            let mut connection = chat.broker.get_multiplexed_async_connection().await?;
            redis::cmd("PUBLISH")
                .arg(format!("{}:{channel}", chat::TYPING_TOPIC))
                .arg(event.to_string())
                .query_async::<i64>(&mut connection)
                .await
        })
        .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreviewInput {
    content_type: String,
    byte_size: i64,
    width: Option<i32>,
    height: Option<i32>,
    duration_ms: Option<i32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredPreview {
    content_type: String,
    byte_size: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FinishInput {
    kind: String,
    content_type: String,
    filename: String,
    byte_size: i64,
    width: Option<i32>,
    height: Option<i32>,
    duration_ms: Option<i32>,
    #[serde(default)]
    animated: bool,
    content_encoding: Option<String>,
    preview: Option<StoredPreview>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FailInput {
    reason: String,
}

fn preview_type(value: &str) -> Result<String, ApiError> {
    let value = content_type(value)?;
    if matches!(value.as_str(), "image/webp" | "image/jpeg" | "image/png") {
        Ok(value)
    } else {
        Err(invalid("invalid preview"))
    }
}

fn preview_size(size: i64) -> Result<i64, ApiError> {
    if (1..=PREVIEW_MAX_BYTES).contains(&size) {
        Ok(size)
    } else {
        Err(invalid("invalid preview"))
    }
}

enum Update {
    Preview {
        content_type: String,
        byte_size: i64,
        width: Option<i32>,
        height: Option<i32>,
        duration_ms: Option<i32>,
    },
    Finish {
        kind: &'static str,
        content_type: String,
        filename: String,
        byte_size: i64,
        width: Option<i32>,
        height: Option<i32>,
        duration_ms: Option<i32>,
        animated: bool,
        content_encoding: Option<&'static str>,
        preview: Option<(String, i64)>,
    },
    Fail {
        reason: String,
    },
}

async fn worker_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(asset): Path<String>,
    Json(input): Json<PreviewInput>,
) -> Result<StatusCode, ApiError> {
    let assets = worker(&state, &headers)?;
    let update = Update::Preview {
        content_type: preview_type(&input.content_type)?,
        byte_size: preview_size(input.byte_size)?,
        width: optional_range(input.width, MAX_DIMENSION, 1)?,
        height: optional_range(input.height, MAX_DIMENSION, 1)?,
        duration_ms: optional_range(input.duration_ms, MAX_DURATION_MS, 0)?,
    };
    apply(assets, wake(&state), &asset, update).await
}

async fn worker_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(asset): Path<String>,
    Json(input): Json<FinishInput>,
) -> Result<StatusCode, ApiError> {
    let assets = worker(&state, &headers)?;
    let kind = match input.kind.as_str() {
        "image" => "image",
        "video" => "video",
        "audio" => "audio",
        "file" => "file",
        _ => return Err(invalid("invalid kind")),
    };
    let content_encoding = match input.content_encoding.as_deref() {
        None => None,
        Some("gzip") => Some("gzip"),
        Some(_) => return Err(invalid("invalid content encoding")),
    };
    if input.byte_size < 1 {
        return Err(invalid("invalid file size"));
    }
    let preview = match input.preview {
        None => None,
        Some(preview) => Some((
            preview_type(&preview.content_type)?,
            preview_size(preview.byte_size)?,
        )),
    };
    let update = Update::Finish {
        kind,
        content_type: content_type(&input.content_type)?,
        filename: filename(&input.filename)?,
        byte_size: input.byte_size,
        width: optional_range(input.width, MAX_DIMENSION, 1)?,
        height: optional_range(input.height, MAX_DIMENSION, 1)?,
        duration_ms: optional_range(input.duration_ms, MAX_DURATION_MS, 0)?,
        animated: input.animated,
        content_encoding,
        preview,
    };
    apply(assets, wake(&state), &asset, update).await
}

async fn worker_fail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(asset): Path<String>,
    Json(input): Json<FailInput>,
) -> Result<StatusCode, ApiError> {
    let assets = worker(&state, &headers)?;
    let reason: String = input
        .reason
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    apply(assets, wake(&state), &asset, Update::Fail { reason }).await
}

fn wake(state: &AppState) -> Option<Arc<Notify>> {
    state.chat.as_ref().map(|chat| chat.wake.clone())
}

/// Applies one worker update and, when the file is already in a message,
/// rewrites that message's attachments and appends a sequenced
/// `message.attachments` event in the same transaction. Locks follow message
/// creation's order: channel, then asset, then message.
async fn apply(
    assets: &Assets,
    wake: Option<Arc<Notify>>,
    asset: &str,
    update: Update,
) -> Result<StatusCode, ApiError> {
    let channel_id: i64 =
        sqlx::query_scalar("SELECT channel_id FROM public.assets WHERE external_id=$1")
            .bind(asset)
            .fetch_optional(&assets.pool)
            .await
            .map_err(database_error)?
            .ok_or_else(finished)?;
    let mut tx = assets.pool.begin().await.map_err(database_error)?;
    let (channel, head): (String, i64) =
        sqlx::query_as("SELECT external_id,last_seq FROM public.channels WHERE id=$1 FOR UPDATE")
            .bind(channel_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(database_error)?;
    let (status, message_id, deleted): (String, Option<i64>, bool) = sqlx::query_as(
        "SELECT status,message_id,deleted_at IS NOT NULL FROM public.assets WHERE external_id=$1 FOR UPDATE",
    )
    .bind(asset)
    .fetch_one(&mut *tx)
    .await
    .map_err(database_error)?;
    let changed = match update {
        Update::Preview {
            content_type,
            byte_size,
            width,
            height,
            duration_ms,
        } => {
            if deleted || status != "processing" {
                return Err(finished());
            }
            sqlx::query(
                "UPDATE public.assets SET preview_content_type=$2, preview_byte_size=$3,
                    width=COALESCE($4,width), height=COALESCE($5,height), duration_ms=COALESCE($6,duration_ms), worker_seen_at=now()
                 WHERE external_id=$1",
            )
            .bind(asset)
            .bind(content_type)
            .bind(byte_size)
            .bind(width)
            .bind(height)
            .bind(duration_ms)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            true
        }
        Update::Finish {
            kind,
            content_type,
            filename,
            byte_size,
            width,
            height,
            duration_ms,
            animated,
            content_encoding,
            preview,
        } => {
            if status == "ready" {
                false
            } else if deleted || status == "failed" {
                return Err(finished());
            } else {
                sqlx::query(
                    "UPDATE public.assets SET status='ready', kind=$2, content_type=$3, filename=$4, byte_size=$5,
                        width=$6, height=$7, duration_ms=$8, animated=$9, content_encoding=$10,
                        preview_content_type=COALESCE($11,preview_content_type), preview_byte_size=COALESCE($12,preview_byte_size),
                        uploaded_at=COALESCE(uploaded_at,now()), processed_at=now(), worker_seen_at=now()
                     WHERE external_id=$1",
                )
                .bind(asset)
                .bind(kind)
                .bind(content_type)
                .bind(filename)
                .bind(byte_size)
                .bind(width)
                .bind(height)
                .bind(duration_ms)
                .bind(animated)
                .bind(content_encoding)
                .bind(preview.as_ref().map(|(t, _)| t.clone()))
                .bind(preview.as_ref().map(|(_, s)| *s))
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
                true
            }
        }
        Update::Fail { reason } => {
            if status == "failed" {
                false
            } else if status == "ready" {
                return Err(finished());
            } else {
                // Deleting releases the quota once the purge loop removes any
                // stored objects; `status` keeps the failure visible.
                sqlx::query(
                    "UPDATE public.assets SET status='failed', failure=$2, processed_at=now(), deleted_at=COALESCE(deleted_at,now())
                     WHERE external_id=$1",
                )
                .bind(asset)
                .bind(reason)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
                true
            }
        }
    };
    let emitted = match (changed, message_id) {
        (true, Some(message_id)) => {
            refresh_message(&mut tx, channel_id, &channel, head, message_id).await?;
            true
        }
        _ => false,
    };
    tx.commit().await.map_err(database_error)?;
    if emitted && let Some(wake) = wake {
        wake.notify_one();
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn refresh_message(
    tx: &mut sqlx::PgConnection,
    channel_id: i64,
    channel: &str,
    head: i64,
    message_id: i64,
) -> Result<(), ApiError> {
    let (message, mut payload): (String, Value) =
        sqlx::query_as("SELECT external_id,payload FROM public.messages WHERE id=$1 FOR UPDATE")
            .bind(message_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(database_error)?;
    let rows: Vec<Row> = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM public.assets WHERE message_id=$1 ORDER BY position"
    ))
    .bind(message_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(database_error)?;
    let attachments: Vec<Value> = rows.iter().map(describe).collect();
    let seq = head + 1;
    payload["content"]["attachments"] = json!(attachments);
    payload["attachmentsSeq"] = json!(seq.to_string());
    sqlx::query("UPDATE public.messages SET payload=$2 WHERE id=$1")
        .bind(message_id)
        .bind(&payload)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    sqlx::query("INSERT INTO public.channel_events(channel_id,seq,payload) VALUES($1,$2,$3)")
        .bind(channel_id)
        .bind(seq)
        .bind(json!({"type":"message.attachments","schemaVersion":1,"channelId":channel,"seq":seq.to_string(),"messageId":message,"attachments":attachments}))
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    sqlx::query("UPDATE public.channels SET last_seq=$2 WHERE id=$1")
        .bind(channel_id)
        .bind(seq)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    Ok(())
}

/// Abandoned uploads lose their objects immediately; files deleted after being
/// sent wait `PURGE_DELAY_HOURS`. Silent processing jobs fail. Rows remain.
pub(crate) fn spawn_purger(assets: Assets, wake: Option<Arc<Notify>>) {
    tokio::spawn(async move {
        loop {
            if purge(&assets, wake.clone()).await.is_err() {
                tracing::warn!(event_name = "asset_purge_retry", "asset purge will retry");
            }
            tokio::time::sleep(PURGE_INTERVAL).await;
        }
    });
}

pub(crate) async fn purge(assets: &Assets, wake: Option<Arc<Notify>>) -> Result<usize, ()> {
    let stalled: Vec<String> = sqlx::query_scalar(
        "SELECT external_id FROM public.assets
         WHERE deleted_at IS NULL AND status IN ('uploading','processing') AND uploaded_at IS NOT NULL
           AND COALESCE(worker_seen_at,uploaded_at) < now() - make_interval(mins => $1)
         ORDER BY created_at LIMIT 25",
    )
    .bind(WORKER_SILENCE_MINUTES)
    .fetch_all(&assets.pool)
    .await
    .map_err(|_| ())?;
    for asset in stalled {
        let failed = apply(
            assets,
            wake.clone(),
            &asset,
            Update::Fail {
                reason: "processing timed out".into(),
            },
        )
        .await;
        if failed.is_ok() {
            tracing::warn!(
                event_name = "asset_processing_timed_out",
                "media processing timed out"
            );
        }
    }
    sqlx::query(
        "UPDATE public.assets SET deleted_at=now()
         WHERE deleted_at IS NULL AND message_id IS NULL
           AND ((uploaded_at IS NULL AND created_at < now() - interval '1 hour')
             OR uploaded_at < now() - interval '24 hours')",
    )
    .execute(&assets.pool)
    .await
    .map_err(|_| ())?;
    let mut tx = assets.pool.begin().await.map_err(|_| ())?;
    let rows: Vec<(i64, String, bool)> = sqlx::query_as(
        "SELECT id, external_id, preview_content_type IS NOT NULL FROM public.assets
         WHERE deleted_at IS NOT NULL AND purged_at IS NULL
           AND (message_id IS NULL OR status='failed' OR deleted_at < now() - make_interval(hours => $1))
         ORDER BY deleted_at LIMIT 25 FOR UPDATE SKIP LOCKED",
    )
    .bind(PURGE_DELAY_HOURS)
    .fetch_all(&mut *tx)
    .await
    .map_err(|_| ())?;
    let mut purged = 0;
    for (id, external, preview) in rows {
        let mut ok = assets.r2.delete(&original_key(&external)).await.is_ok();
        if preview {
            ok &= assets.r2.delete(&preview_key(&external)).await.is_ok();
        }
        if ok {
            sqlx::query("UPDATE public.assets SET purged_at=now() WHERE id=$1")
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(|_| ())?;
            purged += 1;
        }
    }
    tx.commit().await.map_err(|_| ())?;
    if purged > 0 {
        tracing::info!(
            event_name = "assets_purged",
            count = purged,
            "asset objects purged"
        );
    }
    Ok(purged)
}

#[cfg(test)]
mod tests;
