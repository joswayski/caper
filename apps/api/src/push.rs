//! Phone push (APNs and FCM) for invitations, DMs and channel messages, and the account,
//! space, channel and DM notification preferences that govern it.
//!
//! A committed account-authored message enqueues one `notification_jobs` row
//! in its send transaction. With `NOTIFICATIONS_ENABLED=true`, workers in the
//! API role expand each job into one notification per recipient and one
//! delivery per registered device, then send deliveries without holding a
//! database connection. Never log tokens, key material or message text.
use crate::{
    ApiError, AppState, RuntimeEnvironment, assets::CdnSigner, auth::Principal, chat::Chat,
    presence::Presence,
};
use axum::{
    Extension, Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use tokio::sync::Notify;

mod apns;
mod credentials;
mod delivery;
mod expansion;
mod fcm;
pub(crate) mod live;
mod retraction;
mod settings;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Platform {
    Apns,
    ApnsSandbox,
    Fcm,
}

impl Platform {
    const ALL: [Self; 3] = [Self::Apns, Self::ApnsSandbox, Self::Fcm];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Apns => "apns",
            Self::ApnsSandbox => "apnsSandbox",
            Self::Fcm => "fcm",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|platform| platform.name() == value)
    }
}

/// Account default and space/channel override levels. DMs use only `Nothing`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
    All,
    Mentions,
    Nothing,
}

impl Level {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "all" => Some(Self::All),
            "mentions" => Some(Self::Mentions),
            "nothing" => Some(Self::Nothing),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Mentions => "mentions",
            Self::Nothing => "nothing",
        }
    }
}

/// What a provider made of one delivery attempt.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Delivered,
    /// 429, 5xx, an expired provider token or a network failure. `after` is
    /// the provider's `Retry-After`, when it sent one.
    Retry {
        error: String,
        after: Option<Duration>,
    },
    /// The provider says the token is dead: revoke the device.
    Revoke(String),
    /// Retrying would not help.
    Abandon(String),
}

/// Where a notification opens, and how clients group it.
#[derive(Clone, Debug)]
pub(crate) enum Conversation {
    Direct {
        id: String,
    },
    Channel {
        space_id: String,
        channel_id: String,
        /// `#channel (Space)`.
        title: String,
        recipient_count: i64,
    },
    Invitation {
        space_id: String,
        channel_id: Option<String>,
    },
}

/// Notification content, built at send time from the message or consent request.
#[derive(Clone, Debug)]
pub(crate) struct Alert {
    pub kind: String,
    pub message_id: String,
    pub notification_id: Option<String>,
    pub conversation: Conversation,
    pub title: String,
    pub body: String,
    pub sender: String,
    pub sender_id: String,
    pub sender_avatar_id: Option<i16>,
    pub attachment: Option<Attachment>,
    /// The thread a reply belongs to, so a notification opens it there.
    pub thread_root_id: Option<String>,
    /// Takes back the notification already shown for `message_id`, which
    /// was deleted: Android removes it, iPhone replaces it in place.
    pub retracts: bool,
}

/// Shown in place of a deleted message's notification on iPhone.
pub(crate) const RETRACTED_BODY: &str = "This message was deleted.";

/// One existing upload object, never a notification-specific stored rendition.
#[derive(Clone, Debug)]
pub(crate) struct Attachment {
    pub url: String,
    pub content_type: String,
    pub size: u64,
}

const MAX_NOTIFICATION_ATTACHMENT_BYTES: u64 = 50_000_000;

impl Alert {
    pub(crate) fn new(
        kind: &str,
        message_id: &str,
        conversation: Conversation,
        sender: &str,
        sender_id: &str,
        sender_avatar_id: Option<i16>,
        text: &str,
    ) -> Self {
        let title = match &conversation {
            _ if kind == "feedback" => format!("{sender} · Feedback"),
            Conversation::Direct { .. } => sender.to_owned(),
            Conversation::Channel { title, .. } => format!("{sender} · {title}"),
            Conversation::Invitation { .. } => "You’re invited!".into(),
        };
        Self {
            kind: kind.to_owned(),
            message_id: message_id.to_owned(),
            notification_id: None,
            conversation,
            title,
            body: preview(text),
            sender: sender.to_owned(),
            sender_id: sender_id.to_owned(),
            sender_avatar_id: sender_avatar_id.filter(|id| (0..800).contains(id)),
            attachment: None,
            thread_root_id: None,
            retracts: false,
        }
    }

