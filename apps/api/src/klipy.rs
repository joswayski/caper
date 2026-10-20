//! KLIPY GIFs and stickers. KLIPY's terms require the search, trending and
//! media requests to come straight from each person's device, so the API never
//! calls KLIPY: it hands signed-in clients the app key and a stable,
//! unlinkable customer id, and validates the GIF a message carries.
//!
//! A message's `content.gif` is
//! `{provider:"klipy", kind:"gif"|"sticker", slug, title, width, height,
//! gif?, webp?, mp4?, still?}`, holding KLIPY's media URLs unchanged so
//! everyone loads them from KLIPY. At least one of `gif` and `webp` is present.
use crate::{ApiError, AppState, RuntimeEnvironment, auth::Principal};
use axum::{Extension, Json, Router, extract::State, http::StatusCode, routing::get};
use hmac::{Hmac, Mac};
use reqwest::Url;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Sha256;
use std::{fmt::Write, sync::Arc};

const MAX_URL: usize = 2048;
const MAX_TEXT: usize = 200;
const MAX_EDGE: u32 = 4096;

#[derive(Clone, Default)]
pub(crate) struct Klipy {
    enabled: Option<Arc<Enabled>>,
}

struct Enabled {
    app_key: String,
    customer_secret: Vec<u8>,
}

impl Klipy {
    /// Needs `KLIPY_APP_KEY` and accounts (`AUTH_SECRET` keys the customer ids).
    pub(crate) fn from_env(environment: &RuntimeEnvironment) -> Result<Self, String> {
        let Some(app_key) = environment
            .get("KLIPY_APP_KEY")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
        else {
            tracing::info!("KLIPY_APP_KEY unset; GIFs remain hidden");
            return Ok(Self::default());
        };
        // A mistyped key hides GIFs rather than stopping the API.
        if app_key.len() > 200
            || !app_key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            tracing::error!(
                event_name = "klipy_disabled",
                "KLIPY_APP_KEY must be 1-200 letters, digits, - or _; GIFs remain hidden"
            );
            return Ok(Self::default());
        }
        let Some(secret) = environment
            .get("AUTH_SECRET")
            .filter(|value| !value.trim().is_empty())
        else {
            tracing::info!("AUTH_SECRET unset; GIFs remain hidden");
            return Ok(Self::default());
        };
        Ok(Self::enabled(app_key, secret.as_bytes()))
    }

    fn enabled(app_key: String, secret: &[u8]) -> Self {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
        mac.update(b"caper-klipy-customer-key");
        Self {
            enabled: Some(Arc::new(Enabled {
                app_key,
                customer_secret: mac.finalize().into_bytes().to_vec(),
            })),
        }
    }

    #[cfg(test)]
    pub(crate) fn test(app_key: &str) -> Self {
        Self::enabled(app_key.into(), b"test-secret-test-secret-test-secret")
    }
}

impl Enabled {
    /// Stable per account, and meaningless without our secret, so KLIPY can
    /// personalize recents without learning who anyone is on Caper.
    fn customer_id(&self, account: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.customer_secret)
            .expect("HMAC accepts any key length");
        mac.update(account.as_bytes());
        mac.finalize().into_bytes()[..16]
            .iter()
            .fold(String::new(), |mut out, b| {
                let _ = write!(out, "{b:02x}");
                out
            })
    }
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/api/klipy", get(config))
}

