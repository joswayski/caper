//! Account notification settings and space, channel and DM overrides.
//! Levels and mutes are stored in place; `muted_until = 'infinity'` is forever.
use super::{Level, bad_request, database_error, pool};
use crate::{ApiError, AppState, auth::Principal, spaces::channel_access};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, put},
};
use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use serde_json::{Map, Value, json};
use sqlx::PgPool;

const LONGEST_MUTE: TimeDelta = TimeDelta::days(365);

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/notifications/settings",
            get(read_settings).put(update_settings),
        )
        .route("/api/spaces/{space}/notifications", put(update_space))
        .route(
            "/api/spaces/{space}/channels/{channel}/notifications",
            put(update_channel),
        )
        .route("/api/dms/{conversation}/notifications", put(update_direct))
}

fn invalid() -> ApiError {
    bad_request("invalid notification settings")
}

/// The body must be an object with only the given keys.
fn object<'a>(body: &'a Value, keys: &[&str]) -> Result<&'a Map<String, Value>, ApiError> {
    body.as_object()
        .filter(|object| object.keys().all(|key| keys.contains(&key.as_str())))
        .ok_or_else(invalid)
}

#[derive(Debug, PartialEq, Eq)]
enum Mute {
    Forever,
    Until(DateTime<Utc>),
}

/// `None`: leave the field unchanged. `Some(None)`: reset it.
#[derive(Debug, Default, PartialEq, Eq)]
struct OverrideInput {
    level: Option<Option<Level>>,
    muted_until: Option<Option<Mute>>,
}

fn override_input(
    body: &Value,
    direct: bool,
    now: DateTime<Utc>,
) -> Result<OverrideInput, ApiError> {
    let object = object(body, &["level", "mutedUntil"])?;
    let level = match object.get("level") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => Some(Some(
            value
                .as_str()
                .and_then(Level::parse)
                .filter(|level| !direct || *level == Level::Nothing)
                .ok_or_else(|| {
                    bad_request(if direct {
                        "level must be nothing or null"
                    } else {
                        "level must be all, mentions, nothing or null"
                    })
                })?,
        )),
    };
    let muted_until = match object.get("mutedUntil") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(value)) if value == "forever" => Some(Some(Mute::Forever)),
        Some(value) => {
            let until = value
                .as_str()
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|until| until.with_timezone(&Utc))
                .filter(|until| *until > now && *until <= now + LONGEST_MUTE)
                .ok_or_else(|| {
                    bad_request("mutedUntil must be forever or a time within the next year")
                })?;
            // Whole seconds, as every response formats them.
            let seconds = DateTime::from_timestamp(until.timestamp(), 0).unwrap_or(until);
            Some(Some(Mute::Until(seconds)))
        }
    };
    Ok(OverrideInput { level, muted_until })
}

/// Stored override state, with expired mutes already read as none.
#[derive(sqlx::FromRow)]
struct Stored {
    level: Option<String>,
    forever: bool,
    muted_until: Option<DateTime<Utc>>,
}

impl Stored {
    fn level(&self) -> Value {
        json!(self.level)
    }

    fn muted_until(&self) -> Value {
        if self.forever {
            json!("forever")
        } else {
            json!(
                self.muted_until
                    .map(|until| until.to_rfc3339_opts(SecondsFormat::Secs, true))
            )
        }
    }
}

const STORED: &str = "level, COALESCE(muted_until = 'infinity', false) AS forever,
    CASE WHEN muted_until > now() AND muted_until <> 'infinity' THEN muted_until END AS muted_until";

/// Upserts one override. `scope` is `space_id` or `channel_id`.
async fn save(
    pool: &PgPool,
    user: i64,
    scope: &'static str,
    id: i64,
    input: &OverrideInput,
) -> Result<Stored, ApiError> {
    let level = input.level.flatten().map(Level::name);
    let (forever, until) = match input.muted_until.as_ref().and_then(Option::as_ref) {
        Some(Mute::Forever) => (true, None),
        Some(Mute::Until(until)) => (false, Some(*until)),
        None => (false, None),
    };
    sqlx::query_as(&format!(
        "INSERT INTO public.notification_overrides (user_id, {scope}, level, muted_until)
         VALUES ($1, $2, $4, CASE WHEN $6 THEN 'infinity'::timestamptz ELSE $7 END)
         ON CONFLICT (user_id, {scope}) WHERE {scope} IS NOT NULL DO UPDATE SET
             level = CASE WHEN $3 THEN EXCLUDED.level ELSE notification_overrides.level END,
             muted_until = CASE WHEN $5 THEN EXCLUDED.muted_until ELSE notification_overrides.muted_until END,
             updated_at = now()
         RETURNING {STORED}"
    ))
    .bind(user)
    .bind(id)
    .bind(input.level.is_some())
    .bind(level)
    .bind(input.muted_until.is_some())
    .bind(forever)
    .bind(until)
    .fetch_one(pool)
    .await
    .map_err(database_error)
}

