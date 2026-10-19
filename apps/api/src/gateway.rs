//! Independently deployed, bounded WebSocket delivery with make-before-break
//! drain. The broker is a fast path, never the authority for replay.
use crate::{
    ApiError, RuntimeEnvironment,
    chat::{self, Chat},
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
#[cfg(test)]
pub(crate) mod testing;

pub const HANDOFF_WINDOW: Duration = Duration::from_secs(20);
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(35);
const SEND_TIMEOUT: Duration = Duration::from_secs(3);
const REPLAY_LIMIT: i64 = 2000;

#[derive(Clone)]
pub struct Gateway {
    chat: Chat,
    application: Arc<application::Application>,
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
        let (drain, _) = watch::channel(false);
        Self {
            chat,
            application: Arc::new(application::Application::default()),
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
                            pubsub.subscribe(&[
                                chat::TOPIC,
                                chat::TYPING_TOPIC,
                                chat::PROFILE_TOPIC
                            ])
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
                                if message.get_channel_name() == chat::PROFILE_TOPIC {
                                    if let Ok(text) = message.get_payload::<String>()
                                        && let Ok(update) = serde_json::from_str::<Value>(&text) {
                                        state.application.dispatch_profiles(&update);
                                    }
                                } else if let Ok(text) = message.get_payload::<String>()
                                    && let Ok(event) = serde_json::from_str::<Value>(&text) {
                                    state.application.dispatch(&event);
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
                // One readiness check per gateway, for /readyz.
                let result = sqlx::query_scalar::<_, i32>("SELECT 1")
                    .fetch_one(&state.chat.pool)
                    .await;
                state
                    .database_ready
                    .store(result.is_ok(), Ordering::Release);
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
