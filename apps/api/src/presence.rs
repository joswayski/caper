//! Ephemeral account presence backed by one bounded Valkey hash per user.
//!
//! Each gateway socket owns a distinct hash field. Leases use Valkey's clock so
//! gateways do not need synchronized clocks, while activity age preserves the
//! browser's activity baseline across background heartbeats.

use crate::ApiError;
use axum::http::StatusCode;
use redis::aio::MultiplexedConnection;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

const IO_TIMEOUT: Duration = Duration::from_secs(3);
const LEASE: Duration = Duration::from_secs(40);
const KEY_GRACE: Duration = Duration::from_secs(40);
const MAX_SESSIONS: u64 = 32;

const RENEW: &str = r#"
local clock = redis.call('TIME')
local now = tonumber(clock[1]) * 1000 + math.floor(tonumber(clock[2]) / 1000)
local previous = redis.call('HGET', KEYS[1], '__status') or 'offline'
local entries = redis.call('HGETALL', KEYS[1])
local sessions = 0

for i = 1, #entries, 2 do
    local connection = entries[i]
    if connection ~= '__status' then
        local lease, activity = string.match(entries[i + 1], '^(%d+):(%d+)$')
        if not lease or tonumber(lease) <= now then
            redis.call('HDEL', KEYS[1], connection)
        else
            sessions = sessions + 1
        end
    end
end

local rejected = false
if redis.call('HEXISTS', KEYS[1], ARGV[1]) == 0 and sessions >= tonumber(ARGV[8]) then
    rejected = true
else
    local age = math.min(tonumber(ARGV[2]), tonumber(ARGV[3]))
    local activity = math.max(0, now - age)
    local value = string.format('%.0f:%.0f', now + tonumber(ARGV[4]), activity)
    redis.call('HSET', KEYS[1], ARGV[1], value)
end

local current = 'idle'
entries = redis.call('HGETALL', KEYS[1])
for i = 1, #entries, 2 do
    if entries[i] ~= '__status' then
        local _, activity = string.match(entries[i + 1], '^(%d+):(%d+)$')
        if activity and now - tonumber(activity) < tonumber(ARGV[3]) then
            current = 'online'
            break
        end
    end
end

redis.call('HSET', KEYS[1], '__status', current)
if not rejected then redis.call('PEXPIRE', KEYS[1], ARGV[5]) end
if current ~= previous then
    redis.call('PUBLISH', ARGV[6], cjson.encode({userId = ARGV[7], status = current}))
end
if rejected then return -1 end
return 1
"#;

const STATUS: &str = r#"
local clock = redis.call('TIME')
local now = tonumber(clock[1]) * 1000 + math.floor(tonumber(clock[2]) / 1000)
local previous = redis.call('HGET', KEYS[1], '__status')
local entries = redis.call('HGETALL', KEYS[1])
local connected = false
local active = false

for i = 1, #entries, 2 do
    local connection = entries[i]
    if connection ~= '__status' then
        local lease, activity = string.match(entries[i + 1], '^(%d+):(%d+)$')
        if not lease or tonumber(lease) <= now then
            redis.call('HDEL', KEYS[1], connection)
        else
            connected = true
            if now - tonumber(activity) < tonumber(ARGV[1]) then active = true end
        end
    end
end

local current = 'offline'
if active then
    current = 'online'
elseif connected then
    current = 'idle'
end

if current == 'offline' then
    redis.call('DEL', KEYS[1])
else
    redis.call('HSET', KEYS[1], '__status', current)
end
if previous and current ~= previous then
    redis.call('PUBLISH', ARGV[2], cjson.encode({userId = ARGV[3], status = current}))
end
return current
"#;

fn unavailable() -> ApiError {
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "presence unavailable")
}

fn milliseconds(duration: Duration) -> u64 {
    duration.as_millis().min(i64::MAX as u128) as u64
}

fn key(user: &str) -> String {
    // The user ID is the hash tag, so every key touched by one invocation is
    // cluster-compatible without putting different users in the same slot.
    format!("caper:presence:v1:{{{user}}}:sessions")
}

pub(crate) fn topic(user: &str) -> String {
    format!("caper:presence:v1:{user}")
}

#[derive(Clone)]
pub(crate) struct Presence {
    shared: Arc<Shared>,
    idle_timeout_ms: u64,
}

struct Shared {
    client: redis::Client,
    connection: Mutex<MultiplexedConnection>,
}

impl Presence {
    pub(crate) async fn new(
        broker: &redis::Client,
        idle_timeout: Duration,
    ) -> Result<Self, ApiError> {
        let connection = Self::connect(broker).await?;
        Ok(Self {
            shared: Arc::new(Shared {
                client: broker.clone(),
                connection: Mutex::new(connection),
            }),
            idle_timeout_ms: milliseconds(idle_timeout),
        })
    }

    async fn connect(broker: &redis::Client) -> Result<MultiplexedConnection, ApiError> {
        let config = redis::AsyncConnectionConfig::new()
            .set_connection_timeout(IO_TIMEOUT)
            .set_response_timeout(IO_TIMEOUT);
        let mut connection = broker
            .get_multiplexed_async_connection_with_config(&config)
            .await
            .map_err(|_| unavailable())?;
        redis::cmd("PING")
            .query_async::<()>(&mut connection)
            .await
            .map_err(|_| unavailable())?;
        Ok(connection)
    }