async fn update_space(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(space): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let input = override_input(&body, false, Utc::now())?;
    let pool = pool(&state)?;
    let id: i64 = sqlx::query_scalar(
        "SELECT s.id FROM public.spaces s
         WHERE s.external_id=$1 AND s.deleted_at IS NULL AND NOT s.demo
           AND EXISTS(SELECT 1 FROM public.space_members m WHERE m.space_id=s.id AND m.user_id=$2 AND m.deleted_at IS NULL)",
    )
    .bind(&space)
    .bind(principal.user.id)
    .fetch_optional(pool)
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "space not found"))?;
    let stored = save(pool, principal.user.id, "space_id", id, &input).await?;
    Ok(Json(
        json!({"spaceId": space, "level": stored.level(), "mutedUntil": stored.muted_until()}),
    ))
}

async fn update_channel(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((space, channel)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let input = override_input(&body, false, Utc::now())?;
    let pool = pool(&state)?;
    let not_found = || ApiError::new(StatusCode::NOT_FOUND, "channel not found");
    // Read access, as for history: membership plus a grant for private channels.
    let access = channel_access(pool, &channel, Some(principal.user.id))
        .await
        .map_err(|error| {
            if error.status == StatusCode::NOT_FOUND {
                not_found()
            } else {
                error
            }
        })?;
    let in_space: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM public.spaces WHERE external_id=$1 AND id=$2)",
    )
    .bind(&space)
    .bind(access.space_id)
    .fetch_one(pool)
    .await
    .map_err(database_error)?;
    if !in_space {
        return Err(not_found());
    }
    let stored = save(pool, principal.user.id, "channel_id", access.id, &input).await?;
    Ok(Json(json!({
        "spaceId": space,
        "channelId": channel,
        "level": stored.level(),
        "mutedUntil": stored.muted_until(),
    })))
}

async fn update_direct(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(conversation): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let input = override_input(&body, true, Utc::now())?;
    let pool = pool(&state)?;
    let not_found = || ApiError::new(StatusCode::NOT_FOUND, "conversation not found");
    let access = channel_access(pool, &conversation, Some(principal.user.id))
        .await
        .map_err(|error| {
            if error.status == StatusCode::NOT_FOUND {
                not_found()
            } else {
                error
            }
        })?;
    if access.space_id.is_some() {
        return Err(not_found());
    }
    let stored = save(pool, principal.user.id, "channel_id", access.id, &input).await?;
    Ok(Json(json!({
        "conversationId": conversation,
        "level": stored.level(),
        "mutedUntil": stored.muted_until(),
    })))
}

#[derive(sqlx::FromRow)]
struct Listed {
    space_id: Option<String>,
    channel_id: Option<String>,
    direct: bool,
    #[sqlx(flatten)]
    stored: Stored,
}

/// Account settings plus every override still in effect for a scope the
/// account can still read.
async fn settings(pool: &PgPool, user: i64) -> Result<Value, ApiError> {
    let account: Option<(Option<String>, String)> = sqlx::query_as(
        "SELECT default_level, mobile FROM public.notification_settings WHERE user_id=$1",
    )
    .bind(user)
    .fetch_optional(pool)
    .await
    .map_err(database_error)?;
    let (level, mobile) = account.unwrap_or((None, "whenInactive".into()));
    let rows: Vec<Listed> = sqlx::query_as(
        "SELECT s.external_id AS space_id, c.external_id AS channel_id, c.id IS NOT NULL AND c.space_id IS NULL AS direct,
                o.level, COALESCE(o.muted_until = 'infinity', false) AS forever,
                CASE WHEN o.muted_until > now() AND o.muted_until <> 'infinity' THEN o.muted_until END AS muted_until
         FROM public.notification_overrides o
         LEFT JOIN public.channels c ON c.id=o.channel_id
         LEFT JOIN public.spaces s ON s.id=COALESCE(o.space_id, c.space_id)
         WHERE o.user_id=$1 AND (o.level IS NOT NULL OR o.muted_until > now())
           AND ((s.id IS NOT NULL AND s.deleted_at IS NULL AND NOT s.demo
                 AND EXISTS(SELECT 1 FROM public.space_members m WHERE m.space_id=s.id AND m.user_id=$1 AND m.deleted_at IS NULL)
                 AND (o.space_id IS NOT NULL
                      OR (c.deleted_at IS NULL AND (NOT c.private OR s.owner_id=$1
                          OR EXISTS(SELECT 1 FROM public.channel_members cm WHERE cm.channel_id=c.id AND cm.user_id=$1 AND cm.deleted_at IS NULL)))))
                OR (c.space_id IS NULL AND c.deleted_at IS NULL AND EXISTS(
                    SELECT 1 FROM public.direct_conversations d
                    JOIN public.users lo ON lo.id=d.low_user_id JOIN public.users hi ON hi.id=d.high_user_id
                    WHERE d.channel_id=c.id AND $1 IN (d.low_user_id,d.high_user_id)
                      AND lo.deleted_at IS NULL AND hi.deleted_at IS NULL)))
         ORDER BY o.id",
    )
    .bind(user)
    .fetch_all(pool)
    .await
    .map_err(database_error)?;
    let overrides: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            let mut entry = if row.direct {
                json!({"conversationId": row.channel_id})
            } else {
                let mut entry = json!({"spaceId": row.space_id});
                if let Some(channel) = row.channel_id {
                    entry["channelId"] = json!(channel);
                }
                entry
            };
            entry["level"] = row.stored.level();
            entry["mutedUntil"] = row.stored.muted_until();
            entry
        })
        .collect();
    Ok(json!({
        "level": level.as_deref().and_then(Level::parse).unwrap_or(Level::All).name(),
        "mobile": mobile,
        "overrides": overrides,
    }))
}