/// The key is KLIPY's client key: it ships to devices by design, like a
/// publishable key, and is only handed to signed-in accounts.
async fn config(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Value>, ApiError> {
    let enabled = state
        .klipy
        .enabled
        .as_ref()
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "gifs unavailable"))?;
    Ok(Json(json!({
        "appKey": enabled.app_key,
        "customerId": enabled.customer_id(&principal.user.external_id),
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GifInput {
    provider: String,
    kind: String,
    slug: String,
    #[serde(default)]
    title: String,
    width: u32,
    height: u32,
    gif: Option<String>,
    webp: Option<String>,
    mp4: Option<String>,
    still: Option<String>,
}

fn invalid() -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid gif")
}

/// KLIPY media only: HTTPS on klipy.com or its subdomains, no credentials or port.
pub(crate) fn media_url(value: &str) -> bool {
    value.len() <= MAX_URL
        && Url::parse(value).is_ok_and(|url| {
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.port().is_none()
                && url.fragment().is_none()
                && url
                    .host_str()
                    .is_some_and(|host| host == "klipy.com" || host.ends_with(".klipy.com"))
        })
}

fn text(value: &str, min: usize) -> bool {
    (min..=MAX_TEXT).contains(&value.chars().count()) && !value.chars().any(char::is_control)
}

/// The stored `content.gif`, or 400 when anything is off.
pub(crate) fn prepare(input: &GifInput) -> Result<Value, ApiError> {
    if input.provider != "klipy"
        || !matches!(input.kind.as_str(), "gif" | "sticker")
        || !text(&input.slug, 1)
        || !input
            .slug
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        || !text(&input.title, 0)
        || !(1..=MAX_EDGE).contains(&input.width)
        || !(1..=MAX_EDGE).contains(&input.height)
        || (input.gif.is_none() && input.webp.is_none())
    {
        return Err(invalid());
    }
    let mut gif = json!({
        "provider": "klipy",
        "kind": input.kind,
        "slug": input.slug,
        "title": input.title,
        "width": input.width,
        "height": input.height,
    });
    for (key, value) in [
        ("gif", &input.gif),
        ("webp", &input.webp),
        ("mp4", &input.mp4),
        ("still", &input.still),
    ] {
        if let Some(value) = value {
            if !media_url(value) {
                return Err(invalid());
            }
            gif[key] = json!(value);
        }
    }
    Ok(gif)
}

/// What a reply quote or notification says for a message that is only a GIF.
pub(crate) fn label(gif: &Value) -> &'static str {
    if gif["kind"] == "sticker" {
        "Sticker"
    } else {
        "GIF"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(value: Value) -> Result<Value, ApiError> {
        prepare(&serde_json::from_value(value).expect("gif input"))
    }

    fn valid() -> Value {
        json!({"provider":"klipy","kind":"gif","slug":"hello-hi-662","title":"Hello","width":498,"height":498,
            "gif":"https://static.klipy.com/ii/935d/14/af/8GCrVAB7.gif","webp":"https://static1.klipy.com/ii/935d/14/af/JUYsGsrc.webp",
            "mp4":"https://static.klipy.com/ii/935d/14/af/V6da8Awi.mp4","still":"https://static.klipy.com/ii/935d/14/af/UsX8Vqtm.jpg"})
    }

    #[test]
    fn keeps_klipy_urls_unchanged() {
        let mut url = valid();
        url["gif"] = json!("https://static.klipy.com/ii/a.gif?token=x&cid=1");
        let stored = input(url.clone()).unwrap();
        assert_eq!(stored["gif"], url["gif"]);
        assert_eq!(stored["kind"], "gif");
        assert_eq!(input(valid()).unwrap(), valid());
    }

    #[test]
    fn rejects_other_hosts_and_shapes() {
        for (key, bad) in [
            ("gif", json!("http://static.klipy.com/a.gif")),
            ("gif", json!("https://klipy.com.evil.example/a.gif")),
            ("gif", json!("https://evilklipy.com/a.gif")),
            ("gif", json!("https://user@static.klipy.com/a.gif")),
            ("gif", json!("https://static.klipy.com:8443/a.gif")),
            ("webp", json!("javascript:alert(1)")),
            (
                "mp4",
                json!(format!("https://static.klipy.com/{}", "a".repeat(2048))),
            ),
            ("provider", json!("giphy")),
            ("kind", json!("clip")),
            ("slug", json!("")),
            ("slug", json!("../x")),
            ("title", json!("a\u{0}b")),
            ("title", json!("a".repeat(201))),
            ("width", json!(0)),
            ("height", json!(4097)),
        ] {
            let mut value = valid();
            value[key] = bad;
            assert!(input(value).is_err(), "{key}");
        }
        let mut neither = valid();
        neither.as_object_mut().unwrap().remove("gif");
        neither.as_object_mut().unwrap().remove("webp");
        assert!(input(neither).is_err());
        let mut extra = valid();
        extra["html"] = json!("<script>");
        assert!(serde_json::from_value::<GifInput>(extra).is_err());
    }

    #[test]
    fn matches_the_shared_client_rules() {
        let cases: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../shared/klipy/cases.json"
        )))
        .unwrap();
        for case in cases["urls"].as_array().unwrap() {
            let url = case["url"].as_str().unwrap();
            assert_eq!(media_url(url), case["valid"] == true, "{url}");
        }
        let accepted = |value: &Value| {
            serde_json::from_value::<GifInput>(value.clone())
                .ok()
                .and_then(|input| prepare(&input).ok())
        };
        for valid in cases["messages"]["valid"].as_array().unwrap() {
            assert_eq!(accepted(valid).as_ref(), Some(valid), "{valid}");
        }
        for invalid in cases["messages"]["invalid"].as_array().unwrap() {
            assert!(accepted(invalid).is_none(), "{invalid}");
        }
        // Everything the picker would send is accepted unchanged.
        for parse in [&cases["parse"], &cases["parseSticker"]] {
            for item in parse["items"].as_array().unwrap() {
                if item["type"] == "item" {
                    assert_eq!(accepted(&item["gif"]).as_ref(), Some(&item["gif"]));
                }
            }
        }
    }

    #[test]
    fn customer_ids_are_stable_and_distinct() {
        let klipy = Klipy::test("key");
        let enabled = klipy.enabled.as_ref().unwrap();
        assert_eq!(enabled.customer_id("abc"), enabled.customer_id("abc"));
        assert_ne!(enabled.customer_id("abc"), enabled.customer_id("abd"));
        assert_eq!(enabled.customer_id("abc").len(), 32);
        assert!(!enabled.customer_id("abc").contains("abc"));
    }

    #[test]
    fn labels_stickers() {
        assert_eq!(label(&json!({"kind":"sticker"})), "Sticker");
        assert_eq!(label(&json!({"kind":"gif"})), "GIF");
    }
}
