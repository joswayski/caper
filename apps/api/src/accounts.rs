//! Provider-neutral account persistence. Database errors must be logged without parameters or PII.
use serde::Serialize;
use sqlx::{FromRow, PgPool};

#[derive(Clone, Debug, FromRow)]
pub struct User {
    pub id: i64,
    pub email: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicAccount<'a> {
    // JSON numbers cannot safely represent every bigint account ID.
    pub id: String,
    pub username: Option<&'a str>,
    pub display_name: Option<&'a str>,
}

impl User {
    pub fn public(&self) -> PublicAccount<'_> {
        PublicAccount {
            id: self.id.to_string(),
            username: self.username.as_deref(),
            display_name: self.display_name.as_deref(),
        }
    }

    pub fn onboarded(&self) -> bool {
        self.username.is_some() && self.display_name.is_some()
    }
}

pub async fn set_profile(
    pool: &PgPool,
    user_id: i64,
    username: &str,
    display_name: &str,
) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as(
        "UPDATE public.users SET username = $2, display_name = $3, updated_at = now()
         WHERE id = $1 AND deleted_at IS NULL RETURNING *",
    )
    .bind(user_id)
    .bind(username.trim().to_ascii_lowercase())
    .bind(display_name.trim())
    .fetch_optional(pool)
    .await
}
