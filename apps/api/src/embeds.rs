//! Link previews ("embeds"). Sending never waits for them: once the outbox
//! publishes a new or edited message, its links go to the embed Worker
//! (`apps/embed`, on Cloudflare, outside this network) and the cards it returns
//! are stored as `content.embeds` and announced with an unsequenced
//! `message.embeds` event. Replays and history read the stored copy, so a
//! client that missed the event still shows them. See docs/media.md "Link
//! embeds".

use crate::{
    ApiError, AppState, RuntimeEnvironment,
    auth::Principal,
    chat::{Chat, TYPING_TOPIC},
};
use axum::{Extension, Json, Router, extract::State, http::StatusCode, routing::post};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

mod links;

/// Links past this many in one message stay plain links.
pub(crate) const MAX_PER_MESSAGE: usize = 5;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_EMBED_BYTES: usize = 24 * 1024;
const MAX_BLOCKS: usize = 16;
const MAX_STRING: usize = 4096;
/// Unfurls in flight across all messages; the rest wait their turn.
const CONCURRENCY: usize = 16;
/// Composer previews per account per minute.
const PREVIEWS_PER_MINUTE: u32 = 30;

pub(crate) struct Embeds {
    client: reqwest::Client,
    endpoint: String,
    secret: String,
    permits: Semaphore,
    previews: Mutex<HashMap<i64, (Instant, u32)>>,
}

impl Embeds {
    pub(crate) fn from_env(environment: &RuntimeEnvironment) -> Result<Option<Arc<Self>>, String> {
        match (
            environment.get("EMBED_ORIGIN"),
            environment.get("EMBED_API_SECRET"),
        ) {
            (None, None) => Ok(None),
            (Some(origin), Some(secret)) => {
                let origin = origin.trim().trim_end_matches('/');
                let local = origin.starts_with("http://localhost")
                    || origin.starts_with("http://127.0.0.1");
                if !(origin.starts_with("https://") || local) || origin.contains(['?', '#']) {
                    return Err("EMBED_ORIGIN must be an https origin".into());
                }
                if secret.len() < 32 {
                    return Err("EMBED_API_SECRET must contain at least 32 characters".into());
                }
                Ok(Some(Arc::new(Self::new(
                    format!("{origin}/v1/unfurl"),
                    secret,
                ))))
            }
            _ => Err("EMBED_ORIGIN and EMBED_API_SECRET must be set together".into()),
        }
    }

    pub(crate) fn new(endpoint: String, secret: String) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("embed client"),
            endpoint,
            secret,
            permits: Semaphore::new(CONCURRENCY),
            previews: Mutex::new(HashMap::new()),
        }
    }

    /// The Worker's card for one link, or None when it has none, is
    /// unreachable or answers with something that is not a valid card.
    pub(crate) async fn unfurl(&self, url: &str) -> Option<Value> {
        let _permit = self.permits.acquire().await.ok()?;
        let mut response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.secret)
            .json(&json!({ "url": url }))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            body.extend_from_slice(&chunk);
            if body.len() > MAX_RESPONSE_BYTES {
                return None;
            }
        }
        let mut body: Value = serde_json::from_slice(&body).ok()?;
        let embed = body.get_mut("embed")?.take();
        valid(&embed, url).then_some(embed)
    }

    fn allow_preview(&self, user: i64) -> bool {
        let mut previews = self.previews.lock().unwrap();
        let now = Instant::now();
        previews.retain(|_, (start, _)| now.duration_since(*start) < Duration::from_secs(60));
        let (_, count) = previews.entry(user).or_insert((now, 0));
        *count += 1;
        *count <= PREVIEWS_PER_MINUTE
    }
}

/// The links in `text` that get a preview: http(s) links as every client
/// detects them, in order, without repeats, skipping any written `<like this>`.
pub(crate) fn links(text: &str) -> Vec<String> {
    let segments = links::segments(text);
    let mut found: Vec<String> = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        let Some(href) = &segment.href else { continue };
        let before = index
            .checked_sub(1)
            .is_some_and(|previous| segments[previous].text.ends_with('<'));
        let after = segments
            .get(index + 1)
            .is_some_and(|next| next.text.starts_with('>'));
        let bracketed = before && after;
        if !bracketed && !found.contains(href) {
            found.push(href.clone());
        }
        if found.len() == MAX_PER_MESSAGE {
            break;
        }
    }
    found
}

/// A card is plain JSON in a fixed shape: the posted URL, a site, and a short
/// list of typed blocks whose links are http(s). Clients draw only the block
/// types they know, but nothing oversized or scriptable is stored.
fn valid(embed: &Value, url: &str) -> bool {
    fn bounded(value: &Value, depth: usize) -> bool {
        match value {
            Value::String(text) => text.len() <= MAX_STRING,
            Value::Array(items) => depth < 6 && items.iter().all(|item| bounded(item, depth + 1)),
            Value::Object(fields) => {
                depth < 6
                    && fields.iter().all(|(key, value)| {
                        let link = matches!(key.as_str(), "url" | "icon" | "poster");
                        (!link
                            || value.as_str().is_some_and(|value| {
                                value.starts_with("https://") || value.starts_with("http://")
                            }))
                            && bounded(value, depth + 1)
                    })
            }
            Value::Number(_) | Value::Bool(_) | Value::Null => true,
        }
    }
    let blocks = embed["blocks"].as_array();
    embed["version"] == 1
        && embed["url"] == url
        && embed["site"]["name"]
            .as_str()
            .is_some_and(|name| !name.is_empty() && name.len() <= 256)
        && embed["site"]["domain"]
            .as_str()
            .is_some_and(|domain| !domain.is_empty() && domain.len() <= 256)
        && blocks.is_some_and(|blocks| {
            !blocks.is_empty()
                && blocks.len() <= MAX_BLOCKS
                && blocks
                    .iter()
                    .all(|block| block["type"].as_str().is_some_and(|kind| kind.len() <= 32))
        })
        && bounded(embed, 0)
        && serde_json::to_vec(embed).is_ok_and(|bytes| bytes.len() <= MAX_EMBED_BYTES)
}

