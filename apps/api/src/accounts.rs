//! Account persistence. Database errors must be logged without parameters or PII.
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool, Postgres, Transaction};

#[derive(Clone, Debug, FromRow)]
pub struct User {
    pub id: i64,
    pub external_id: String,
    pub workos_user_id: String,
    pub email: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicAccount<'a> {
    pub id: &'a str,
    pub username: Option<&'a str>,
    pub display_name: Option<&'a str>,
}

impl User {
    pub fn public(&self) -> PublicAccount<'_> {
        PublicAccount {
            id: &self.external_id,
            username: self.username.as_deref(),
            display_name: self.display_name.as_deref(),
        }
    }

    pub fn onboarded(&self) -> bool {
        self.username.is_some() && self.display_name.is_some()
    }
}

pub fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

/// Upsert only after WorkOS returned this subject and a verified email. The
/// immutable subject owns identity; email changes never merge two accounts.
pub async fn sync_workos_user(
    pool: &PgPool,
    subject: &str,
    verified_email: &str,
    provider_updated_at: DateTime<Utc>,
) -> Result<User, sqlx::Error> {
    let email = normalize_email(verified_email);
    sqlx::query_as(
        "INSERT INTO users (external_id, workos_user_id, email, workos_updated_at)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (workos_user_id) DO UPDATE
         SET email = CASE WHEN users.workos_updated_at IS NULL OR users.workos_updated_at < $4
                          THEN EXCLUDED.email ELSE users.email END,
             email_verified_at = CASE
                 WHEN users.workos_updated_at IS NULL OR users.workos_updated_at < $4 THEN now()
                 ELSE users.email_verified_at END,
             workos_updated_at = CASE
                 WHEN users.workos_updated_at IS NULL OR users.workos_updated_at < $4 THEN $4
                 ELSE users.workos_updated_at END,
             updated_at = CASE
                 WHEN users.workos_updated_at IS NULL OR users.workos_updated_at < $4 THEN now()
                 ELSE users.updated_at END
         WHERE users.deleted_at IS NULL
         RETURNING *",
    )
    .bind(nanoid::nanoid!())
    .bind(subject)
    .bind(&email)
    .bind(provider_updated_at)
    .fetch_one(pool)
    .await
    .and_then(|user: User| {
        if user.email.is_some() {
            Ok(user)
        } else {
            Err(sqlx::Error::RowNotFound)
        }
    })
}

pub async fn record_workos_event(
    tx: &mut Transaction<'_, Postgres>,
    event_id: &str,
    event_type: &str,
) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query(
        "INSERT INTO workos_events (event_id, event_type) VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(event_id)
    .bind(event_type)
    .execute(&mut **tx)
    .await?
    .rows_affected()
        == 1)
}

pub async fn apply_workos_update(
    tx: &mut Transaction<'_, Postgres>,
    subject: &str,
    email: Option<&str>,
    updated_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let email = email.map(normalize_email);
    sqlx::query(
        "UPDATE users SET email = $2, email_verified_at = CASE WHEN $2 IS NULL THEN NULL ELSE now() END,
             workos_updated_at = $3, updated_at = now()
         WHERE workos_user_id = $1 AND deleted_at IS NULL
           AND (workos_updated_at IS NULL OR workos_updated_at < $3)",
    )
    .bind(subject)
    .bind(email)
    .bind(updated_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn apply_workos_deletion(
    tx: &mut Transaction<'_, Postgres>,
    subject: &str,
    deleted_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO users (external_id, workos_user_id, email, email_verified_at,
             workos_updated_at, deleted_at)
         VALUES ($1, $2, NULL, NULL, $3, $3)
         ON CONFLICT (workos_user_id) DO UPDATE SET email = NULL,
             email_verified_at = NULL,
             deleted_at = COALESCE(users.deleted_at, $3),
             workos_updated_at = GREATEST(users.workos_updated_at, $3), updated_at = now()",
    )
    .bind(nanoid::nanoid!())
    .bind(subject)
    .bind(deleted_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn set_profile(
    pool: &PgPool,
    user_id: i64,
    username: &str,
    display_name: &str,
) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as(
        "UPDATE users SET username = $2, display_name = $3, updated_at = now()
         WHERE id = $1 AND deleted_at IS NULL RETURNING *",
    )
    .bind(user_id)
    .bind(username.trim().to_ascii_lowercase())
    .bind(display_name.trim())
    .fetch_optional(pool)
    .await
}
