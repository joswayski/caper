//! Multiplexed application sessions. Connections are disposable; accepted commands
//! and current room/account state are not owned by a gateway process.
use super::*;
use crate::{AppState, Cloudflare, Config, account_token, presence::Presence};
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use futures_util::SinkExt;
use serde::Serialize;
use std::{collections::HashMap, sync::Mutex};
use tokio::sync::mpsc;
use tower::ServiceExt;
use uuid::Uuid;

const MAX_SUBSCRIPTIONS: usize = 32;
const COMMAND_WINDOW_MS: i64 = 120_000;

#[cfg(test)]
mod tests;

pub(super) struct ChannelFeed {
    events: broadcast::Sender<Value>,
    head: watch::Sender<i64>,
    registered: watch::Sender<bool>,
}

struct PersonFeed {
    value: watch::Sender<Value>,
}

#[derive(Default)]
pub(super) struct Application {
    state: Option<AppState>,
    presence: Option<Presence>,
    broker: Mutex<Option<redis::aio::MultiplexedConnection>>,
    idle_seconds: u64,
    channels: Mutex<HashMap<String, Arc<ChannelFeed>>>,
    people: Mutex<HashMap<String, Arc<PersonFeed>>>,
    pub(super) interests: tokio::sync::Notify,
}

impl Application {
    pub(super) async fn from_env(chat: &Chat, env: &RuntimeEnvironment) -> Result<Self, String> {
        let idle_seconds = env
            .get("PRESENCE_IDLE_TIMEOUT_SECONDS")
            .unwrap_or_else(|| "600".into())
            .parse::<u64>()
            .ok()
            .filter(|v| (1..=86400).contains(v))
            .ok_or("PRESENCE_IDLE_TIMEOUT_SECONDS must be between 1 and 86400")?;
        let mut state = AppState::with_database(
            Config::from_env(env)?,
            Arc::new(Cloudflare::new()),
            Some(chat.pool.clone()),
        );
        state.enable_shared_media(env).await?;
        state.chat = Some(chat.clone());
        let presence = Presence::new(&chat.broker, Duration::from_secs(idle_seconds))
            .await
            .map_err(|_| "could not initialize shared presence")?;
        let broker = Self::connect(chat)
            .await
            .map_err(|_| "could not connect application gateway")?;
        Ok(Self {
            state: Some(state),
            presence: Some(presence),
            broker: Mutex::new(Some(broker)),
            idle_seconds,
            ..Self::default()
        })
    }

    async fn connect(chat: &Chat) -> Result<redis::aio::MultiplexedConnection, redis::RedisError> {
        chat.broker
            .get_multiplexed_async_connection_with_config(
                &redis::AsyncConnectionConfig::new()
                    .set_connection_timeout(Duration::from_secs(3))
                    .set_response_timeout(Duration::from_secs(3)),
            )
            .await
    }

    pub(super) fn channel(&self, id: &str) -> Arc<ChannelFeed> {
        let feed = self
            .channels
            .lock()
            .unwrap()
            .entry(id.into())
            .or_insert_with(|| {
                Arc::new(ChannelFeed {
                    events: broadcast::channel(256).0,
                    head: watch::channel(-1).0,
                    registered: watch::channel(false).0,
                })
            })
            .clone();
        self.interests.notify_one();
        feed
    }

    pub(super) fn topics(&self) -> std::collections::HashSet<String> {
        self.channels
            .lock()
            .unwrap()
            .keys()
            .flat_map(|id| {
                [
                    format!("{}:{id}", chat::TOPIC),
                    format!("{}:{id}", chat::TYPING_TOPIC),
                ]
            })
            .collect()
    }

    pub(super) fn registered(&self, topics: &std::collections::HashSet<String>) {
        for (id, feed) in self.channels.lock().unwrap().iter() {
            feed.registered
                .send_replace(topics.contains(&format!("{}:{id}", chat::TOPIC)));
        }
    }

    fn person(&self, id: &str) -> Arc<PersonFeed> {
        self.people
            .lock()
            .unwrap()
            .entry(id.into())
            .or_insert_with(|| {
                Arc::new(PersonFeed {
                    value: watch::channel(Value::Null).0,
                })
            })
            .clone()
    }