    /// Replaces the notification of a deleted message: same title and
    /// conversation, no content.
    pub(crate) fn retraction(mut self) -> Self {
        self.body = RETRACTED_BODY.into();
        self.attachment = None;
        self.retracts = true;
        self
    }

    /// APNs `apns-collapse-id`: a message's notification can be replaced by
    /// its retraction. Invitations and requests are never retracted.
    pub(crate) fn collapse_id(&self) -> Option<&str> {
        (self.notification_id.is_none() && !self.message_id.is_empty())
            .then_some(self.message_id.as_str())
    }

    pub(crate) fn invitation(
        id: i64,
        space_id: &str,
        space_name: &str,
        channel: Option<(&str, &str)>,
        sender: &str,
        sender_id: &str,
        avatar: Option<i16>,
    ) -> Self {
        let (kind, target) = match channel {
            Some((_, name)) => ("channel.invitation", format!("private channel #{name}")),
            None => ("space.invitation", space_name.to_owned()),
        };
        let mut alert = Self::new(
            kind,
            "",
            Conversation::Invitation {
                space_id: space_id.into(),
                channel_id: channel.map(|(id, _)| id.into()),
            },
            sender,
            sender_id,
            avatar,
            &format!("{sender} invited you to {target}."),
        );
        alert.notification_id = Some(id.to_string());
        alert
    }

    pub(crate) fn request(
        id: i64,
        conversation: &str,
        sender: &str,
        sender_id: &str,
        avatar: Option<i16>,
    ) -> Self {
        let mut alert = Self::new(
            "direct.request",
            "",
            Conversation::Direct {
                id: conversation.into(),
            },
            sender,
            sender_id,
            avatar,
            &format!("{sender} wants to message you."),
        );
        alert.title = "Message request".into();
        alert.notification_id = Some(id.to_string());
        alert
    }

    /// Call only after recipient authorization and preparing notification content.
    pub(crate) fn with_content(mut self, message: &Value, cdn: Option<&CdnSigner>) -> Self {
        self.thread_root_id = message["threadRootId"].as_str().map(str::to_owned);
        let files: Vec<&Value> = [
            "/content/attachments",
            "/forward/message/content/attachments",
        ]
        .into_iter()
        .filter_map(|path| message.pointer(path).and_then(Value::as_array))
        .flatten()
        .collect();
        if message
            .pointer("/content/text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .is_empty()
            && !files.is_empty()
        {
            self.body = if files.len() > 1 {
                format!("Sent {} files", files.len())
            } else if files[0]["spoiler"] == true {
                "Sent a file".into()
            } else {
                match files[0]["kind"].as_str() {
                    Some("image") => "Sent a photo".into(),
                    Some("video") => "Sent a video".into(),
                    Some("audio") => "Sent an audio file".into(),
                    _ => preview(&format!(
                        "Sent a file: {}",
                        files[0]["name"].as_str().unwrap_or("attachment")
                    )),
                }
            };
        }
        if let Some(cdn) = cdn {
            self.attachment = files.into_iter().find_map(|file| {
                if file["unavailable"] == true || file["spoiler"] == true {
                    return None;
                }
                let id = file["id"].as_str()?;
                let (key, media) = if file["kind"] == "video" {
                    (crate::assets::preview_key(id), file.get("preview")?)
                } else {
                    (crate::assets::original_key(id), file)
                };
                let content_type = media["contentType"].as_str()?;
                if crate::assets::kind(content_type) != "image" {
                    return None;
                }
                let size = media["size"].as_u64()?;
                if !(1..=MAX_NOTIFICATION_ATTACHMENT_BYTES).contains(&size) {
                    return None;
                }
                Some(Attachment {
                    url: cdn.notification_url(&key, chrono::Utc::now().timestamp()),
                    content_type: content_type.into(),
                    size,
                })
            });
        }
        self
    }

    /// APNs `thread-id`, FCM `collapse_key`; invitations never group with messages.
    pub(crate) fn thread(&self) -> String {
        match &self.conversation {
            Conversation::Direct { .. } if self.kind == "direct.request" => format!(
                "request:{}",
                self.notification_id.as_deref().unwrap_or_default()
            ),
            Conversation::Direct { id } => id.clone(),
            Conversation::Channel { channel_id, .. } => channel_id.clone(),
            Conversation::Invitation { .. } => format!(
                "invitation:{}",
                self.notification_id.as_deref().unwrap_or_default()
            ),
        }
    }
}

const PREVIEW_CHARACTERS: usize = 180;

/// Message text, trimmed and cut to 180 characters including the `…`.
/// Messages without text (a forward, say) read `Sent a message`.
pub(crate) fn preview(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return "Sent a message".into();
    }
    if text.chars().count() <= PREVIEW_CHARACTERS {
        return text.into();
    }
    let mut cut: String = text.chars().take(PREVIEW_CHARACTERS - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

/// `Retry-After` in seconds, as APNs and FCM send it.
pub(crate) fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get("retry-after")?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|seconds| *seconds <= 86_400)
        .map(Duration::from_secs)
}

