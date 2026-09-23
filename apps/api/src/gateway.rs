//! Independently deployed, bounded WebSocket delivery with make-before-break
//! drain. The broker is a fast path, never the authority for replay.
use crate::{
    ApiError, RuntimeEnvironment,
    chat::{self, Chat},
    session_cookie,
    spaces::{channel_access, session_user},
};
use axum::{
    Router,
    extract::{
        Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Semaphore, broadcast, watch};

mod application;

pub const HANDOFF_WINDOW: Duration = Duration::from_secs(20);
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(35);
const SEND_TIMEOUT: Duration = Duration::from_secs(3);
const REPLAY_LIMIT: i64 = 2000;

#[derive(Clone)]
pub struct Gateway {
    chat: Chat,
    application: Arc<application::Application>,
    events: broadcast::Sender<Value>,
    typing: broadcast::Sender<Value>,
    repair: watch::Sender<u64>,
    drain: watch::Sender<bool>,
    slots: Arc<Semaphore>,
    broker_ready: Arc<AtomicBool>,
    database_ready: Arc<AtomicBool>,
}

impl Gateway {
    pub async fn from_env(
        pool: Option<&sqlx::PgPool>,
        environment: &RuntimeEnvironment,
    ) -> Result<Self, String> {
        let chat = Chat::from_env(pool, environment)
            .await?
            .ok_or("gateway requires CHAT_ENABLED=true")?;
        let mut state = Self::new(chat);
        state.application =
            Arc::new(application::Application::from_env(&state.chat, environment).await?);
        let sockets = environment
            .get("GATEWAY_MAX_CONNECTIONS")
            .unwrap_or_else(|| "4096".into())
            .parse::<usize>()
            .ok()
            .filter(|v| (1..=100_000).contains(v))
            .ok_or("GATEWAY_MAX_CONNECTIONS must be between 1 and 100000")?;
        state.slots = Arc::new(Semaphore::new(sockets));
        Ok(state)
    }
    pub(crate) fn new(chat: Chat) -> Self {
        let (events, _) = broadcast::channel(256);
        let (typing, _) = broadcast::channel(64);
        let (repair, _) = watch::channel(0);
        let (drain, _) = watch::channel(false);
        Self {
            chat,
            application: Arc::new(application::Application::default()),
            events,
            typing,
            repair,
            drain,
            slots: Arc::new(Semaphore::new(128)),
            broker_ready: Arc::new(AtomicBool::new(false)),
            database_ready: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn begin_shutdown(&self) {
        self.drain.send_replace(true);
    }

    pub fn start(&self) {
        self.application.start(self.chat.clone());
        let state = self.clone();
        tokio::spawn(async move {
            loop {
                if let Ok(Ok(mut pubsub)) = tokio::time::timeout(
                    Duration::from_secs(3),
                    state.chat.broker.get_async_pubsub(),
                )
                .await
                    && matches!(
                        tokio::time::timeout(
                            Duration::from_secs(3),
                            pubsub.subscribe(&[chat::TOPIC, chat::TYPING_TOPIC])
                        )
                        .await,
                        Ok(Ok(()))
                    )
                {
                    state.broker_ready.store(true, Ordering::Release);
                    let (mut sink, mut stream) = pubsub.split();
                    let mut topics = std::collections::HashSet::<String>::new();
                    let mut refresh = tokio::time::interval(Duration::from_secs(2));
                    loop {
                        tokio::select! {
                            message = stream.next() => {
                                let Some(message) = message else { break; };
                                if let Ok(text) = message.get_payload::<String>()
                                    && let Ok(event) = serde_json::from_str::<Value>(&text) {
                                    state.application.dispatch(&event);
                                    let sender = if event["type"] == "typing.updated" { &state.typing } else { &state.events };
                                    let _ = sender.send(event);
                                }
                            }
                            _ = async {
                                tokio::select! { _ = refresh.tick() => {}, _ = state.application.interests.notified() => {} }
                            } => {
                                let next = state.application.topics();
                                let added: Vec<_> = next.difference(&topics).cloned().collect();
                                let removed: Vec<_> = topics.difference(&next).cloned().collect();
                                let result = tokio::time::timeout(Duration::from_secs(3), async {
                                    if !added.is_empty() { sink.subscribe(added).await?; }
                                    if !removed.is_empty() { sink.unsubscribe(removed).await?; }
                                    sink.ping::<Vec<String>>().await
                                }).await;
                                if !matches!(result, Ok(Ok(_))) { break; }
                                topics = next;
                                state.application.registered(&topics);
                            }
                        }
                    }
                }
                state.broker_ready.store(false, Ordering::Release);
                state
                    .application
                    .registered(&std::collections::HashSet::new());
                tracing::warn!(
                    event_name = "chat_broker_reconnect",
                    "gateway reconnecting to broker"
                );
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
        let state = self.clone();
        tokio::spawn(async move {
            loop {
                // One readiness check per gateway. Each recipient repairs its
                // own channel cursor on the resulting tick.
                let result = sqlx::query_scalar::<_, i32>("SELECT 1")
                    .fetch_one(&state.chat.pool)
                    .await;
                state
                    .database_ready
                    .store(result.is_ok(), Ordering::Release);
                if result.is_ok() {
                    state
                        .repair
                        .send_modify(|tick| *tick = tick.wrapping_add(1));
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }
}

pub fn router(state: Gateway) -> Router {
    Router::new()
        .route("/health", get(|| async { StatusCode::NO_CONTENT }))
        .route("/readyz", get(ready))
        .route("/api/chat/events", get(application::upgrade))
        .with_state(state)
}

async fn ready(State(state): State<Gateway>) -> StatusCode {
    if *state.drain.borrow()
        || !state.broker_ready.load(Ordering::Acquire)
        || !state.database_ready.load(Ordering::Acquire)
    {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::NO_CONTENT
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Subscription {
    channel_id: String,
    after: String,
    // Older browser parsers reject unknown events. Typing is explicitly opt-in.
    #[serde(default)]
    typing: bool,
}

async fn upgrade(
    State(state): State<Gateway>,
    headers: HeaderMap,
    Query(query): Query<Subscription>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    if *state.drain.borrow() {
        return Err(chat::unavailable());
    }
    // Public demo reads do not require login, but browsers may only open
    // same-origin sockets. Credentials are accepted only from the account cookie.
    if headers
        .get("sec-fetch-site")
        .is_some_and(|v| v == "cross-site")
        || headers.get("origin").is_some_and(|origin| {
            let origin = origin
                .to_str()
                .ok()
                .and_then(|v| reqwest::Url::parse(v).ok());
            let host = headers.get("host").and_then(|v| v.to_str().ok());
            origin
                .as_ref()
                .and_then(|url| url.as_str().split('/').nth(2))
                != host
        })
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "cross-origin socket refused",
        ));
    }
    let after = chat::cursor(&query.after)?;
    let token_hash =
        session_cookie(&headers).map(|token| Sha256::digest(token.as_bytes()).to_vec());
    let user = match token_hash.as_deref() {
        Some(hash) => Some(session_user(&state.chat.pool, hash).await?),
        None => None,
    };
    let channel = channel_access(&state.chat.pool, &query.channel_id, user)
        .await?
        .id;
    let slot = state
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| chat::unavailable())?;
    Ok(ws
        .read_buffer_size(4096)
        .write_buffer_size(0)
        .max_write_buffer_size(64 * 1024)
        .max_message_size(1024)
        .max_frame_size(1024)
        .on_upgrade(move |socket| async move {
            let _slot = slot;
            let _ = serve(
                socket,
                state,
                channel,
                query.channel_id,
                token_hash,
                after,
                query.typing,
            )
            .await;
        })
        .into_response())
}

async fn write(socket: &mut WebSocket, event: Value) -> Result<(), ()> {
    tokio::time::timeout(
        SEND_TIMEOUT,
        socket.send(Message::Text(event.to_string().into())),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())
}

async fn catch_up(
    socket: &mut WebSocket,
    state: &Gateway,
    channel: i64,
    external_id: &str,
    token_hash: Option<&[u8]>,
    after: &mut i64,
) -> Result<(), ()> {
    let head = authorized(state, external_id, token_hash).await?.last_seq;
    if *after > head || head - *after > REPLAY_LIMIT {
        write(socket, json!({"type":"resync_required"})).await?;
        return Err(());
    }
    while *after < head {
        let access = authorized(state, external_id, token_hash).await?;
        if access.id != channel {
            return Err(());
        }
        let rows: Vec<(i64, Value)> = sqlx::query_as("SELECT seq, payload FROM public.channel_events WHERE channel_id = $1 AND seq > $2 AND seq <= $3 ORDER BY seq LIMIT 16")
            .bind(channel).bind(*after).bind(head).fetch_all(&state.chat.pool).await.map_err(|_| ())?;
        if rows.is_empty() {
            write(socket, json!({"type":"resync_required"})).await?;
            return Err(());
        }
        for (seq, payload) in rows {
            if seq != *after + 1 {
                write(socket, json!({"type":"resync_required"})).await?;
                return Err(());
            }
            write(socket, payload).await?;
            *after = seq;
        }
    }
    write(socket, json!({"type":"ready","cursor":after.to_string()})).await
}

async fn authorized(
    state: &Gateway,
    external_id: &str,
    token_hash: Option<&[u8]>,
) -> Result<crate::spaces::ChannelAccess, ()> {
    let user = match token_hash {
        Some(hash) => Some(session_user(&state.chat.pool, hash).await.map_err(|_| ())?),
        None => None,
    };
    channel_access(&state.chat.pool, external_id, user)
        .await
        .map_err(|_| ())
}

async fn serve(
    mut socket: WebSocket,
    state: Gateway,
    channel: i64,
    external_id: String,
    token_hash: Option<Vec<u8>>,
    mut after: i64,
    with_typing: bool,
) -> Result<(), ()> {
    // Buffer live before capturing a DB high-water mark. Replay then merge by
    // sequence. Lagging bounded buffers trigger another replay, never a skip.
    let _feed = state.application.channel(&external_id);
    let mut events = state.events.subscribe();
    let mut repair = state.repair.subscribe();
    let mut drain = state.drain.subscribe();
    let mut deadline = None;
    let mut last_pong = tokio::time::Instant::now();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(10));
    // Bound initial catch-up, including slow readers, to avoid retaining tasks.
    tokio::time::timeout(
        Duration::from_secs(10),
        catch_up(
            &mut socket,
            &state,
            channel,
            &external_id,
            token_hash.as_deref(),
            &mut after,
        ),
    )
    .await
    .map_err(|_| ())??;
    // No replay or initial buffer for ephemeral presence; dropping it must
    // never consume durable buffer space or alter the delivery cursor.
    let mut typing = state.typing.subscribe();
    if *drain.borrow_and_update() {
        write(&mut socket, json!({"type":"migrating"})).await?;
        deadline = Some(tokio::time::Instant::now() + HANDOFF_WINDOW);
    }
    loop {
        tokio::select! {
            _ = async { match deadline { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => return Ok(()),
            _ = drain.changed(), if deadline.is_none() => {
                write(&mut socket, json!({"type":"migrating"})).await?;
                deadline = Some(tokio::time::Instant::now() + HANDOFF_WINDOW);
            }
            event = events.recv() => {
                match event {
                    Ok(event) if event["channelId"].as_str() == Some(&external_id) => {
                        authorized(&state, &external_id, token_hash.as_deref()).await?;
                        let seq = event["seq"].as_str().and_then(|v| v.parse::<i64>().ok()).ok_or(())?;
                        if seq == after + 1 { write(&mut socket, event).await?; after = seq; }
                        else if seq > after { tokio::time::timeout(Duration::from_secs(10), catch_up(&mut socket, &state, channel, &external_id, token_hash.as_deref(), &mut after)).await.map_err(|_| ())??; }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => { tokio::time::timeout(Duration::from_secs(10), catch_up(&mut socket, &state, channel, &external_id, token_hash.as_deref(), &mut after)).await.map_err(|_| ())??; }
                    Err(broadcast::error::RecvError::Closed) => return Err(()),
                    _ => {},
                }
            }
            event = typing.recv(), if with_typing => {
                if let Ok(event) = event
                    && event["channelId"].as_str() == Some(&external_id)
                {
                    authorized(&state, &external_id, token_hash.as_deref()).await?;
                    write(&mut socket, event).await?;
                }
            }
            _ = repair.changed() => {
                repair.borrow_and_update();
                let access = authorized(&state, &external_id, token_hash.as_deref()).await?;
                if access.last_seq > after {
                    tokio::time::timeout(Duration::from_secs(10), catch_up(&mut socket, &state, channel, &external_id, token_hash.as_deref(), &mut after)).await.map_err(|_| ())??;
                }
            }
            _ = heartbeat.tick() => {
                if last_pong.elapsed() > Duration::from_secs(30) { return Err(()); }
                tokio::time::timeout(SEND_TIMEOUT, socket.send(Message::Ping(Vec::new().into()))).await.map_err(|_| ())?.map_err(|_| ())?;
            }
            frame = socket.recv() => match frame {
                Some(Ok(Message::Pong(_))) => last_pong = tokio::time::Instant::now(),
                Some(Ok(Message::Ping(_))) => {},
                // Commands belong to HTTP; arbitrary client payloads are refused.
                _ => return Ok(()),
            }
        }
    }
}
