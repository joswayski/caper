//! Shared state for the bounded General channel. Transactions contain only state
//! changes: provider HTTP requests must never run inside a retryable closure.
use super::*;
use redis::aio::{MultiplexedConnection, PubSub};
use std::{
    ops::{Add, Sub},
    sync::atomic::{AtomicBool, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Semaphore;

const IO_TIMEOUT: Duration = Duration::from_secs(3);
const KEY: &str = "caper:{general}:v1:state";

/// Absolute UTC milliseconds, not a process-relative Instant. Hosts must have
/// synchronized clocks; elapsed calculations saturate during clock correction.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Timestamp(u64);
impl Timestamp {
    pub fn now() -> Self {
        Self(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        )
    }
    pub fn elapsed(self) -> Duration {
        Self::now().duration_since(self)
    }
    pub fn duration_since(self, earlier: Self) -> Duration {
        Duration::from_millis(self.0.saturating_sub(earlier.0))
    }
}
impl Add<Duration> for Timestamp {
    type Output = Self;
    fn add(self, rhs: Duration) -> Self {
        Self(self.0.saturating_add(rhs.as_millis() as u64))
    }
}
impl Sub<Duration> for Timestamp {
    type Output = Self;
    fn sub(self, rhs: Duration) -> Self {
        Self(self.0.saturating_sub(rhs.as_millis() as u64))
    }
}

fn unavailable() -> ApiError {
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "media state unavailable")
}

pub(crate) fn validate_url(url: &str, allow_insecure: bool) -> Result<(), ApiError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| unavailable())?;
    let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
        || (allow_insecure && parsed.host_str() == Some("valkey"));
    if !matches!(parsed.scheme(), "redis" | "rediss")
        || (!local && (parsed.scheme() != "rediss" || parsed.password().is_none()))
        || parsed.fragment().is_some()
    {
        return Err(unavailable());
    }
    Ok(())
}

pub(super) struct ValkeyStore {
    client: redis::Client,
    reads: Mutex<MultiplexedConnection>,
    transactions: Mutex<Vec<MultiplexedConnection>>,
    slots: Semaphore,
    key: String,
    topic: String,
    subscribed: Arc<AtomicBool>,
    listener: std::sync::Mutex<Option<tokio::task::AbortHandle>>,
}

impl Drop for ValkeyStore {
    fn drop(&mut self) {
        if let Some(listener) = self.listener.get_mut().unwrap().take() {
            listener.abort();
        }
    }
}

impl ValkeyStore {
    async fn connect(url: &str, key: &str) -> Result<(Arc<Self>, PubSub), ApiError> {
        // Process-only opt-in, matching DATABASE_ALLOW_INSECURE: a hosted secret
        // cannot disable transport requirements. Only the Compose hostname qualifies.
        let allow_insecure = std::env::var("VALKEY_ALLOW_INSECURE")
            .ok()
            .is_some_and(|value| value == "true" || value == "1");
        validate_url(url, allow_insecure)?;
        let client = redis::Client::open(url).map_err(|_| unavailable())?;
        let reads = client
            .get_multiplexed_async_connection()
            .await
            .map_err(|_| unavailable())?;
        let mut pubsub = client.get_async_pubsub().await.map_err(|_| unavailable())?;
        let topic = format!("{key}:changed");
        pubsub.subscribe(&topic).await.map_err(|_| unavailable())?;
        let store = Arc::new(Self {
            client,
            reads: Mutex::new(reads),
            transactions: Mutex::new(Vec::new()),
            // Every write targets this store's one channel key. Queue local
            // writers instead of making them invalidate each other's WATCH.
            slots: Semaphore::new(1),
            key: key.into(),
            topic,
            subscribed: Arc::new(AtomicBool::new(true)),
            listener: std::sync::Mutex::new(None),
        });
        // Validate schema before accepting traffic, including after an upgrade.
        store.read().await?;
        Ok((store, pubsub))
    }

    async fn read_connection(
        &self,
        connection: &mut MultiplexedConnection,
    ) -> Result<Registry, ApiError> {
        let fields: HashMap<String, String> = redis::cmd("HGETALL")
            .arg(&self.key)
            .query_async(connection)
            .await
            .map_err(|_| unavailable())?;
        if fields.is_empty() {
            return Ok(Registry::default());
        }
        let meta = fields.get("meta").ok_or_else(unavailable)?;
        let mut state: Registry = serde_json::from_str(meta).map_err(|_| unavailable())?;
        for (field, value) in fields.iter().filter(|(field, _)| field.as_str() != "meta") {
            let participant: Participant =
                serde_json::from_str(value).map_err(|_| unavailable())?;
            if field != &participant.id.to_string() {
                return Err(unavailable());
            }
            state
                .tokens
                .insert(participant.token.clone(), participant.id);
            state.participants.insert(participant.id, participant);
        }
        Ok(state)
    }

