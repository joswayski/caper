use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

use crate::{AppState, accounts};

const SIGNATURE_TOLERANCE_MS: i64 = 180_000;

#[derive(Deserialize)]
struct Event {
    id: String,
    #[serde(rename = "event")]
    kind: String,
    data: serde_json::Value,
}

#[derive(Deserialize)]
struct UpdatedUser {
    id: String,
    email: String,
    email_verified: bool,
    updated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct DeletedUser {
    id: String,
    updated_at: DateTime<Utc>,
}

pub async fn handle(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let Some(secret) = state.config.workos_webhook_secret.as_deref() else {
        return StatusCode::SERVICE_UNAVAILABLE;
    };
    let Some(signature) = headers
        .get("workos-signature")
        .and_then(|value| value.to_str().ok())
    else {
        return StatusCode::UNAUTHORIZED;
    };
    if !verify_signature(signature, &body, secret, now_ms()) {
        return StatusCode::UNAUTHORIZED;
    }
    let Ok(event) = serde_json::from_slice::<Event>(&body) else {
        return StatusCode::BAD_REQUEST;
    };
    let Some(pool) = state.database.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE;
    };
    let Ok(mut tx) = pool.begin().await else {
        return StatusCode::SERVICE_UNAVAILABLE;
    };
    match accounts::record_workos_event(&mut tx, &event.id, &event.kind).await {
        Ok(false) => return commit(tx).await,
        Ok(true) => {}
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE,
    }
    let result = match event.kind.as_str() {
        "user.updated" => match serde_json::from_value::<UpdatedUser>(event.data) {
            Ok(user) => {
                let email = user.email_verified.then_some(user.email.as_str());
                accounts::apply_workos_update(&mut tx, &user.id, email, user.updated_at).await
            }
            Err(_) => return StatusCode::BAD_REQUEST,
        },
        "user.deleted" => match serde_json::from_value::<DeletedUser>(event.data) {
            Ok(user) => accounts::apply_workos_deletion(&mut tx, &user.id, user.updated_at).await,
            Err(_) => return StatusCode::BAD_REQUEST,
        },
        _ => Ok(()),
    };
    if result.is_err() {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    commit(tx).await
}

async fn commit(tx: sqlx::Transaction<'_, sqlx::Postgres>) -> StatusCode {
    if tx.commit().await.is_ok() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

fn verify_signature(header: &str, body: &[u8], secret: &str, now: i64) -> bool {
    let mut timestamp = None;
    let mut signatures = Vec::new();
    for part in header.split(',').map(str::trim) {
        if let Some(value) = part.strip_prefix("t=") {
            timestamp = value.parse::<i64>().ok();
        } else if let Some(value) = part.strip_prefix("v1=") {
            signatures.push(value);
        }
    }
    let Some(timestamp) = timestamp else {
        return false;
    };
    if now.abs_diff(timestamp) > SIGNATURE_TOLERANCE_MS.unsigned_abs() {
        return false;
    }
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    signatures
        .into_iter()
        .filter_map(|signature| hex::decode(signature).ok())
        .any(|signature| mac.clone().verify_slice(&signature).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_raw_body_signature_and_timestamp() {
        let body = br#"{"event":"user.updated"}"#;
        let timestamp = 1_000_000;
        let mut mac = Hmac::<Sha256>::new_from_slice(b"secret").unwrap();
        mac.update(format!("{timestamp}.").as_bytes());
        mac.update(body);
        let header = format!(
            "t={timestamp}, v1={}",
            hex::encode(mac.finalize().into_bytes())
        );
        assert!(verify_signature(&header, body, "secret", timestamp));
        assert!(!verify_signature(&header, b"changed", "secret", timestamp));
        assert!(!verify_signature(
            &header,
            body,
            "secret",
            timestamp + SIGNATURE_TOLERANCE_MS + 1
        ));
    }
}