/// Provider reasons are short identifiers; anything else is not logged.
pub(crate) fn safe_reason(value: &str) -> &str {
    if value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        value
    } else {
        ""
    }
}

#[derive(Default)]
pub(crate) struct Providers {
    pub(crate) apns: Option<apns::Apns>,
    pub(crate) apns_sandbox: Option<apns::Apns>,
    pub(crate) fcm: Option<fcm::Fcm>,
}

/// Configured providers. `None` while `NOTIFICATIONS_ENABLED` is off.
#[derive(Clone, Default)]
pub(crate) struct Push {
    providers: Option<Arc<Providers>>,
    platforms: Vec<Platform>,
}

impl Push {
    /// Lists a platform only when it is in `PUSH_PLATFORMS` and its
    /// credentials load. Bad credentials leave that platform off rather than
    /// stopping the API, because a secret change restarts every replica.
    pub(crate) fn from_env(environment: &RuntimeEnvironment) -> Self {
        if !environment
            .get("NOTIFICATIONS_ENABLED")
            .is_some_and(|v| v == "true" || v == "1")
        {
            return Self::default();
        }
        let get = |key: &str| environment.get(key).filter(|v| !v.trim().is_empty());
        let listed = get("PUSH_PLATFORMS").unwrap_or_default();
        let mut providers = Providers::default();
        for name in listed.split(',').map(str::trim).filter(|v| !v.is_empty()) {
            let Some(platform) = Platform::parse(name) else {
                tracing::warn!(
                    event_name = "push_platform_unknown",
                    "PUSH_PLATFORMS lists an unknown platform"
                );
                continue;
            };
            let loaded = match platform {
                Platform::Apns => {
                    apns_from_env(&get, apns::PRODUCTION, "APNS_KEY_ID", "APNS_PRIVATE_KEY")
                        .map(|apns| providers.apns = Some(apns))
                }
                Platform::ApnsSandbox => apns_from_env(
                    &get,
                    apns::SANDBOX,
                    "APNS_SANDBOX_KEY_ID",
                    "APNS_SANDBOX_PRIVATE_KEY",
                )
                .map(|apns| providers.apns_sandbox = Some(apns)),
                Platform::Fcm => get("FCM_SERVICE_ACCOUNT_JSON")
                    .ok_or("credentials missing")
                    .and_then(|json| fcm::Fcm::new(fcm::API, &json))
                    .map(|fcm| providers.fcm = Some(fcm)),
            };
            if let Err(reason) = loaded {
                tracing::error!(
                    event_name = "push_platform_unavailable",
                    platform = platform.name(),
                    reason,
                    "push platform is listed but its credentials did not load"
                );
            }
        }
        Self::with_providers(providers)
    }