async fn read_settings(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(settings(pool(&state)?, principal.user.id).await?))
}

async fn update_settings(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let object = object(&body, &["level", "mobile"])?;
    let level = object
        .get("level")
        .map(|level| {
            level
                .as_str()
                .and_then(Level::parse)
                .ok_or_else(|| bad_request("level must be all, mentions or nothing"))
        })
        .transpose()?;
    let mobile = object
        .get("mobile")
        .map(|mobile| {
            mobile
                .as_str()
                .filter(|mobile| ["always", "whenInactive"].contains(mobile))
                .ok_or_else(|| bad_request("mobile must be always or whenInactive"))
        })
        .transpose()?;
    let pool = pool(&state)?;
    sqlx::query(
        "INSERT INTO public.notification_settings (user_id, default_level, mobile)
         VALUES ($1, $3, COALESCE($5, 'whenInactive'))
         ON CONFLICT (user_id) DO UPDATE SET
             default_level = CASE WHEN $2 THEN EXCLUDED.default_level ELSE notification_settings.default_level END,
             mobile = CASE WHEN $4 THEN EXCLUDED.mobile ELSE notification_settings.mobile END,
             updated_at = now()",
    )
    .bind(principal.user.id)
    .bind(level.is_some())
    .bind(level.map(Level::name))
    .bind(mobile.is_some())
    .bind(mobile)
    .execute(pool)
    .await
    .map_err(database_error)?;
    Ok(Json(settings(pool, principal.user.id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_input_distinguishes_missing_null_and_values() {
        let now = Utc::now();
        assert_eq!(
            override_input(&json!({}), false, now).unwrap(),
            OverrideInput::default()
        );
        assert_eq!(
            override_input(&json!({"level": null, "mutedUntil": "forever"}), false, now).unwrap(),
            OverrideInput {
                level: Some(None),
                muted_until: Some(Some(Mute::Forever)),
            }
        );
        let soon = now + TimeDelta::minutes(15);
        let parsed = override_input(
            &json!({"level": "mentions", "mutedUntil": soon.to_rfc3339()}),
            false,
            now,
        )
        .unwrap();
        assert_eq!(parsed.level, Some(Some(Level::Mentions)));
        assert_eq!(
            parsed.muted_until,
            Some(Some(Mute::Until(
                DateTime::from_timestamp(soon.timestamp(), 0).unwrap()
            )))
        );
        // Other offsets are accepted and normalized to UTC.
        assert!(
            override_input(
                &json!({"mutedUntil": (now + TimeDelta::hours(1)).with_timezone(&chrono::FixedOffset::east_opt(3600).unwrap()).to_rfc3339()}),
                false,
                now
            )
            .is_ok()
        );
    }

    #[test]
    fn override_input_rejects_bad_levels_times_and_keys() {
        let now = Utc::now();
        let message =
            |body: Value, direct: bool| override_input(&body, direct, now).unwrap_err().message;
        assert_eq!(
            message(json!({"level": "loud"}), false),
            "level must be all, mentions, nothing or null"
        );
        assert_eq!(
            message(json!({"level": "all"}), true),
            "level must be nothing or null"
        );
        assert!(override_input(&json!({"level": "nothing"}), true, now).is_ok());
        assert_eq!(
            message(json!({"muted": true}), false),
            "invalid notification settings"
        );
        assert_eq!(message(json!([]), false), "invalid notification settings");
        for until in [
            json!("tomorrow"),
            json!(42),
            json!((now - TimeDelta::seconds(1)).to_rfc3339()),
            json!((now + TimeDelta::days(366)).to_rfc3339()),
        ] {
            assert_eq!(
                message(json!({"mutedUntil": until}), false),
                "mutedUntil must be forever or a time within the next year"
            );
        }
    }
}