    fn update_people(&self, values: Vec<Value>) {
        let feeds = self.people.lock().unwrap();
        for value in values {
            if let Some(id) = value["userId"].as_str()
                && let Some(feed) = feeds.get(id)
            {
                feed.value.send_if_modified(|old| {
                    if *old == value {
                        false
                    } else {
                        *old = value.clone();
                        true
                    }
                });
            }
        }
    }

    pub(super) fn dispatch(&self, event: &Value) {
        if let Some(id) = event["channelId"].as_str()
            && let Some(feed) = self.channels.lock().unwrap().get(id)
        {
            let _ = feed.events.send(event.clone());
        }
    }

    pub(super) fn start(self: &Arc<Self>, chat: Chat) {
        let app = self.clone();
        tokio::spawn(async move {
            loop {
                let connection = app.broker.lock().unwrap().clone();
                if let Some(mut connection) = connection
                    && redis::cmd("PING")
                        .query_async::<()>(&mut connection)
                        .await
                        .is_err()
                    && let Ok(connection) = Self::connect(&chat).await
                {
                    *app.broker.lock().unwrap() = Some(connection);
                }
                // Batch recovery-head reads once per interested channel per pod,
                // rather than a Postgres poll per connected recipient.
                let ids: Vec<String> = {
                    let mut feeds = app.channels.lock().unwrap();
                    feeds.retain(|_, feed| Arc::strong_count(feed) > 1);
                    feeds.keys().cloned().collect()
                };
                for chunk in ids.chunks(256) {
                    if let Ok(rows) = sqlx::query_as::<_, (String, i64)>("SELECT external_id,last_seq FROM public.channels WHERE external_id = ANY($1)")
                        .bind(chunk).fetch_all(&chat.pool).await {
                        let feeds = app.channels.lock().unwrap();
                        for (id, head) in rows {
                            if let Some(feed) = feeds.get(&id) {
                                feed.head.send_if_modified(|old| { if *old == head { false } else { *old = head; true } });
                            }
                        }
                    }
                }
                // Expiration is checked once per watched person per gateway,
                // not once per viewer. No whole-population index or snapshot.
                let people: Vec<String> = {
                    let mut feeds = app.people.lock().unwrap();
                    feeds.retain(|_, feed| Arc::strong_count(feed) > 1);
                    feeds.keys().cloned().collect()
                };
                if let Some(presence) = &app.presence {
                    for chunk in people.chunks(100) {
                        match presence.statuses(chunk).await {
                            Ok(values) => app.update_people(values),
                            Err(_) => {
                                let feeds = app.people.lock().unwrap();
                                for id in chunk {
                                    if let Some(feed) = feeds.get(id) {
                                        feed.value.send_replace(Value::Null);
                                    }
                                }
                            }
                        }
                    }
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }
}

#[derive(Clone)]
struct Identity {
    headers: HeaderMap,
    hash: Option<Vec<u8>>,
    user: Option<i64>,
    external_id: Option<String>,
}

impl Identity {
    async fn check(&self, state: &Gateway) -> Result<(), ApiError> {
        if let Some(hash) = &self.hash {
            let user = session_user(&state.chat.pool, hash).await?;
            if self.user != Some(user) {
                return Err(unauthorized());
            }
        }
        Ok(())
    }
}

fn unauthorized() -> ApiError {
    ApiError::new(StatusCode::UNAUTHORIZED, "session expired")
}
fn invalid() -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid gateway command")
}

pub(super) async fn upgrade(
    State(state): State<Gateway>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    // Old tabs can drain through the previous protocol during an API/gateway/web
    // rollout. New clients never create a channel-specific connection.
    if query.contains_key("channelId") {
        let query = Subscription {
            channel_id: query.get("channelId").cloned().ok_or_else(invalid)?,
            after: query.get("after").cloned().ok_or_else(invalid)?,
            typing: query.get("typing").is_some_and(|v| v == "true"),
        };
        return super::upgrade(State(state), headers, Query(query), ws).await;
    }
    if *state.drain.borrow() {
        return Err(chat::unavailable());
    }
    if headers
        .get("sec-fetch-site")
        .is_some_and(|v| v == "cross-site")
        || headers.get("origin").is_some_and(|origin| {
            origin
                .to_str()
                .ok()
                .and_then(|v| reqwest::Url::parse(v).ok())
                .as_ref()
                .and_then(|url| url.as_str().split('/').nth(2))
                != headers.get("host").and_then(|v| v.to_str().ok())
        })
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "cross-origin socket refused",
        ));
    }
    let hash = account_token(&headers).map(|token| Sha256::digest(token.as_bytes()).to_vec());
    let user = match &hash {
        Some(hash) => Some(session_user(&state.chat.pool, hash).await?),
        None => None,
    };
    let external_id = match user {
        Some(user) => Some(
            sqlx::query_scalar("SELECT external_id FROM public.users WHERE id=$1")
                .bind(user)
                .fetch_one(&state.chat.pool)
                .await
                .map_err(|_| chat::unavailable())?,
        ),
        None => None,
    };
    let identity = Identity {
        headers,
        hash,
        user,
        external_id,
    };
    let slot = state
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| chat::unavailable())?;
    Ok(ws
        .read_buffer_size(4096)
        .write_buffer_size(0)
        .max_write_buffer_size(256 * 1024)
        .max_frame_size(128 * 1024)
        .max_message_size(128 * 1024)
        .on_upgrade(move |socket| async move {
            let _slot = slot;
            serve(socket, state, identity).await;
        })
        .into_response())
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Subscribe {
    id: String,
    kind: String,
    channel_id: Option<String>,
    after: Option<String>,
    token: Option<String>,
    space_id: Option<String>,
    #[serde(default)]
    user_ids: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Command {
    id: Uuid,
    issued_at: i64,
    method: String,
    channel_id: Option<String>,
    token: Option<String>,
    chat_token: Option<String>,
    #[serde(default)]
    body: Value,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Frame {
    Subscribe(Subscribe),
    Unsubscribe {
        id: String,
    },
    Command(Command),
    #[serde(rename_all = "camelCase")]
    Heartbeat {
        activity_age_ms: u64,
    },
    #[serde(rename_all = "camelCase")]
    Activity {
        activity_age_ms: u64,
    },
}

async fn send(out: &mpsc::Sender<Value>, value: Value) -> Result<(), ()> {
    tokio::time::timeout(SEND_TIMEOUT, out.send(value))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

async fn serve(socket: WebSocket, state: Gateway, identity: Identity) {
    let (mut sink, mut source) = socket.split();
    let (out, mut queue) = mpsc::channel::<Value>(256);
    let writer = tokio::spawn(async move {
        while let Some(event) = queue.recv().await {
            if !matches!(
                tokio::time::timeout(
                    SEND_TIMEOUT,
                    sink.send(Message::Text(event.to_string().into()))
                )
                .await,
                Ok(Ok(()))
            ) {
                break;
            }
        }
    });
    let mut subscriptions: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
    let mut commands = tokio::task::JoinSet::new();
    let connection = Uuid::new_v4().to_string();
    let mut last_heartbeat = tokio::time::Instant::now();
    let mut timer = tokio::time::interval(Duration::from_secs(10));
    let mut drain = state.drain.subscribe();
    let mut deadline = None;
    let mut budget = 60u32;
    let mut activity_at =
        tokio::time::Instant::now() - Duration::from_secs(state.application.idle_seconds.max(600));
    let _ = send(&out, json!({"type":"hello","idleTimeoutSeconds":state.application.idle_seconds.max(1),"serverTime":chrono::Utc::now().timestamp_millis()})).await;
    if *drain.borrow_and_update() {
        let _ = send(&out, json!({"type":"migrating"})).await;
        deadline = Some(tokio::time::Instant::now() + HANDOFF_WINDOW);
    }
    loop {
        if writer.is_finished() {
            break;
        }
        tokio::select! {
            _ = async { match deadline { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => break,
            _ = drain.changed(), if deadline.is_none() => {
                if send(&out, json!({"type":"migrating"})).await.is_err() { break; }
                deadline = Some(tokio::time::Instant::now() + HANDOFF_WINDOW);
            }
            _ = timer.tick() => {
                budget = 60;
                if last_heartbeat.elapsed() > Duration::from_secs(30) || identity.check(&state).await.is_err() { break; }
                if let (Some(user), Some(presence)) = (&identity.external_id, &state.application.presence)
                    && presence.renew(user, &connection, activity_at.elapsed().as_millis().min(u64::MAX as u128) as u64).await.is_err() { break; }
                if send(&out, json!({"type":"heartbeat"})).await.is_err() { break; }
            }
            _ = commands.join_next(), if !commands.is_empty() => {}
            frame = source.next() => {
                let Some(Ok(Message::Text(text))) = frame else { break; };
                if budget == 0 { break; }
                budget -= 1;
                let Ok(frame) = serde_json::from_str::<Frame>(&text) else { break; };
                match frame {
                    Frame::Heartbeat { activity_age_ms } | Frame::Activity { activity_age_ms } => {
                        last_heartbeat = tokio::time::Instant::now();
                        // Client activity is a hint, never an account/auth claim.
                        let age = activity_age_ms.min(86_400_000);
                        activity_at = tokio::time::Instant::now() - Duration::from_millis(age);
                        if let (Some(user), Some(presence)) = (&identity.external_id, &state.application.presence)
                            && presence.renew(user, &connection, age).await.is_err() { break; }
                    }
                    Frame::Unsubscribe { id } => { if let Some(task) = subscriptions.remove(&id) { task.abort(); } }
                    Frame::Subscribe(subscription) => {
                        if subscription.id.is_empty() || subscription.id.len() > 64 || subscriptions.len() >= MAX_SUBSCRIPTIONS { break; }
                        if let Some(task) = subscriptions.remove(&subscription.id) { task.abort(); }
                        let state = state.clone(); let identity = identity.clone(); let out = out.clone();
                        let id = subscription.id.clone();
                        subscriptions.insert(id, tokio::spawn(async move {
                            let result = match subscription.kind.as_str() {
                                "chat" => chat_subscription(&state, &identity, &subscription, &out).await,
                                "media" => media_subscription(&state, &identity, &subscription, &out).await,
                                "presence" => presence_subscription(&state, &identity, &subscription, &out).await,
                                _ => Err(invalid()),
                            };
                            if let Err(error) = result {
                                let _ = send(&out, json!({"type":"error","id":subscription.id,"status":error.status.as_u16(),"error":error.message})).await;
                            }
                        }));
                    }
                    Frame::Command(command) => {
                        if commands.len() >= 8 { break; }
                        let state = state.clone(); let identity = identity.clone(); let out = out.clone();
                        commands.spawn(async move {
                            let id = command.id;
                            let result = tokio::time::timeout(COMMAND_TIMEOUT, execute(&state, &identity, &command))
                                .await.unwrap_or_else(|_| Err(chat::unavailable()));
                            let (status, body) = match result {
                                Ok(result) => result,
                                Err(error) => (error.status.as_u16(), json!({"error":error.message,"code":error.code})),
                            };
                            let _ = send(&out, json!({"type":"result","id":id,"status":status,"body":body})).await;
                        });
                    }
                }
            }
        }
    }
    for (_, task) in subscriptions {
        task.abort();
    }
    // An accepted command must finish and cache its result even when the socket
    // disappears. The bounded operation timeout owns its remaining lifetime.
    commands.detach_all();
    writer.abort();
    // Do not delete shared presence here: a short lease bridges reconnect/handoff.
}

async fn event(out: &mpsc::Sender<Value>, id: &str, value: Value) -> Result<(), ApiError> {
    send(out, json!({"type":"event","id":id,"event":value}))
        .await
        .map_err(|_| chat::unavailable())
}
async fn subscribed(out: &mpsc::Sender<Value>, id: &str) -> Result<(), ApiError> {
    send(out, json!({"type":"subscribed","id":id}))
        .await
        .map_err(|_| chat::unavailable())
}

async fn replay(
    state: &Gateway,
    identity: &Identity,
    sub: &Subscribe,
    out: &mpsc::Sender<Value>,
    after: &mut i64,
) -> Result<(), ApiError> {
    identity.check(state).await?;
    let channel = sub.channel_id.as_deref().ok_or_else(invalid)?;
    let access = channel_access(&state.chat.pool, channel, identity.user).await?;
    if *after > access.last_seq || access.last_seq - *after > REPLAY_LIMIT {
        event(out, &sub.id, json!({"type":"resync_required"})).await?;
        return Err(invalid());
    }
    while *after < access.last_seq {
        identity.check(state).await?;
        channel_access(&state.chat.pool, channel, identity.user).await?;
        let rows: Vec<(i64, Value)> = sqlx::query_as("SELECT seq,payload FROM public.channel_events WHERE channel_id=$1 AND seq>$2 AND seq<=$3 ORDER BY seq LIMIT 128")
            .bind(access.id).bind(*after).bind(access.last_seq).fetch_all(&state.chat.pool).await.map_err(|_| chat::unavailable())?;
        if rows.is_empty() {
            return Err(chat::unavailable());
        }
        for (seq, payload) in rows {
            if seq != *after + 1 {
                event(out, &sub.id, json!({"type":"resync_required"})).await?;
                return Err(invalid());
            }
            event(out, &sub.id, payload).await?;
            *after = seq;
        }
    }
    event(
        out,
        &sub.id,
        json!({"type":"ready","cursor":after.to_string()}),
    )
    .await
}

async fn chat_subscription(
    state: &Gateway,
    identity: &Identity,
    sub: &Subscribe,
    out: &mpsc::Sender<Value>,
) -> Result<(), ApiError> {
    let channel = sub.channel_id.as_deref().ok_or_else(invalid)?;
    identity.check(state).await?;
    channel_access(&state.chat.pool, channel, identity.user).await?;
    let feed = state.application.channel(channel);
    let mut events = feed.events.subscribe();
    let mut heads = feed.head.subscribe();
    let mut registered = feed.registered.subscribe();
    tokio::time::timeout(Duration::from_secs(5), registered.wait_for(|ready| *ready))
        .await
        .map_err(|_| chat::unavailable())?
        .map_err(|_| chat::unavailable())?;
    let mut check = tokio::time::interval(Duration::from_secs(10));
    let mut after = chat::cursor(sub.after.as_deref().unwrap_or("0"))?;
    replay(state, identity, sub, out, &mut after).await?;
    subscribed(out, &sub.id).await?;
    loop {
        tokio::select! {
            incoming = events.recv() => match incoming {
                Ok(payload) => {
                    identity.check(state).await?;
                    channel_access(&state.chat.pool, channel, identity.user).await?;
                    if payload["type"] == "typing.updated" { event(out, &sub.id, payload).await?; }
                    else if let Some(seq) = payload["seq"].as_str().and_then(|s| s.parse::<i64>().ok()) {
                        if seq == after + 1 { event(out, &sub.id, payload).await?; after = seq; }
                        else if seq > after { replay(state, identity, sub, out, &mut after).await?; }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => replay(state, identity, sub, out, &mut after).await?,
                Err(_) => return Err(chat::unavailable()),
            },
            _ = heads.changed() => {
                let head = *heads.borrow_and_update();
                if head > after { replay(state, identity, sub, out, &mut after).await?; }
            }
            _ = check.tick() => { identity.check(state).await?; channel_access(&state.chat.pool, channel, identity.user).await?; }
        }
    }
}

async fn room(
    state: &Gateway,
    identity: &Identity,
    channel: Option<&str>,
) -> Result<AppState, ApiError> {
    identity.check(state).await?;
    let mut room = state
        .application
        .state
        .clone()
        .ok_or_else(chat::unavailable)?;
    room.media_channel = channel.map(str::to_owned);
    room.media_session = identity.hash.clone();
    room.check_media_access().await?;
    crate::ensure_enabled(&room)?;
    Ok(room)
}

async fn media_subscription(
    state: &Gateway,
    identity: &Identity,
    sub: &Subscribe,
    out: &mpsc::Sender<Value>,
) -> Result<(), ApiError> {
    let room = room(state, identity, sub.channel_id.as_deref()).await?;
    let mut updates = room.room_updates();
    let mut repair = tokio::time::interval(Duration::from_secs(10));
    let mut revision = None;
    loop {
        identity.check(state).await?;
        room.check_media_access().await?;
        let value = room
            .read(|registry| {
                if let Some(token) = &sub.token {
                    let id = crate::authenticate(registry, token)?;
                    let participant = &registry.participants[&id];
                    if participant.monitor.is_some()
                        || (room.media_channel.is_some()
                            && participant.account_session != identity.hash)
                    {
                        return Err(unauthorized());
                    }
                    Ok(crate::public_snapshot(registry))
                } else {
                    Ok(crate::presence_snapshot(registry))
                }
            })
            .await?;
        let next = value["revision"].as_u64().ok_or_else(invalid)?;
        if revision != Some(next) {
            let mut value = value;
            value["type"] = json!("snapshot");
            event(out, &sub.id, value).await?;
            if revision.is_none() {
                subscribed(out, &sub.id).await?;
            }
            revision = Some(next);
        }
        tokio::select! { _ = updates.changed() => {}, _ = repair.tick() => {} }
    }
}

async fn presence_subscription(
    state: &Gateway,
    identity: &Identity,
    sub: &Subscribe,
    out: &mpsc::Sender<Value>,
) -> Result<(), ApiError> {
    if sub.user_ids.is_empty() || sub.user_ids.len() > 100 {
        return Err(invalid());
    }
    let presence = state
        .application
        .presence
        .as_ref()
        .ok_or_else(chat::unavailable)?;
    let space = sub.space_id.as_deref().ok_or_else(invalid)?;
    let user = identity.user.ok_or_else(unauthorized)?;
    let feeds: Vec<_> = sub
        .user_ids
        .iter()
        .map(|id| state.application.person(id))
        .collect();
    let streams: Vec<_> = feeds
        .iter()
        .map(|feed| {
            futures_util::stream::unfold(feed.value.subscribe(), |mut receiver| async move {
                receiver.changed().await.ok()?;
                receiver.borrow_and_update();
                Some(((), receiver))
            })
            .boxed()
        })
        .collect();
    let mut changes = futures_util::stream::select_all(streams);
    let mut last = Value::Null;
    let mut timer = tokio::time::interval(Duration::from_secs(10));
    loop {
        identity.check(state).await?;
        let permitted: Vec<String> = sqlx::query_scalar("SELECT u.external_id FROM public.space_members m JOIN public.spaces s ON s.id=m.space_id JOIN public.users u ON u.id=m.user_id WHERE s.external_id=$1 AND s.deleted_at IS NULL AND u.deleted_at IS NULL AND u.external_id=ANY($2) AND EXISTS(SELECT 1 FROM public.space_members own WHERE own.space_id=s.id AND own.user_id=$3)")
            .bind(space).bind(&sub.user_ids).bind(user).fetch_all(&state.chat.pool).await.map_err(|_| chat::unavailable())?;
        // Fail closed on removed memberships, including subscriptions established
        // before a privacy change. Never return global population snapshots.
        let unique: std::collections::HashSet<_> = sub.user_ids.iter().collect();
        if permitted.len() != unique.len() {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "presence subscription denied",
            ));
        }
        if last.is_null() {
            state
                .application
                .update_people(presence.statuses(&sub.user_ids).await?);
        }
        let values: Vec<Value> = feeds
            .iter()
            .map(|feed| feed.value.borrow().clone())
            .collect();
        if values.iter().any(Value::is_null) {
            return Err(chat::unavailable());
        }
        let members = json!(values);
        if members != last {
            event(out, &sub.id, json!({"type":"snapshot","members":members})).await?;
            if last.is_null() {
                subscribed(out, &sub.id).await?;
            }
            last = members;
        }
        tokio::select! { _ = changes.next() => {}, _ = timer.tick() => {} }
    }
}

async fn execute(
    state: &Gateway,
    identity: &Identity,
    command: &Command,
) -> Result<(u16, Value), ApiError> {
    identity.check(state).await?;
    if *state.drain.borrow() {
        return Err(
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "gateway is draining")
                .with_code("gateway_draining"),
        );
    }
    let age = chrono::Utc::now()
        .timestamp_millis()
        .checked_sub(command.issued_at)
        .ok_or_else(invalid)?;
    if !(-10_000..COMMAND_WINDOW_MS).contains(&age) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "command expired; refresh current state",
        ));
    }
    let operation = command.method.strip_prefix("media.");
    if command.method != "typing"
        && !matches!(
            operation,
            Some(
                "join"
                    | "leave"
                    | "state"
                    | "snapshot"
                    | "publish"
                    | "subscribe"
                    | "negotiate"
                    | "close"
                    | "turn"
                    | "restart-ice"
                    | "restart-ice-ack"
                    | "status"
            )
        )
    {
        return Err(invalid());
    }
    if let Some(channel) = &command.channel_id {
        if !valid_id(channel) {
            return Err(invalid());
        }
        channel_access(&state.chat.pool, channel, identity.user).await?;
    }
    // Binding to credentials prevents another account from retrieving cached
    // capability-bearing results, even if it learns a request UUID.
    let owner = identity
        .hash
        .as_deref()
        .map(|h| format!("{h:x?}"))
        .unwrap_or_else(|| {
            format!(
                "{:x}",
                Sha256::digest(
                    command
                        .chat_token
                        .as_deref()
                        .or(command.token.as_deref())
                        .unwrap_or("guest")
                        .as_bytes()
                )
            )
        });
    let key = format!("caper:gateway:command:{{{owner}}}:{}", command.id);
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(command).map_err(|_| invalid())?)
    );
    let mut broker = state
        .application
        .broker
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(chat::unavailable)?;
    let script = redis::Script::new(
        r#"
        local old=redis.call('GET',KEYS[1])
        if old then return old end
        redis.call('SET',KEYS[1],ARGV[1],'PX',ARGV[2])
        return ''
    "#,
    );
    let pending = json!({"fingerprint":fingerprint,"pending":true}).to_string();
    let cached: String = script
        .key(&key)
        .arg(&pending)
        .arg(COMMAND_WINDOW_MS + 30_000)
        .invoke_async(&mut broker)
        .await
        .map_err(|_| chat::unavailable())?;
    if !cached.is_empty() {
        let cached: Value = serde_json::from_str(&cached).map_err(|_| chat::unavailable())?;
        if cached["fingerprint"] != fingerprint {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "request ID already used",
            ));
        }
        if cached["pending"] == true {
            return Err(
                ApiError::new(StatusCode::CONFLICT, "command outcome pending")
                    .with_code("command_pending"),
            );
        }
        return Ok((
            cached["status"].as_u64().ok_or_else(invalid)? as u16,
            cached["body"].clone(),
        ));
    }
    let app = state
        .application
        .state
        .clone()
        .ok_or_else(chat::unavailable)?;
    let mut headers = identity.headers.clone();
    headers.insert("content-type", "application/json".parse().unwrap());
    if let Some(token) = &command.token {
        headers.insert("x-caper-media-token", token.parse().map_err(|_| invalid())?);
    }
    if let Some(token) = &command.chat_token {
        headers.insert("x-caper-chat-token", token.parse().map_err(|_| invalid())?);
    }
    let path = match operation {
        Some(operation) => match &command.channel_id {
            Some(channel) if valid_id(channel) => {
                format!("/api/channels/{channel}/media/{operation}")
            }
            Some(_) => return Err(invalid()),
            None => format!("/api/media/{operation}"),
        },
        None => {
            let channel = command
                .channel_id
                .as_deref()
                .filter(|id| valid_id(id))
                .ok_or_else(invalid)?;
            format!("/api/chat/channels/{channel}/typing")
        }
    };
    let mut request = Request::builder()
        .method(if operation == Some("status") {
            "GET"
        } else {
            "POST"
        })
        .uri(path)
        .body(Body::from(
            serde_json::to_vec(&command.body).map_err(|_| invalid())?,
        ))
        .map_err(|_| invalid())?;
    *request.headers_mut() = headers;
    // Fixed operations dispatch into the SAME authorization/state-machine handlers
    // as HTTP, not a provider proxy or a second implementation of media rules.
    let response = tokio::time::timeout(Duration::from_secs(25), crate::app(app).oneshot(request))
        .await
        .map_err(|_| chat::unavailable())?
        .unwrap();
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), 128 * 1024)
        .await
        .map_err(|_| chat::unavailable())?;
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).map_err(|_| chat::unavailable())?
    };
    let saved = json!({"fingerprint":fingerprint,"status":status,"body":body}).to_string();
    let _: () = redis::cmd("SET")
        .arg(&key)
        .arg(saved)
        .arg("PX")
        .arg(COMMAND_WINDOW_MS + 30_000)
        .query_async(&mut broker)
        .await
        .map_err(|_| chat::unavailable())?;
    Ok((status, body))
}

fn valid_id(id: &str) -> bool {
    id.len() == 12 && id.bytes().all(|c| c.is_ascii_alphanumeric())
}