    pub(crate) fn with_providers(providers: Providers) -> Self {
        let platforms = Platform::ALL
            .into_iter()
            .filter(|platform| match platform {
                Platform::Apns => providers.apns.is_some(),
                Platform::ApnsSandbox => providers.apns_sandbox.is_some(),
                Platform::Fcm => providers.fcm.is_some(),
            })
            .collect();
        Self {
            providers: Some(Arc::new(providers)),
            platforms,
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.providers.is_some()
    }

    /// Advertised platforms, in a stable order.
    pub(crate) fn platforms(&self) -> &[Platform] {
        &self.platforms
    }

    pub(crate) async fn send(&self, platform: Platform, token: &str, alert: &Alert) -> Outcome {
        let providers = self.providers.as_deref();
        let sent = match platform {
            Platform::Apns => providers.and_then(|p| p.apns.as_ref()),
            Platform::ApnsSandbox => providers.and_then(|p| p.apns_sandbox.as_ref()),
            Platform::Fcm => {
                return match providers.and_then(|p| p.fcm.as_ref()) {
                    Some(fcm) => fcm.send(token, alert).await,
                    None => Outcome::Abandon("platform unavailable".into()),
                };
            }
        };
        match sent {
            Some(apns) => apns.send(token, alert).await,
            None => Outcome::Abandon("platform unavailable".into()),
        }
    }
}

/// Production and sandbox keys are separate; they share the team and topic.
fn apns_from_env(
    get: &impl Fn(&str) -> Option<String>,
    base: &str,
    key_id: &str,
    private_key: &str,
) -> Result<apns::Apns, &'static str> {
    let (Some(team), Some(key_id), Some(private_key)) =
        (get("APNS_TEAM_ID"), get(key_id), get(private_key))
    else {
        return Err("credentials missing");
    };
    let topic = get("APNS_TOPIC").unwrap_or_else(|| apns::DEFAULT_TOPIC.into());
    apns::Apns::new(base, &topic, &team, &key_id, &private_key)
}

/// Shared state of the expansion and delivery workers.
#[derive(Clone)]
pub(crate) struct Workers {
    pool: PgPool,
    push: Push,
    cdn: Option<Arc<CdnSigner>>,
    broker: Option<redis::Client>,
    presence: Arc<tokio::sync::Mutex<Option<Presence>>>,
    idle_timeout: Duration,
    deliveries: Arc<Notify>,
}

impl Workers {
    fn new(
        pool: PgPool,
        push: Push,
        broker: Option<redis::Client>,
        idle_timeout: Duration,
    ) -> Self {
        Self {
            pool,
            push,
            cdn: None,
            broker,
            presence: Arc::new(tokio::sync::Mutex::new(None)),
            idle_timeout,
            deliveries: Arc::new(Notify::new()),
        }
    }

    /// Connects lazily, so a Valkey outage at startup does not stop the API.
    async fn presence(&self) -> Option<Presence> {
        let mut presence = self.presence.lock().await;
        if presence.is_none()
            && let Some(broker) = &self.broker
        {
            *presence = tokio::time::timeout(
                Duration::from_secs(3),
                Presence::new(broker, self.idle_timeout),
            )
            .await
            .ok()
            .and_then(Result::ok);
        }
        presence.clone()
    }
}

fn idle_timeout(environment: &RuntimeEnvironment) -> Result<Duration, String> {
    // The gateway's activity timeout: the same value decides `online`.
    environment
        .get("PRESENCE_IDLE_TIMEOUT_SECONDS")
        .unwrap_or_else(|| "600".into())
        .parse::<u64>()
        .ok()
        .filter(|v| (1..=86400).contains(v))
        .map(Duration::from_secs)
        .ok_or_else(|| "PRESENCE_IDLE_TIMEOUT_SECONDS must be between 1 and 86400".into())
}