    async fn connection(&self) -> MultiplexedConnection {
        self.shared.connection.lock().await.clone()
    }

    async fn repair(&self) {
        // Serialize repairs, then check whether another failed caller already
        // replaced the shared handle. Never replay the operation whose result
        // became unknown: only prepare a connection for the next call.
        let mut shared = self.shared.connection.lock().await;
        let mut current = shared.clone();
        if redis::cmd("PING")
            .query_async::<()>(&mut current)
            .await
            .is_ok()
        {
            return;
        }
        if let Ok(connection) = Self::connect(&self.shared.client).await {
            *shared = connection;
        }
    }

    pub(crate) async fn renew(
        &self,
        user: &str,
        connection: &str,
        activity_age_ms: u64,
    ) -> Result<(), ApiError> {
        let mut broker = self.connection().await;
        let renewed = match redis::Script::new(RENEW)
            .key(key(user))
            .arg(connection)
            // Ages beyond the idle boundary are equivalent and can exceed Lua's
            // exact integer range after a long-suspended browser resumes.
            .arg(activity_age_ms.min(self.idle_timeout_ms))
            .arg(self.idle_timeout_ms)
            .arg(milliseconds(LEASE))
            .arg(milliseconds(LEASE.saturating_add(KEY_GRACE)))
            .arg(topic(user))
            .arg(user)
            .arg(MAX_SESSIONS)
            .invoke_async::<i64>(&mut broker)
            .await
        {
            Ok(renewed) => renewed,
            Err(_) => {
                self.repair().await;
                return Err(unavailable());
            }
        };
        if renewed < 0 {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "too many presence sessions",
            ));
        }
        Ok(())
    }

    pub(crate) async fn statuses(&self, users: &[String]) -> Result<Vec<Value>, ApiError> {
        if users.is_empty() {
            return Ok(Vec::new());
        }
        let mut pipeline = redis::pipe();
        for user in users {
            let mut command = redis::cmd("EVAL");
            command
                .arg(STATUS)
                .arg(1)
                .arg(key(user))
                .arg(self.idle_timeout_ms)
                .arg(topic(user))
                .arg(user);
            pipeline.add_command(command);
        }
        let mut broker = self.connection().await;
        let statuses: Vec<String> = match pipeline.query_async(&mut broker).await {
            Ok(statuses) => statuses,
            Err(_) => {
                self.repair().await;
                return Err(unavailable());
            }
        };
        Ok(users
            .iter()
            .zip(statuses)
            .map(|(user, status)| json!({"userId": user, "status": status}))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    #[test]
    fn keys_are_cluster_scoped_and_topics_are_per_user() {
        assert_eq!(key("user123"), "caper:presence:v1:{user123}:sessions");
        assert_eq!(topic("user123"), "caper:presence:v1:user123");
    }

    #[test]
    fn key_ttl_is_bounded_independently_of_idle_timeout() {
        let idle_timeout = Duration::from_secs(600);
        assert!(LEASE.saturating_add(KEY_GRACE) < idle_timeout);
        assert_eq!(milliseconds(LEASE.saturating_add(KEY_GRACE)), 80_000);
    }

    async fn disposable() -> (Presence, redis::Client, String) {
        let url = std::env::var("TEST_VALKEY_URL")
            .expect("set TEST_VALKEY_URL to a disposable Valkey instance");
        let broker = redis::Client::open(url).unwrap();
        let user = format!("presence{}", uuid::Uuid::new_v4().simple());
        let presence = Presence::new(&broker, Duration::from_millis(100))
            .await
            .unwrap();
        (presence, broker, user)
    }

    async fn kill_connection(presence: &Presence, broker: &redis::Client) {
        let mut connection = presence.connection().await;
        let id: i64 = redis::cmd("CLIENT")
            .arg("ID")
            .query_async(&mut connection)
            .await
            .unwrap();
        let mut admin = broker.get_multiplexed_async_connection().await.unwrap();
        assert_eq!(
            redis::cmd("CLIENT")
                .arg("KILL")
                .arg("ID")
                .arg(id)
                .query_async::<i64>(&mut admin)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    #[ignore = "requires disposable TEST_VALKEY_URL"]
    async fn aggregates_independent_sessions_and_preserves_activity_age() {
        let (presence, broker, user) = disposable().await;
        assert_eq!(
            presence
                .statuses(std::slice::from_ref(&user))
                .await
                .unwrap(),
            vec![json!({"userId": user, "status": "offline"})]
        );

        presence.renew(&user, "old-socket", 80).await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(
            presence
                .statuses(std::slice::from_ref(&user))
                .await
                .unwrap()[0]["status"],
            "idle"
        );

        presence.renew(&user, "new-socket", 0).await.unwrap();
        assert_eq!(
            presence
                .statuses(std::slice::from_ref(&user))
                .await
                .unwrap()[0]["status"],
            "online"
        );

        let mut connection = broker.get_multiplexed_async_connection().await.unwrap();
        redis::cmd("HSET")
            .arg(key(&user))
            .arg("old-socket")
            .arg("0:0")
            .query_async::<()>(&mut connection)
            .await
            .unwrap();
        // Expiring the old socket must not erase the independently leased one.
        assert_eq!(
            presence
                .statuses(std::slice::from_ref(&user))
                .await
                .unwrap()[0]["status"],
            "online"
        );
        redis::cmd("DEL")
            .arg(key(&user))
            .query_async::<()>(&mut connection)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires disposable TEST_VALKEY_URL"]
    async fn pipelines_ordered_statuses_and_rejects_a_thirty_third_session() {
        let (presence, broker, user) = disposable().await;
        for index in 0..MAX_SESSIONS {
            presence
                .renew(&user, &format!("socket-{index}"), 0)
                .await
                .unwrap();
        }
        let error = presence
            .renew(&user, "socket-over-limit", 0)
            .await
            .unwrap_err();
        assert_eq!(error.status, StatusCode::TOO_MANY_REQUESTS);

        let offline = format!("presence{}", uuid::Uuid::new_v4().simple());
        assert_eq!(
            presence
                .statuses(&[offline.clone(), user.clone()])
                .await
                .unwrap(),
            vec![
                json!({"userId": offline, "status": "offline"}),
                json!({"userId": user, "status": "online"}),
            ]
        );
        let mut connection = broker.get_multiplexed_async_connection().await.unwrap();
        redis::cmd("DEL")
            .arg(key(&user))
            .query_async::<()>(&mut connection)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires disposable TEST_VALKEY_URL"]
    async fn publishes_only_aggregate_transitions_including_sweep_to_offline() {
        let (presence, broker, user) = disposable().await;
        let mut subscriber = broker.get_async_pubsub().await.unwrap();
        subscriber.subscribe(topic(&user)).await.unwrap();
        let mut messages = subscriber.on_message();

        presence.renew(&user, "socket", 0).await.unwrap();
        let online = tokio::time::timeout(Duration::from_secs(1), messages.next())
            .await
            .unwrap()
            .unwrap()
            .get_payload::<String>()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&online).unwrap(),
            json!({"userId": user, "status": "online"})
        );

        presence.renew(&user, "socket", 0).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), messages.next())
                .await
                .is_err()
        );

        let mut connection = broker.get_multiplexed_async_connection().await.unwrap();
        redis::cmd("HSET")
            .arg(key(&user))
            .arg("socket")
            .arg("0:0")
            .query_async::<()>(&mut connection)
            .await
            .unwrap();
        assert_eq!(
            presence
                .statuses(std::slice::from_ref(&user))
                .await
                .unwrap()[0]["status"],
            "offline"
        );
        let offline = tokio::time::timeout(Duration::from_secs(1), messages.next())
            .await
            .unwrap()
            .unwrap()
            .get_payload::<String>()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&offline).unwrap(),
            json!({"userId": user, "status": "offline"})
        );
    }

    #[tokio::test]
    #[ignore = "requires disposable TEST_VALKEY_URL with CLIENT KILL permission"]
    async fn repairs_after_failed_reads_and_renews_without_replaying_them() {
        let (presence, broker, user) = disposable().await;

        kill_connection(&presence, &broker).await;
        assert!(
            presence
                .statuses(std::slice::from_ref(&user))
                .await
                .is_err()
        );
        presence.renew(&user, "socket", 0).await.unwrap();
        assert_eq!(
            presence
                .statuses(std::slice::from_ref(&user))
                .await
                .unwrap()[0]["status"],
            "online"
        );

        kill_connection(&presence, &broker).await;
        assert!(presence.renew(&user, "socket", 0).await.is_err());
        assert_eq!(
            presence
                .statuses(std::slice::from_ref(&user))
                .await
                .unwrap()[0]["status"],
            "online"
        );

        let mut connection = broker.get_multiplexed_async_connection().await.unwrap();
        redis::cmd("DEL")
            .arg(key(&user))
            .query_async::<()>(&mut connection)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires disposable TEST_VALKEY_URL"]
    async fn stored_key_ttl_is_bounded_with_a_ten_minute_idle_timeout() {
        let url = std::env::var("TEST_VALKEY_URL")
            .expect("set TEST_VALKEY_URL to a disposable Valkey instance");
        let broker = redis::Client::open(url).unwrap();
        let presence = Presence::new(&broker, Duration::from_secs(600))
            .await
            .unwrap();
        let user = format!("presence{}", uuid::Uuid::new_v4().simple());
        presence.renew(&user, "socket", 0).await.unwrap();

        let mut connection = broker.get_multiplexed_async_connection().await.unwrap();
        let ttl: i64 = redis::cmd("PTTL")
            .arg(key(&user))
            .query_async(&mut connection)
            .await
            .unwrap();
        assert!(ttl > 0 && ttl <= 80_000, "unexpected TTL: {ttl}");
        redis::cmd("DEL")
            .arg(key(&user))
            .query_async::<()>(&mut connection)
            .await
            .unwrap();
    }
}
