//! Uploaded files. Bytes go straight between clients and R2: the API only
//! reserves quota, presigns one exact upload, verifies the stored object, and
//! signs short-lived delivery URLs that the CDN Worker checks.
use crate::{
    ApiError, AppState, RuntimeEnvironment, auth::Principal, auth::random_id,
    spaces::channel_access,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::StatusCode,
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

const DEFAULT_QUOTA_BYTES: i64 = 1024 * 1024 * 1024;
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

/// Signature check of the stored bytes for inline types, so a renamed HTML or
/// SVG file can never be served as an image.
pub(crate) fn sniff(content_type: &str, bytes: &[u8]) -> bool {
    let at = |offset: usize, magic: &[u8]| bytes.get(offset..offset + magic.len()) == Some(magic);
    let ftyp = at(4, b"ftyp");
    match content_type {
        "image/png" => at(0, b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => at(0, b"\xff\xd8\xff"),
        "image/gif" => at(0, b"GIF87a") || at(0, b"GIF89a"),
        "image/webp" => at(0, b"RIFF") && at(8, b"WEBP"),
        "image/avif" => {
            ftyp && [b"avif", b"avis", b"mif1", b"msf1"]
                .iter()
                .any(|b| at(8, *b))
        }
        "video/mp4" | "audio/mp4" | "audio/x-m4a" => ftyp,
        "video/quicktime" => {
            ftyp || [b"moov", b"mdat", b"wide", b"free"]
                .iter()
                .any(|b| at(4, *b))
        }
        "video/webm" | "audio/webm" => at(0, b"\x1a\x45\xdf\xa3"),
        "audio/mpeg" | "audio/aac" => {
            at(0, b"ID3") || (bytes.len() > 1 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0)
        }
        "audio/ogg" => at(0, b"OggS"),
        "audio/wav" | "audio/x-wav" => at(0, b"RIFF") && at(8, b"WAVE"),
        "audio/flac" => at(0, b"fLaC"),
        _ => true,
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

/// AWS Signature Version 4 query presigning (S3 dialect, unsigned payload).
/// `headers` are signed in addition to `host`, so the client must send them
/// byte-for-byte; R2 rejects any other Content-Type or Content-Length.
#[allow(clippy::too_many_arguments)]
fn presign_v4(
    method: &str,
    scheme_host: &str,
    path: &str,
    access_key: &str,
    secret_key: &str,
    region: &str,
    now: DateTime<Utc>,
    expires: u64,
    headers: &[(&str, &str)],
) -> String {
    let host = scheme_host
        .split_once("://")
        .map_or(scheme_host, |(_, host)| host);
    let date = now.format("%Y%m%d").to_string();
    let stamp = now.format("%Y%m%dT%H%M%SZ").to_string();
    let scope = format!("{date}/{region}/s3/aws4_request");
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
        ("X-Amz-Credential", format!("{access_key}/{scope}")),
        ("X-Amz-Date", stamp.clone()),
        ("X-Amz-Expires", expires.to_string()),
        ("X-Amz-SignedHeaders", signed_names.clone()),
    ]
    .into_iter()
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
    let key = [region.as_bytes(), b"s3", b"aws4_request"].iter().fold(
        hmac(format!("AWS4{secret_key}").as_bytes(), date.as_bytes()),
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
                Ok(Some(Self::new(
                    endpoint,
                    bucket,
                    access_key,
                    secret_key,
                )))
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
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("static HTTP client configuration"),
        }
    }

    fn presign(&self, method: &str, key: &str, expires: u64, headers: &[(&str, &str)]) -> String {
        presign_v4(
            method,
            &self.endpoint,
            &format!("/{}/{key}", self.bucket),
            &self.access_key,
            &self.secret_key,
            "auto",
            Utc::now(),
            expires,
            headers,
        )
    }

    /// Stored size, or `None` when the object does not exist.
    async fn size(&self, key: &str) -> Result<Option<i64>, ApiError> {
        let response = self
            .http
            .head(self.presign("HEAD", key, 60, &[]))
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

    async fn head_bytes(&self, key: &str) -> Result<Vec<u8>, ApiError> {
        let response = self
            .http
            .get(self.presign("GET", key, 60, &[]))
            .header("range", "bytes=0-63")
            .send()
            .await
            .map_err(|_| unavailable())?;
        if !response.status().is_success() {
            return Err(unavailable());
        }
        let bytes = response.bytes().await.map_err(|_| unavailable())?;
        Ok(bytes.iter().take(64).copied().collect())
    }

    async fn delete(&self, key: &str) -> Result<(), ()> {
        let response = self
            .http
            .delete(self.presign("DELETE", key, 60, &[]))
            .send()
            .await
            .map_err(|_| ())?;
        (response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND)
            .then_some(())
            .ok_or(())
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

/// Adds fresh delivery URLs to a message or `message.created` event. URLs are
/// never stored: every response and socket frame signs at the moment it leaves.
pub(crate) fn sign_attachments(mut payload: Value, signer: Option<&CdnSigner>) -> Value {
    let Some(signer) = signer else {
        return payload;
    };
    let now = Utc::now().timestamp();
    let pointer = if payload.get("message").is_some() {
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
    attachment.insert("url".into(), json!(signer.url(&original_key(&id), now)));
    if attachment.get("preview").is_some_and(Value::is_object) {
        attachment.insert(
            "previewUrl".into(),
            json!(signer.url(&preview_key(&id), now)),
        );
    }
}

#[derive(Clone)]
pub(crate) struct Assets {
    pool: PgPool,
    r2: R2,
    quota_bytes: i64,
}

impl Assets {
    pub(crate) fn from_env(
        pool: Option<&PgPool>,
        environment: &RuntimeEnvironment,
    ) -> Result<Option<Self>, String> {
        let Some(r2) = R2::from_env(environment)? else {
            return Ok(None);
        };
        let pool = pool.ok_or("uploads require DATABASE_URL")?.clone();
        let quota_bytes = match environment.get("ASSET_QUOTA_BYTES") {
            None => DEFAULT_QUOTA_BYTES,
            Some(value) => value
                .trim()
                .parse::<i64>()
                .ok()
                .filter(|v| *v > 0)
                .ok_or("ASSET_QUOTA_BYTES must be a positive integer")?,
        };
        Ok(Some(Self::new(pool, r2, quota_bytes)))
    }

    pub(crate) fn new(pool: PgPool, r2: R2, quota_bytes: i64) -> Self {
        Self {
            pool,
            r2,
            quota_bytes,
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

fn enabled(state: &AppState) -> Result<&Assets, ApiError> {
    state.assets.as_ref().ok_or_else(unavailable)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreviewInput {
    content_type: String,
    byte_size: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateInput {
    channel_id: String,
    filename: String,
    content_type: String,
    byte_size: i64,
    source_byte_size: Option<i64>,
    width: Option<i32>,
    height: Option<i32>,
    duration_ms: Option<i32>,
    preview: Option<PreviewInput>,
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

/// RFC 6266 download name. R2 stores it, and the Worker only overrides it with
/// `inline` for allowlisted media types.
fn disposition(name: &str) -> String {
    let fallback: String = name
        .chars()
        .map(|c| match c {
            ' '..='~' if c != '"' && c != '\\' => c,
            _ => '_',
        })
        .collect();
    format!(
        "attachment; filename=\"{fallback}\"; filename*=UTF-8''{}",
        uri_encode(name, true)
    )
}

fn optional_range(value: Option<i32>, max: i32, min: i32) -> Result<Option<i32>, ApiError> {
    match value {
        Some(v) if !(min..=max).contains(&v) => Err(invalid("invalid media dimensions")),
        v => Ok(v),
    }
}

fn upload(url: String, headers: &[(&str, &str)]) -> Value {
    let headers: Map<String, Value> = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), json!(value)))
        .collect();
    json!({"method":"PUT","url":url,"headers":headers})
}

#[derive(sqlx::FromRow)]
struct Row {
    external_id: String,
    kind: String,
    content_type: String,
    filename: String,
    byte_size: i64,
    width: Option<i32>,
    height: Option<i32>,
    duration_ms: Option<i32>,
    preview_content_type: Option<String>,
    preview_byte_size: Option<i64>,
    uploaded_at: Option<DateTime<Utc>>,
}
const COLUMNS: &str = "external_id,kind,content_type,filename,byte_size,width,height,duration_ms,preview_content_type,preview_byte_size,uploaded_at";

/// The attachment shape embedded in message content (without URLs).
fn describe(row: &Row) -> Value {
    let mut value = json!({
        "id": row.external_id,
        "kind": row.kind,
        "contentType": row.content_type,
        "name": row.filename,
        "size": row.byte_size,
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
    if row.preview_content_type.is_some() {
        value["preview"] = json!({});
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
    if input.byte_size < 1 || input.source_byte_size.is_some_and(|v| v < 1) {
        return Err(invalid("invalid file size"));
    }
    let width = optional_range(input.width, MAX_DIMENSION, 1)?;
    let height = optional_range(input.height, MAX_DIMENSION, 1)?;
    let duration = optional_range(input.duration_ms, MAX_DURATION_MS, 0)?;
    let preview = match &input.preview {
        None => None,
        Some(preview) => {
            let preview_type = content_type_for_preview(&preview.content_type)?;
            if !matches!(kind, "image" | "video")
                || !(1..=PREVIEW_MAX_BYTES).contains(&preview.byte_size)
            {
                return Err(invalid("invalid preview"));
            }
            Some((preview_type, preview.byte_size))
        }
    };
    let total = input.byte_size + preview.as_ref().map_or(0, |(_, size)| *size);
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
    if used + total > assets.quota_bytes {
        return Err(
            ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "storage limit reached")
                .with_code("storage_full"),
        );
    }
    sqlx::query(
        "INSERT INTO public.assets (external_id,owner_id,purpose,channel_id,kind,content_type,filename,byte_size,source_byte_size,width,height,duration_ms,preview_content_type,preview_byte_size)
         VALUES ($1,$2,'attachment',$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
    )
    .bind(&id)
    .bind(principal.user.id)
    .bind(access.id)
    .bind(kind)
    .bind(&content_type)
    .bind(&name)
    .bind(input.byte_size)
    .bind(input.source_byte_size)
    .bind(width)
    .bind(height)
    .bind(duration)
    .bind(preview.as_ref().map(|(t, _)| t.as_str()))
    .bind(preview.as_ref().map(|(_, size)| *size))
    .execute(&mut *tx)
    .await
    .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;

    let length = input.byte_size.to_string();
    let disposition = disposition(&name);
    let headers = [
        ("content-type", content_type.as_str()),
        ("content-length", length.as_str()),
        ("content-disposition", disposition.as_str()),
    ];
    let mut body = json!({
        "id": id,
        "kind": kind,
        "upload": upload(assets.r2.presign("PUT", &original_key(&id), UPLOAD_URL_SECONDS, &headers), &headers[..1]),
        "storage": {"used": used + total, "limit": assets.quota_bytes},
    });
    // Browsers set Content-Length themselves; clients send only the listed headers.
    body["upload"]["headers"]["content-disposition"] = json!(disposition);
    if let Some((preview_type, preview_size)) = &preview {
        let length = preview_size.to_string();
        let headers = [
            ("content-type", preview_type.as_str()),
            ("content-length", length.as_str()),
        ];
        body["previewUpload"] = upload(
            assets
                .r2
                .presign("PUT", &preview_key(&id), UPLOAD_URL_SECONDS, &headers),
            &headers[..1],
        );
    }
    Ok((StatusCode::CREATED, Json(body)))
}

fn content_type_for_preview(value: &str) -> Result<String, ApiError> {
    let value = content_type(value)?;
    if matches!(value.as_str(), "image/webp" | "image/jpeg" | "image/png") {
        Ok(value)
    } else {
        Err(invalid("invalid preview"))
    }
}

async fn complete(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(asset): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let assets = enabled(&state)?;
    let row: Row = sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM public.assets WHERE external_id=$1 AND owner_id=$2 AND deleted_at IS NULL"
    ))
    .bind(&asset)
    .bind(principal.user.id)
    .fetch_optional(&assets.pool)
    .await
    .map_err(database_error)?
    .ok_or_else(not_found)?;
    if row.uploaded_at.is_some() {
        return Ok(Json(describe(&row)));
    }
    let mut checks = vec![(
        original_key(&asset),
        row.byte_size,
        row.content_type.clone(),
    )];
    if let (Some(t), Some(size)) = (&row.preview_content_type, row.preview_byte_size) {
        checks.push((preview_key(&asset), size, t.clone()));
    }
    for (key, expected, content_type) in &checks {
        match assets.r2.size(key).await? {
            None => {
                return Err(ApiError::new(StatusCode::CONFLICT, "upload not finished"));
            }
            Some(size) if size != *expected => return reject(assets, &asset).await,
            Some(_) => {}
        }
        if kind(content_type) != "file" && !sniff(content_type, &assets.r2.head_bytes(key).await?) {
            return reject(assets, &asset).await;
        }
    }
    sqlx::query("UPDATE public.assets SET uploaded_at=now() WHERE external_id=$1 AND uploaded_at IS NULL AND deleted_at IS NULL")
        .bind(&asset)
        .execute(&assets.pool)
        .await
        .map_err(database_error)?;
    Ok(Json(describe(&row)))
}

/// Refuse an upload whose bytes do not match what was reserved. The purge loop
/// removes its objects and releases the quota.
async fn reject(assets: &Assets, asset: &str) -> Result<Json<Value>, ApiError> {
    sqlx::query(
        "UPDATE public.assets SET deleted_at=now() WHERE external_id=$1 AND deleted_at IS NULL",
    )
    .bind(asset)
    .execute(&assets.pool)
    .await
    .map_err(database_error)?;
    Err(ApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "file does not match its declared size or type",
    ))
}

async fn usage(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Value>, ApiError> {
    let assets = enabled(&state)?;
    let mut connection = assets.pool.acquire().await.map_err(database_error)?;
    let (used, _, _) = stored_bytes(&mut connection, principal.user.id).await?;
    Ok(Json(json!({"used": used, "limit": assets.quota_bytes})))
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
    let rows: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT a.external_id, c.external_id, a.preview_content_type IS NOT NULL
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
    for (asset, channel, preview) in rows {
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
            let mut urls = json!({"url": signer.url(&original_key(&asset), now)});
            if preview {
                urls["previewUrl"] = json!(signer.url(&preview_key(&asset), now));
            }
            out.insert(asset, urls);
        }
    }
    Ok(Json(json!({"urls": out})))
}

/// Locks and links ready, unattached uploads to a new message in one channel.
/// Returns attachment descriptions in the requested order.
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
           AND uploaded_at IS NOT NULL AND message_id IS NULL AND deleted_at IS NULL
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
                "attachment is not ready or already sent",
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
/// placeholder instead of a broken link before the objects are purged.
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
        "SELECT external_id FROM public.assets WHERE external_id = ANY($1) AND deleted_at IS NOT NULL",
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

/// Abandoned and rejected uploads lose their objects immediately; files
/// deleted after being sent wait `PURGE_DELAY_HOURS`. Rows remain.
pub(crate) fn spawn_purger(assets: Assets) {
    tokio::spawn(async move {
        loop {
            if purge(&assets).await.is_err() {
                tracing::warn!(event_name = "asset_purge_retry", "asset purge will retry");
            }
            tokio::time::sleep(PURGE_INTERVAL).await;
        }
    });
}

pub(crate) async fn purge(assets: &Assets) -> Result<usize, ()> {
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
           AND (message_id IS NULL OR deleted_at < now() - make_interval(hours => $1))
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
