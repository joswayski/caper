//! Optional, durable SNS mobile push delivery for direct messages.
use crate::{ApiError, AppState, RuntimeEnvironment, auth::Principal};
use async_trait::async_trait;
use aws_sdk_sns::{Client, error::ProvideErrorMetadata};
use axum::{
    Extension, Json, Router,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::Notify;

const MAX_TOKEN_BYTES: usize = 4096;
const MAX_ATTEMPTS: i16 = 8;
const MAX_AGE_HOURS: i32 = 24;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) enum Platform {
    #[serde(rename = "fcm")]
    Fcm,
    #[serde(rename = "apns")]
    Apns,
    #[serde(rename = "apnsSandbox")]
    ApnsSandbox,
}

impl Platform {
    fn database(self) -> &'static str {
        match self {
            Self::Fcm => "fcm",
            Self::Apns => "apns",
            Self::ApnsSandbox => "apnsSandbox",
        }
    }
    fn message_key(self) -> &'static str {
        match self {
            Self::Fcm => "GCM",
            Self::Apns => "APNS",
            Self::ApnsSandbox => "APNS_SANDBOX",
        }
    }
}

#[derive(Clone)]
pub(crate) struct Push {
    pool: PgPool,
    applications: Arc<HashMap<&'static str, String>>,
    backend: Arc<dyn DeliveryBackend>,
    wake: Arc<Notify>,
}

#[async_trait]
trait DeliveryBackend: Send + Sync {
    async fn create_endpoint(&self, application: &str, token: &str)
    -> Result<String, BackendError>;
    async fn publish(&self, endpoint: &str, message: String) -> Result<(), BackendError>;
}

struct SnsBackend(Client);

#[derive(Debug, PartialEq, Eq)]
enum BackendError {
    Disabled,
    Invalid,
    Temporary,
}

#[async_trait]
impl DeliveryBackend for SnsBackend {
    async fn create_endpoint(
        &self,
        application: &str,
        token: &str,
    ) -> Result<String, BackendError> {
        let endpoint = self
            .0
            .create_platform_endpoint()
            .platform_application_arn(application)
            .token(token)
            .send()
            .await
            .map_err(|e| classify(e.code()))?
            .endpoint_arn
            .ok_or(BackendError::Temporary)?;
        self.0
            .set_endpoint_attributes()
            .endpoint_arn(&endpoint)
            .attributes("Enabled", "true")
            .send()
            .await
            .map_err(|e| classify(e.code()))?;
        Ok(endpoint)
    }

    async fn publish(&self, endpoint: &str, message: String) -> Result<(), BackendError> {
        self.0
            .publish()
            .target_arn(endpoint)
            .message(message)
            .message_structure("json")
            .send()
            .await
            .map(|_| ())
            .map_err(|e| classify(e.code()))
    }
}

fn classify(code: Option<&str>) -> BackendError {
    match code {
        Some("EndpointDisabled") | Some("NotFound") => BackendError::Disabled,
        Some("InvalidParameter") => BackendError::Invalid,
        _ => BackendError::Temporary,
    }
}

impl Push {
    /// Uses the standard AWS credential and region provider chain. The service
    /// remains available with an empty platform list when SNS is not configured.
    pub(crate) async fn from_env(
        pool: Option<&PgPool>,
        env: &RuntimeEnvironment,
    ) -> Result<Self, String> {
        let mut applications = HashMap::new();
        for (platform, name) in [
            ("fcm", "PUSH_FCM_APPLICATION_ARN"),
            ("apns", "PUSH_APNS_APPLICATION_ARN"),
            ("apnsSandbox", "PUSH_APNS_SANDBOX_APPLICATION_ARN"),
        ] {
            if let Some(value) = env.get(name).filter(|v| !v.trim().is_empty()) {
                applications.insert(platform, value);
            }
        }
        let pool = pool
            .ok_or("push configuration requires DATABASE_URL")?
            .clone();
        // A disabled provider must not query EC2 metadata or require AWS access.
        let config = if applications.is_empty() {
            aws_config::SdkConfig::builder()
                .behavior_version(aws_config::BehaviorVersion::latest())
                .region(aws_config::Region::new("us-east-1"))
                .build()
        } else {
            aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await
        };
        Ok(Self {
            pool,
            applications: Arc::new(applications),
            backend: Arc::new(SnsBackend(Client::new(&config))),
            wake: Arc::new(Notify::new()),
        })
    }