/// Account notifications run whenever chat is enabled. The phone flag controls
/// provider delivery only; desktop notifications need no provider credentials.
pub(crate) fn start(
    push: &Push,
    chat: Option<&Chat>,
    environment: &RuntimeEnvironment,
) -> Result<(), String> {
    let Some(chat) = chat else {
        return if push.enabled() {
            Err("NOTIFICATIONS_ENABLED requires CHAT_ENABLED".into())
        } else {
            Ok(())
        };
    };
    let mut workers = Workers::new(
        chat.pool.clone(),
        push.clone(),
        Some(chat.broker.clone()),
        idle_timeout(environment)?,
    );
    workers.cdn = chat.cdn.clone();
    tracing::info!(
        event_name = "notifications_enabled",
        platforms = ?push.platforms().iter().map(|p| p.name()).collect::<Vec<_>>(),
        "notification workers starting"
    );
    let expansion = workers.clone();
    tokio::spawn(async move {
        loop {
            match expansion::expand_pending(&expansion).await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(()) => tracing::warn!(
                    event_name = "notification_expansion_retry",
                    "notification expansion will retry"
                ),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    // Pruning runs beside delivery so a large backlog never delays a send.
    let pruning = workers.pool.clone();
    tokio::spawn(async move {
        loop {
            delivery::prune(&pruning).await;
            tokio::time::sleep(Duration::from_secs(600)).await;
        }
    });
    let retractions = workers.clone();
    tokio::spawn(async move {
        loop {
            match if retractions.push.enabled() {
                retraction::retract_pending(&retractions).await
            } else {
                Ok(false)
            } {
                Ok(true) => continue,
                Ok(false) => {}
                Err(()) => tracing::warn!(
                    event_name = "push_retraction_retry",
                    "push retraction batch will retry"
                ),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    tokio::spawn(async move {
        loop {
            // Never race a batch against a timer: cancelling one mid-send
            // could deliver a notification twice.
            match if workers.push.enabled() {
                delivery::deliver_pending(&workers).await
            } else {
                Ok(false)
            } {
                Ok(true) => continue,
                Ok(false) => {}
                Err(()) => tracing::warn!(
                    event_name = "push_delivery_retry",
                    "push delivery batch will retry"
                ),
            }
            tokio::select! {
                _ = workers.deliveries.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }
    });
    Ok(())
}

/// One outbox row per account-authored message, in its send transaction.
pub(crate) async fn enqueue(
    connection: &mut sqlx::PgConnection,
    message: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO public.notification_jobs (message_id) VALUES ($1)")
        .bind(message)
        .execute(connection)
        .await?;
    Ok(())
}

/// The caller already holds the recipient lock, serializing account cursor IDs.
/// The notification and its deliveries commit atomically with the consent request.
pub(crate) async fn consent(
    connection: &mut sqlx::PgConnection,
    recipient: i64,
    actor: i64,
    kind: &str,
    space: Option<i64>,
    channel: Option<i64>,
) -> Result<(), sqlx::Error> {
    let notification: Option<i64> = sqlx::query_scalar(
        "INSERT INTO public.notifications(user_id,kind,space_id,channel_id,actor_id)
         SELECT $1,$2,$3,$4,$5 WHERE NOT EXISTS(
           SELECT 1 FROM public.notification_settings WHERE user_id=$1
           AND (default_level='nothing' OR paused_until>now())) RETURNING id",
    )
    .bind(recipient)
    .bind(kind)
    .bind(space)
    .bind(channel)
    .bind(actor)
    .fetch_optional(&mut *connection)
    .await?;
    if let Some(notification) = notification {
        sqlx::query(
            "INSERT INTO public.notification_deliveries(notification_id,device_id)
             SELECT $1,d.id FROM public.notification_devices d
             JOIN public.account_sessions a ON a.token_hash=d.account_session_hash
             WHERE d.user_id=$2 AND d.revoked_at IS NULL AND a.revoked_at IS NULL
               AND a.expires_at>now() AND a.user_id=d.user_id",
        )
        .bind(notification)
        .bind(recipient)
        .execute(connection)
        .await?;
    }
    Ok(())
}

/// Logout ends every registration the session holds.
pub(crate) async fn revoke_session(
    connection: &mut sqlx::PgConnection,
    token_hash: &[u8],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE public.notification_devices SET revoked_at=now(),revoked_reason='logout',updated_at=now()
         WHERE account_session_hash=$1 AND revoked_at IS NULL",
    )
    .bind(token_hash)
    .execute(connection)
    .await?;
    Ok(())
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/push/config", get(config))
        .route("/api/push/devices", post(register).delete(unregister))
        .merge(settings::routes())
}

fn unavailable() -> ApiError {
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "notifications unavailable")
}

fn database_error(_: sqlx::Error) -> ApiError {
    tracing::error!(
        event_name = "notifications_database_failed",
        "notification database operation failed"
    );
    unavailable()
}

fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.database.as_ref().ok_or_else(unavailable)
}

fn bad_request(message: &'static str) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, message)
}

async fn config(
    State(state): State<AppState>,
    Extension(_principal): Extension<Principal>,
) -> Json<Value> {
    let platforms: Vec<&str> = state.push.platforms().iter().map(|p| p.name()).collect();
    Json(json!({ "platforms": platforms }))
}

#[derive(Debug, PartialEq, Eq)]
struct Device {
    platform: Platform,
    token: String,
    app_id: String,
}

fn visible(value: &str, max: usize) -> bool {
    (1..=max).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_graphic())
}

/// `{"platform", "token", "appId"?}`. APNs tokens are 64–200 hex characters
/// (stored lowercase); FCM tokens are 1–4096 visible characters.
fn device_input(body: &Value, platforms: &[Platform]) -> Result<Device, ApiError> {
    let platform = body["platform"]
        .as_str()
        .and_then(Platform::parse)
        .filter(|platform| platforms.contains(platform))
        .ok_or_else(|| bad_request("push platform unavailable"))?;
    let token = body["token"]
        .as_str()
        .filter(|token| match platform {
            Platform::Fcm => visible(token, 4096),
            Platform::Apns | Platform::ApnsSandbox => {
                (64..=200).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_hexdigit())
            }
        })
        .ok_or_else(|| bad_request("invalid push token"))?;
    let app_id = match body.get("appId") {
        None | Some(Value::Null) => "",
        Some(Value::String(app_id)) if visible(app_id, 255) => app_id,
        Some(_) => return Err(bad_request("invalid app id")),
    };
    Ok(Device {
        platform,
        token: match platform {
            Platform::Fcm => token.to_owned(),
            Platform::Apns | Platform::ApnsSandbox => token.to_ascii_lowercase(),
        },
        app_id: app_id.to_owned(),
    })
}

/// One registration per sign-in session, and one holder per address: a new
/// address replaces the session's previous one, and registering an address
/// another account or session holds revokes that row.
async fn register(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<Value>,
) -> Result<StatusCode, ApiError> {
    let device = device_input(&body, state.push.platforms())?;
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    // Session first, then address, so concurrent registrations cannot deadlock.
    for (namespace, key) in [
        (731_903, hex(&principal.token_hash)),
        (
            731_904,
            format!("{}:{}", device.platform.name(), device.token),
        ),
    ] {
        sqlx::query("SELECT pg_advisory_xact_lock($1, hashtext($2))")
            .bind(namespace)
            .bind(key)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
    }
    sqlx::query(
        "UPDATE public.notification_devices SET revoked_at=now(),revoked_reason='replaced',updated_at=now()
         WHERE revoked_at IS NULL
           AND ((transport=$1 AND address=$2 AND (account_session_hash<>$3 OR user_id<>$4))
                OR (account_session_hash=$3 AND NOT (transport=$1 AND address=$2)))",
    )
    .bind(device.platform.name())
    .bind(&device.token)
    .bind(&principal.token_hash)
    .bind(principal.user.id)
    .execute(&mut *tx)
    .await
    .map_err(database_error)?;
    let existing: Option<i64> = sqlx::query_scalar(
        "UPDATE public.notification_devices SET app_id=$4,updated_at=now()
         WHERE revoked_at IS NULL AND transport=$1 AND address=$2 AND account_session_hash=$3
         RETURNING id",
    )
    .bind(device.platform.name())
    .bind(&device.token)
    .bind(&principal.token_hash)
    .bind(&device.app_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?;
    if existing.is_none() {
        sqlx::query(
            "INSERT INTO public.notification_devices (user_id,account_session_hash,transport,app_id,address)
             VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(principal.user.id)
        .bind(&principal.token_hash)
        .bind(device.platform.name())
        .bind(&device.app_id)
        .bind(&device.token)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    }
    tx.commit().await.map_err(database_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Idempotent, and works for a platform that is no longer advertised.
async fn unregister(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<Value>,
) -> Result<StatusCode, ApiError> {
    let device = device_input(&body, &Platform::ALL)?;
    sqlx::query(
        "UPDATE public.notification_devices SET revoked_at=now(),revoked_reason='deleted',updated_at=now()
         WHERE user_id=$1 AND transport=$2 AND address=$3 AND revoked_at IS NULL",
    )
    .bind(principal.user.id)
    .bind(device.platform.name())
    .bind(&device.token)
    .execute(pool(&state)?)
    .await
    .map_err(database_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
