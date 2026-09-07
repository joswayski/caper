//! Account persistence. Database errors must be logged without parameters or PII.
use serde::Serialize;
use sqlx::{FromRow, PgPool};

#[derive(Clone, Debug, FromRow)]
pub struct User {
    pub id: i64,
    pub public_id: String,
    pub workos_user_id: String,
    pub email: String,
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
            id: &self.public_id,
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
) -> Result<User, sqlx::Error> {
    let email = normalize_email(verified_email);
    sqlx::query_as(
        "INSERT INTO users (public_id, workos_user_id, email) VALUES ($1, $2, $3)
         ON CONFLICT (workos_user_id) DO UPDATE
         SET email = EXCLUDED.email, updated_at = now() RETURNING *",
    )
    .bind(nanoid::nanoid!())
    .bind(subject)
    .bind(&email)
    .fetch_one(pool)
    .await
}

pub async fn set_profile(
    pool: &PgPool,
    user_id: i64,
    username: &str,
    display_name: &str,
) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as(
        "UPDATE users SET username = $2, display_name = $3, updated_at = now()
         WHERE id = $1 RETURNING *",
    )
    .bind(user_id)
    .bind(username.trim().to_ascii_lowercase())
    .bind(display_name.trim())
    .fetch_optional(pool)
    .await
}