    async fn read(&self) -> Result<Registry, ApiError> {
        if !self.subscribed.load(Ordering::Acquire) {
            return Err(unavailable());
        }
        // Establish a fresh connection after an I/O error; never read a local
        // replica/cache as authoritative state.
        let mut connection = self.reads.lock().await.clone();
        match self.read_connection(&mut connection).await {
            Ok(state) => Ok(state),
            Err(_) => {
                let mut connection = self
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(|_| unavailable())?;
                let state = self.read_connection(&mut connection).await?;
                *self.reads.lock().await = connection;
                Ok(state)
            }
        }
    }

    async fn update<T>(
        &self,
        update: impl Fn(&mut Registry) -> Result<T, ApiError>,
    ) -> Result<(T, bool), ApiError> {
        if !self.subscribed.load(Ordering::Acquire) {
            return Err(unavailable());
        }
        let _permit = self.slots.acquire().await.map_err(|_| unavailable())?;
        // WATCH is connection-scoped. Never clone/share this connection while
        // a transaction is in progress; errors/cancellation discard it.
        let pooled = self.transactions.lock().await.pop();
        let mut connection = match pooled {
            Some(connection) => connection,
            None => self
                .client
                .get_multiplexed_async_connection()
                .await
                .map_err(|_| unavailable())?,
        };
        // AppState::update bounds this entire operation with IO_TIMEOUT. A burst
        // can legitimately lose more than 16 races before its deadline; only a
        // confirmed EXEC conflict is safe to retry, never an ambiguous I/O error.
        loop {
            if redis::cmd("WATCH")
                .arg(&self.key)
                .query_async::<()>(&mut connection)
                .await
                .is_err()
            {
                // A pooled connection may have died during an outage. Reconnect
                // only before any write was sent; never replay an ambiguous EXEC.
                connection = self
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(|_| unavailable())?;
                redis::cmd("WATCH")
                    .arg(&self.key)
                    .query_async::<()>(&mut connection)
                    .await
                    .map_err(|_| unavailable())?;
            }
            let mut state = self.read_connection(&mut connection).await?;
            let before = fields(&state)?;
            let visible = public_snapshot(&state);
            let connections = connection_ids(&state);
            let result = update(&mut state)?;
            let changed = visible != public_snapshot(&state);
            if changed {
                state.revision = state.revision.checked_add(1).ok_or_else(unavailable)?;
            }
            let notify = changed || connections != connection_ids(&state);
            let after = fields(&state)?;
            if before == after {
                redis::cmd("UNWATCH")
                    .query_async::<()>(&mut connection)
                    .await
                    .map_err(|_| unavailable())?;
                self.transactions.lock().await.push(connection);
                return Ok((result, false));
            }
            let mut transaction = redis::pipe();
            transaction.atomic();
            for (field, value) in &after {
                if before.get(field) != Some(value) {
                    transaction.cmd("HSET").arg(&self.key).arg(field).arg(value);
                }
            }
            for field in before.keys().filter(|field| !after.contains_key(*field)) {
                transaction.cmd("HDEL").arg(&self.key).arg(field);
            }
            if notify {
                transaction
                    .cmd("PUBLISH")
                    .arg(&self.topic)
                    .arg(state.revision);
            }
            let committed: Option<Vec<redis::Value>> = transaction
                .query_async(&mut connection)
                .await
                .map_err(|_| unavailable())?;
            if committed.is_some() {
                self.transactions.lock().await.push(connection);
                return Ok((result, notify));
            }
            tokio::task::yield_now().await;
        }
    }