/// An edit keeps the cards for links it still has, so they do not flicker
/// while the new text's links are unfurled.
pub(crate) fn carry_over(previous: &Value, content: &mut Value) {
    let Some(embeds) = previous["embeds"].as_array() else {
        return;
    };
    let kept: Vec<Value> = {
        let current = links(content["text"].as_str().unwrap_or_default());
        embeds
            .iter()
            .filter(|embed| {
                embed["url"]
                    .as_str()
                    .is_some_and(|url| current.iter().any(|link| link == url))
            })
            .cloned()
            .collect()
    };
    if !kept.is_empty() {
        content["embeds"] = Value::Array(kept);
    }
}

/// A published message whose links need cards.
pub(crate) struct Job {
    channel: String,
    message: String,
    revision: i64,
    /// The outbox event that carries this revision, refreshed for replays.
    seq: i64,
    links: Vec<String>,
}

impl Job {
    /// From an outbox event; None for anything that is not a live message
    /// with links (or, after an edit, cards that may need removing).
    pub(crate) fn from_event(event: &Value) -> Option<Self> {
        let created = match event["type"].as_str() {
            Some("message.created") => true,
            Some("message.edited") => false,
            _ => return None,
        };
        let message = &event["message"];
        if message.get("deletedAt").is_some() || message.get("forward").is_some() {
            return None;
        }
        let links = links(message["content"]["text"].as_str().unwrap_or_default());
        let had = message["content"]["embeds"]
            .as_array()
            .is_some_and(|cards| !cards.is_empty());
        if links.is_empty() && (created || !had) {
            return None;
        }
        Some(Self {
            channel: event["channelId"].as_str()?.to_owned(),
            message: message["id"].as_str()?.to_owned(),
            revision: message["revision"].as_i64().unwrap_or(1),
            seq: event["seq"].as_str()?.parse().ok()?,
            links,
        })
    }
}

pub(crate) fn spawn(chat: &Chat, jobs: Vec<Job>) {
    let Some(embeds) = chat.embeds.clone() else {
        return;
    };
    for job in jobs {
        let (chat, embeds) = (chat.clone(), embeds.clone());
        tokio::spawn(async move {
            let cards =
                futures_util::future::join_all(job.links.iter().map(|url| embeds.unfurl(url)))
                    .await
                    .into_iter()
                    .flatten()
                    .collect();
            if let Err(error) = apply(&chat, &job, cards).await {
                tracing::warn!(event_name = "embeds_apply_failed", %error, "link previews not stored");
            }
        });
    }
}

/// Stores the cards on the message (if it is still at the revision they were
/// made for) and on the outbox event replays read, then announces them.
pub(crate) async fn apply(chat: &Chat, job: &Job, cards: Vec<Value>) -> Result<bool, sqlx::Error> {
    let mut tx = chat.pool.begin().await?;
    let row: Option<(i64, i64, Value)> = sqlx::query_as(
        "SELECT m.id,m.channel_id,m.payload FROM public.messages m JOIN public.channels c ON c.id=m.channel_id
         WHERE c.external_id=$1 AND m.external_id=$2 AND m.deleted_at IS NULL FOR UPDATE OF m",
    )
    .bind(&job.channel)
    .bind(&job.message)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((message_id, channel_id, mut payload)) = row else {
        return Ok(false);
    };
    // A newer edit unfurls its own links.
    if payload["revision"].as_i64().unwrap_or(1) != job.revision {
        return Ok(false);
    }
    let current = payload["content"]["embeds"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if current == cards {
        return Ok(false);
    }
    let content = payload["content"].as_object_mut().expect("message content");
    if cards.is_empty() {
        content.remove("embeds");
    } else {
        content.insert("embeds".into(), Value::Array(cards.clone()));
    }
    sqlx::query("UPDATE public.messages SET payload=$2 WHERE id=$1")
        .bind(message_id)
        .bind(&payload)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE public.channel_events SET payload=jsonb_set(payload,'{message,content}',$3)
         WHERE channel_id=$1 AND seq=$2 AND payload->'message'->>'id'=$4",
    )
    .bind(channel_id)
    .bind(job.seq)
    .bind(&payload["content"])
    .bind(&job.message)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    chat.publish(
        &format!("{TYPING_TOPIC}:{}", job.channel),
        &json!({"type":"message.embeds","schemaVersion":1,"channelId":job.channel,"messageId":job.message,"revision":job.revision,"embeds":cards}),
    )
    .await;
    Ok(true)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewInput {
    url: String,
}

/// The card a link will get, for the composer to show before sending.
async fn preview(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<PreviewInput>,
) -> Result<Json<Value>, ApiError> {
    let embeds = state
        .chat
        .as_ref()
        .and_then(|chat| chat.embeds.clone())
        .ok_or_else(|| {
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "link previews unavailable")
        })?;
    let url = links(&input.url)
        .into_iter()
        .next()
        .filter(|url| url.len() <= 2048);
    let Some(url) = url else {
        return Ok(Json(json!({ "embed": null })));
    };
    if !embeds.allow_preview(principal.user.id) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "previewing too quickly; try again shortly",
        ));
    }
    Ok(Json(json!({ "embed": embeds.unfurl(&url).await })))
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/api/embeds/preview", post(preview))
}

#[cfg(test)]
mod tests;