    fn application(&self, platform: Platform) -> Option<&str> {
        self.applications
            .get(platform.database())
            .map(String::as_str)
    }
}

/// Merge into the already account-authenticated router. The Principal extension
/// is deliberately required by every handler.
pub(crate) fn routes(push: Push) -> Router<AppState> {
    Router::new()
        .route("/api/push/config", get(config))
        .route("/api/push/devices", post(register).delete(remove))
        .layer(Extension(push))
}

#[derive(Deserialize)]
struct DeviceInput {
    platform: Platform,
    token: String,
}

async fn config(
    Extension(push): Extension<Push>,
    Extension(_principal): Extension<Principal>,
) -> Json<serde_json::Value> {
    let platforms: Vec<_> = [Platform::Fcm, Platform::Apns, Platform::ApnsSandbox]
        .into_iter()
        .filter(|p| push.application(*p).is_some())
        .collect();
    Json(json!({ "platforms": platforms }))
}

async fn register(
    Extension(push): Extension<Push>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<DeviceInput>,
) -> Result<StatusCode, ApiError> {
    let token = input.token.trim();
    if token.is_empty() || token.len() > MAX_TOKEN_BYTES {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid push token"));
    }
    let application = push
        .application(input.platform)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "push platform unavailable"))?;
    let endpoint = tokio::time::timeout(
        Duration::from_secs(10),
        push.backend.create_endpoint(application, token),
    )
    .await
    .map_err(|_| ApiError::new(StatusCode::BAD_GATEWAY, "push registration unavailable"))?
    .map_err(|_| ApiError::new(StatusCode::BAD_GATEWAY, "push registration unavailable"))?;
    let token_hash = Sha256::digest(token.as_bytes()).to_vec();
    let changed = sqlx::query("INSERT INTO public.push_devices (user_id, account_session_hash, platform, token_hash, endpoint_arn) SELECT $1,$2,$3,$4,$5 FROM public.account_sessions s WHERE s.token_hash=$2 AND s.user_id=$1 AND s.revoked_at IS NULL AND s.expires_at>now() ON CONFLICT (platform, token_hash) DO UPDATE SET user_id=EXCLUDED.user_id, account_session_hash=EXCLUDED.account_session_hash, endpoint_arn=EXCLUDED.endpoint_arn, updated_at=now() WHERE (SELECT created_at FROM public.account_sessions WHERE token_hash=push_devices.account_session_hash) <= (SELECT created_at FROM public.account_sessions WHERE token_hash=EXCLUDED.account_session_hash)")
        .bind(principal.user.id).bind(&principal.token_hash).bind(input.platform.database()).bind(token_hash).bind(endpoint)
        .execute(&push.pool).await.map_err(database_error)?;
    if changed.rows_affected() == 0 {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "push registration session expired",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn remove(
    Extension(push): Extension<Push>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<DeviceInput>,
) -> Result<StatusCode, ApiError> {
    let token_hash = Sha256::digest(input.token.trim().as_bytes()).to_vec();
    // Do not delete the SNS endpoint: a concurrent new login may already own
    // the same token. Without a DB registration it cannot receive new pushes.
    sqlx::query("DELETE FROM public.push_devices WHERE user_id=$1 AND account_session_hash=$2 AND platform=$3 AND token_hash=$4")
        .bind(principal.user.id).bind(&principal.token_hash).bind(input.platform.database()).bind(token_hash)
        .execute(&push.pool).await.map_err(database_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn database_error(_: sqlx::Error) -> ApiError {
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "push unavailable")
}

/// Call inside the message transaction. This is idempotent and only inserts for
/// a direct conversation's other participant; ordinary channels are a no-op.
pub(crate) async fn enqueue(
    tx: &mut Transaction<'_, Postgres>,
    channel_id: i64,
    author_id: i64,
    message_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO public.push_notifications (channel_id, recipient_user_id, message_id) SELECT dc.channel_id, CASE WHEN dc.low_user_id=$2 THEN dc.high_user_id ELSE dc.low_user_id END, $3 FROM public.direct_conversations dc WHERE dc.channel_id=$1 AND $2 IN (dc.low_user_id,dc.high_user_id) ON CONFLICT (message_id, recipient_user_id) DO NOTHING")
        .bind(channel_id).bind(author_id).bind(message_id).execute(&mut **tx).await?;
    Ok(())
}

pub(crate) fn spawn_worker(push: Push) {
    if push.applications.is_empty() {
        return;
    }
    tokio::spawn(async move {
        loop {
            for _ in 0..64 {
                match work_once(&push).await {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(_) => {
                        tracing::warn!(
                            event_name = "push_worker_failed",
                            "push worker iteration failed"
                        );
                        break;
                    }
                }
            }
            tokio::select! { _ = push.wake.notified() => {}, _ = tokio::time::sleep(Duration::from_secs(5)) => {} }
        }
    });
}

async fn work_once(push: &Push) -> Result<bool, sqlx::Error> {
    // Expansion and marking are atomic. Only endpoints backed by a currently
    // valid, nonrevoked account session become deliveries.
    let mut tx = push.pool.begin().await?;
    let notification: Option<i64> = sqlx::query_scalar("SELECT id FROM public.push_notifications WHERE expanded_at IS NULL AND created_at > now()-make_interval(hours=>$1) ORDER BY id FOR UPDATE SKIP LOCKED LIMIT 1")
        .bind(MAX_AGE_HOURS).fetch_optional(&mut *tx).await?;
    if let Some(id) = notification {
        sqlx::query("INSERT INTO public.push_deliveries (notification_id,device_id) SELECT n.id,d.id FROM public.push_notifications n JOIN public.push_devices d ON d.user_id=n.recipient_user_id JOIN public.account_sessions s ON s.token_hash=d.account_session_hash AND s.user_id=d.user_id WHERE n.id=$1 AND s.revoked_at IS NULL AND s.expires_at>now() ON CONFLICT DO NOTHING").bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE public.push_notifications SET expanded_at=now() WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE public.push_notifications SET expanded_at=now() WHERE expanded_at IS NULL AND created_at <= now()-make_interval(hours=>$1)").bind(MAX_AGE_HOURS).execute(&mut *tx).await?;
    tx.commit().await?;

    sqlx::query("UPDATE public.push_deliveries pd SET abandoned_at=now(),locked_at=NULL FROM public.push_devices d,public.push_notifications n WHERE d.id=pd.device_id AND n.id=pd.notification_id AND pd.delivered_at IS NULL AND pd.abandoned_at IS NULL AND (n.recipient_user_id<>d.user_id OR n.created_at<=now()-interval '24 hours' OR NOT EXISTS (SELECT 1 FROM public.account_sessions s JOIN public.users u ON u.id=s.user_id WHERE s.token_hash=d.account_session_hash AND s.user_id=d.user_id AND s.revoked_at IS NULL AND s.expires_at>now() AND u.deleted_at IS NULL))")
        .execute(&push.pool).await?;
    let platforms: Vec<&str> = push.applications.keys().copied().collect();
    let row = sqlx::query_as::<_, (i64,i64,String,String,String,String,chrono::DateTime<chrono::Utc>)>("UPDATE public.push_deliveries pd SET locked_at=now(), attempts=attempts+1 FROM (SELECT pd.notification_id,pd.device_id FROM public.push_deliveries pd JOIN public.push_devices d ON d.id=pd.device_id WHERE pd.delivered_at IS NULL AND pd.abandoned_at IS NULL AND pd.attempts<8 AND pd.available_at<=now() AND d.platform=ANY($1) AND (pd.locked_at IS NULL OR pd.locked_at<now()-interval '2 minutes') ORDER BY pd.available_at FOR UPDATE OF pd SKIP LOCKED LIMIT 1) claim, public.push_devices d, public.push_notifications n, public.channels c, public.messages m, public.account_sessions s WHERE pd.notification_id=claim.notification_id AND pd.device_id=claim.device_id AND d.id=pd.device_id AND n.id=pd.notification_id AND n.recipient_user_id=d.user_id AND c.id=n.channel_id AND m.id=n.message_id AND s.token_hash=d.account_session_hash AND s.user_id=d.user_id AND s.revoked_at IS NULL AND s.expires_at>now() RETURNING pd.notification_id,pd.device_id,d.platform,d.endpoint_arn,c.external_id,m.external_id,d.updated_at")
        .bind(platforms)
        .fetch_optional(&push.pool).await?;
    let Some((
        notification_id,
        device_id,
        platform,
        endpoint,
        conversation,
        message,
        registered_at,
    )) = row
    else {
        return Ok(notification.is_some());
    };
    let platform = match platform.as_str() {
        "fcm" => Platform::Fcm,
        "apns" => Platform::Apns,
        _ => Platform::ApnsSandbox,
    };
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        push.backend
            .publish(&endpoint, payload(platform, &conversation, &message)),
    )
    .await
    .unwrap_or(Err(BackendError::Temporary));
    match result {
        Ok(()) => {
            sqlx::query("UPDATE public.push_deliveries SET delivered_at=now(),locked_at=NULL WHERE notification_id=$1 AND device_id=$2").bind(notification_id).bind(device_id).execute(&push.pool).await?;
        }
        Err(BackendError::Disabled | BackendError::Invalid) => {
            // A late failure for an old registration cannot delete a refreshed
            // or reassigned registration using the same physical device token.
            sqlx::query("UPDATE public.push_deliveries SET abandoned_at=now(),locked_at=NULL WHERE notification_id=$1 AND device_id=$2")
                .bind(notification_id).bind(device_id).execute(&push.pool).await?;
            sqlx::query("DELETE FROM public.push_devices WHERE id=$1 AND updated_at=$2")
                .bind(device_id)
                .bind(registered_at)
                .execute(&push.pool)
                .await?;
        }
        Err(BackendError::Temporary) => {
            sqlx::query("UPDATE public.push_deliveries SET locked_at=NULL, available_at=now()+make_interval(secs=>LEAST(3600, power(2,attempts)::int*5)), abandoned_at=CASE WHEN attempts >= $3 THEN now() END WHERE notification_id=$1 AND device_id=$2").bind(notification_id).bind(device_id).bind(MAX_ATTEMPTS).execute(&push.pool).await?;
        }
    }
    Ok(true)
}

fn payload(platform: Platform, conversation: &str, message: &str) -> String {
    let inner = match platform {
        Platform::Fcm => {
            // Data-only lets Android check local opt-in/account state before
            // displaying; notification messages bypass that check in background.
            json!({"fcmV1Message":{"message":{"android":{"priority":"HIGH","ttl":"86400s"},"data":{"conversationId":conversation,"messageId":message}}}})
        }
        _ => {
            json!({"aps":{"alert":{"title":"Caper","body":"You have a new direct message."},"sound":"default"},"conversationId":conversation,"messageId":message})
        }
    };
    json!({"default": "You have a new direct message in Caper.", platform.message_key(): inner.to_string()}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payloads_are_private_and_platform_specific() {
        let fcm: serde_json::Value =
            serde_json::from_str(&payload(Platform::Fcm, "conversation", "message")).unwrap();
        let fcm_inner: serde_json::Value =
            serde_json::from_str(fcm["GCM"].as_str().unwrap()).unwrap();
        assert_eq!(
            fcm_inner["fcmV1Message"]["message"]["data"]["conversationId"],
            "conversation"
        );
        assert!(
            fcm_inner["fcmV1Message"]["message"]
                .get("notification")
                .is_none()
        );
        assert_eq!(
            fcm_inner["fcmV1Message"]["message"]["android"]["priority"],
            "HIGH"
        );
        let apns: serde_json::Value =
            serde_json::from_str(&payload(Platform::Apns, "conversation", "message")).unwrap();
        let apns_inner: serde_json::Value =
            serde_json::from_str(apns["APNS"].as_str().unwrap()).unwrap();
        assert_eq!(
            apns_inner["aps"]["alert"]["body"],
            "You have a new direct message."
        );
        for (platform, key) in [
            (Platform::Fcm, "GCM"),
            (Platform::Apns, "APNS"),
            (Platform::ApnsSandbox, "APNS_SANDBOX"),
        ] {
            let body = payload(platform, "conversation", "message");
            let envelope: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(
                envelope["default"].as_str(),
                Some("You have a new direct message in Caper.")
            );
            let inner: serde_json::Value =
                serde_json::from_str(envelope[key].as_str().unwrap()).unwrap();
            assert!(
                inner.is_object(),
                "platform payload remains a JSON-encoded string"
            );
            assert!(!body.contains("sender"));
            assert!(!body.contains("profile"));
            assert!(!body.contains("content"));
        }
    }

    #[test]
    fn classifies_permanent_endpoint_errors() {
        assert_eq!(classify(Some("EndpointDisabled")), BackendError::Disabled);
        assert_eq!(classify(Some("NotFound")), BackendError::Disabled);
        assert_eq!(classify(Some("InvalidParameter")), BackendError::Invalid);
        assert_eq!(classify(Some("Throttled")), BackendError::Temporary);
    }

    #[derive(Default)]
    struct FakeBackend {
        calls: std::sync::Mutex<Vec<String>>,
        failures: std::sync::Mutex<std::collections::VecDeque<BackendError>>,
    }

    #[async_trait]
    impl DeliveryBackend for FakeBackend {
        async fn create_endpoint(&self, _: &str, _: &str) -> Result<String, BackendError> {
            Ok("fake-endpoint".into())
        }
        async fn publish(&self, endpoint: &str, _: String) -> Result<(), BackendError> {
            self.calls.lock().unwrap().push(endpoint.into());
            self.failures
                .lock()
                .unwrap()
                .pop_front()
                .map_or(Ok(()), Err)
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires disposable loopback DATABASE_URL"]
    async fn retries_are_durable_and_reassigned_or_revoked_devices_are_not_delivered(pool: PgPool) {
        let mut users = Vec::new();
        for name in ["sender", "recipient", "nextlogin"] {
            let user: i64 = sqlx::query_scalar("INSERT INTO users (external_id,username,display_name) VALUES ($1,$1,$1) RETURNING id")
                .bind(name).fetch_one(&pool).await.unwrap();
            sqlx::query("INSERT INTO account_sessions (token_hash,user_id,expires_at) VALUES ($1,$2,now()+interval '1 day')")
                .bind(name.as_bytes()).bind(user).execute(&pool).await.unwrap();
            users.push(user);
        }
        let session: i64 = sqlx::query_scalar("INSERT INTO chat_sessions (external_id,token_hash,user_id,name) VALUES ('chat',$1,$2,'sender') RETURNING id")
            .bind(b"chat".as_slice()).bind(users[0]).fetch_one(&pool).await.unwrap();
        let channel: i64 = sqlx::query_scalar("INSERT INTO channels (external_id,name,private) VALUES ('conversation','direct',true) RETURNING id")
            .fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO direct_conversations (channel_id,low_user_id,high_user_id) VALUES ($1,$2,$3)")
            .bind(channel).bind(users[0]).bind(users[1]).execute(&pool).await.unwrap();
        let mut message_ids = Vec::new();
        for seq in 1..=3_i64 {
            let message: i64 = sqlx::query_scalar("INSERT INTO messages (external_id,channel_id,session_id,client_message_id,request_hash,channel_seq,payload) VALUES ($1,$2,$3,$4,$5,$6,'{}') RETURNING id")
                .bind(format!("message{seq}")).bind(channel).bind(session).bind(uuid::Uuid::new_v4())
                .bind(b"hash".as_slice()).bind(seq).fetch_one(&pool).await.unwrap();
            message_ids.push(message);
        }
        let device: i64 = sqlx::query_scalar("INSERT INTO push_devices (user_id,account_session_hash,platform,token_hash,endpoint_arn) VALUES ($1,$2,'fcm',$3,'recipient-device') RETURNING id")
            .bind(users[1]).bind(b"recipient".as_slice()).bind(b"device".as_slice()).fetch_one(&pool).await.unwrap();
        // The sender's own opted-in device must not be expanded into a delivery.
        sqlx::query("INSERT INTO push_devices (user_id,account_session_hash,platform,token_hash,endpoint_arn) VALUES ($1,$2,'fcm',$3,'sender-device')")
            .bind(users[0]).bind(b"sender".as_slice()).bind(b"otherdevice".as_slice()).execute(&pool).await.unwrap();
        let backend = Arc::new(FakeBackend::default());
        let push = Push {
            pool: pool.clone(),
            applications: Arc::new(HashMap::from([("fcm", "fake-app".into())])),
            backend: backend.clone(),
            wake: Arc::new(Notify::new()),
        };
        let mut tx = pool.begin().await.unwrap();
        enqueue(&mut tx, channel, users[0], message_ids[0])
            .await
            .unwrap();
        enqueue(&mut tx, channel, users[0], message_ids[0])
            .await
            .unwrap();
        tx.commit().await.unwrap();
        backend
            .failures
            .lock()
            .unwrap()
            .push_back(BackendError::Temporary);
        assert!(work_once(&push).await.unwrap());
        assert_eq!(*backend.calls.lock().unwrap(), ["recipient-device"]);
        assert!(
            !work_once(&push).await.unwrap(),
            "backoff prevents immediate retries"
        );
        sqlx::query("UPDATE push_deliveries SET available_at=now()-interval '1 second'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(work_once(&push).await.unwrap());
        assert_eq!(
            *backend.calls.lock().unwrap(),
            ["recipient-device", "recipient-device"]
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM push_deliveries WHERE delivered_at IS NOT NULL"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        assert!(
            !work_once(&push).await.unwrap(),
            "committed delivery is not republished"
        );

        // Expand without sending, then simulate the same token logging into a
        // different account. An old queued DM cannot follow the device owner.
        let mut tx = pool.begin().await.unwrap();
        for message in &message_ids[1..] {
            enqueue(&mut tx, channel, users[0], *message).await.unwrap();
        }
        tx.commit().await.unwrap();
        sqlx::query("INSERT INTO push_deliveries (notification_id,device_id) SELECT id,$1 FROM push_notifications WHERE expanded_at IS NULL")
            .bind(device).execute(&pool).await.unwrap();
        sqlx::query("UPDATE push_notifications SET expanded_at=now()")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE push_devices SET user_id=$1,account_session_hash=$2,updated_at=now() WHERE id=$3")
            .bind(users[2]).bind(b"nextlogin".as_slice()).bind(device).execute(&pool).await.unwrap();
        assert!(!work_once(&push).await.unwrap());
        assert_eq!(backend.calls.lock().unwrap().len(), 2);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM push_deliveries WHERE abandoned_at IS NOT NULL"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            2
        );

        // A retry for the original owner is also suppressed by session revocation.
        sqlx::query("UPDATE push_devices SET user_id=$1,account_session_hash=$2 WHERE id=$3")
            .bind(users[1])
            .bind(b"recipient".as_slice())
            .bind(device)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE push_deliveries SET abandoned_at=NULL WHERE delivered_at IS NULL")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE account_sessions SET revoked_at=now() WHERE user_id=$1")
            .bind(users[1])
            .execute(&pool)
            .await
            .unwrap();
        assert!(!work_once(&push).await.unwrap());
        assert_eq!(backend.calls.lock().unwrap().len(), 2);
    }
}