    fn listen(&self, mut pubsub: PubSub, events: watch::Sender<()>) {
        let client = self.client.clone();
        let topic = self.topic.clone();
        let subscribed = self.subscribed.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut sink, mut messages) = pubsub.split();
                subscribed.store(true, Ordering::Release);
                // Pub/Sub is lossy: on every resubscription wake existing streams
                // to rebuild from current state, never replay old UI transitions.
                events.send_replace(());
                let mut ping = tokio::time::interval(Duration::from_secs(5));
                loop {
                    tokio::select! {
                        message = messages.next() => {
                            if message.is_none() { break; }
                            events.send_replace(());
                        },
                        _ = ping.tick() => {
                            if !matches!(tokio::time::timeout(IO_TIMEOUT, sink.ping::<Vec<String>>()).await, Ok(Ok(_))) { break; }
                        }
                    }
                }
                subscribed.store(false, Ordering::Release);
                events.send_replace(());
                drop(sink);
                drop(messages);
                loop {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    let reconnect = async {
                        let mut connection = client.get_async_pubsub().await?;
                        connection.subscribe(&topic).await?;
                        Ok::<_, redis::RedisError>(connection)
                    };
                    if let Ok(Ok(connection)) = tokio::time::timeout(IO_TIMEOUT, reconnect).await {
                        pubsub = connection;
                        break;
                    }
                }
            }
        });
        *self.listener.lock().unwrap() = Some(task.abort_handle());
    }
}

fn fields(state: &Registry) -> Result<HashMap<String, String>, ApiError> {
    let mut meta = state.clone();
    meta.participants.clear();
    meta.tokens.clear();
    let mut result = HashMap::from([(
        "meta".into(),
        serde_json::to_string(&meta).map_err(|_| unavailable())?,
    )]);
    for (id, participant) in &state.participants {
        result.insert(
            id.to_string(),
            serde_json::to_string(participant).map_err(|_| unavailable())?,
        );
    }
    Ok(result)
}

fn connection_ids(state: &Registry) -> std::collections::BTreeMap<Uuid, Option<Uuid>> {
    state
        .participants
        .iter()
        .filter(|(_, p)| p.monitor.is_none() && p.events.is_some())
        .map(|(id, p)| (*id, p.events))
        .collect()
}

impl AppState {
    pub async fn enable_shared_media(
        &mut self,
        environment: &RuntimeEnvironment,
    ) -> Result<(), String> {
        if let Some(url) = environment
            .get("VALKEY_URL")
            .filter(|value| !value.trim().is_empty())
        {
            self.connect_media_store(&url, KEY).await.map_err(|_| {
                "VALKEY_URL is invalid or shared media state is unavailable".to_owned()
            })?;
        }
        Ok(())
    }

    pub(super) async fn connect_media_store(
        &mut self,
        url: &str,
        key: &str,
    ) -> Result<(), ApiError> {
        let (store, subscriber) = tokio::time::timeout(IO_TIMEOUT, ValkeyStore::connect(url, key))
            .await
            .map_err(|_| unavailable())??;
        store.listen(subscriber, self.events.clone());
        self.store = Some(store);
        Ok(())
    }

    pub(super) async fn read<T>(
        &self,
        read: impl FnOnce(&Registry) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        if let Some(store) = &self.store {
            let state = tokio::time::timeout(IO_TIMEOUT, store.read())
                .await
                .map_err(|_| unavailable())??;
            read(&state)
        } else {
            read(&*self.registry.lock().await)
        }
    }

    pub(super) async fn update<T>(
        &self,
        update: impl Fn(&mut Registry) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        let (result, notify) = if let Some(store) = &self.store {
            tokio::time::timeout(IO_TIMEOUT, store.update(update))
                .await
                .map_err(|_| unavailable())??
        } else {
            let mut state = self.registry.lock().await;
            let mut next = state.clone();
            let result = update(&mut next)?;
            let changed = public_snapshot(&state) != public_snapshot(&next);
            let notify = changed || connection_ids(&state) != connection_ids(&next);
            if changed {
                next.revision += 1;
            }
            *state = next;
            (result, notify)
        };
        if notify {
            self.events.send_replace(());
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plaintext_compose_requires_opt_in_without_weakening_hosted_urls() {
        for (url, without_opt_in, with_opt_in) in [
            ("redis://valkey:6379", false, true),
            ("redis://localhost:6379", true, true),
            ("redis://127.0.0.1:6379", true, true),
            ("redis://[::1]:6379", true, true),
            ("redis://valkey.example:6379", false, false),
            ("redis://user:password@cache.example:6379", false, false),
            ("rediss://cache.example:6379", false, false),
            ("rediss://user:password@cache.example:6379", true, true),
            ("redis://valkey:6379#fragment", false, false),
            ("https://valkey:6379", false, false),
        ] {
            assert_eq!(validate_url(url, false).is_ok(), without_opt_in, "{url}");
            assert_eq!(validate_url(url, true).is_ok(), with_opt_in, "{url}");
        }
    }
}
