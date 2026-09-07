//! Postgres persistence only: no HTTP authentication, cookies, or email delivery.
//! Callers must never log SQL errors with parameters, emails, tokens, or codes.
use sqlx::{FromRow, PgConnection, PgPool};
use subtle::ConstantTimeEq;

/// Internal model, deliberately not Serialize: never expose bigint IDs or email
/// by serializing a database row into a public profile response.
#[derive(FromRow)]
pub struct User {
    pub id: i64,
    pub public_id: String,
    pub email: String,
    pub username: Option<String>,
    pub display_name: Option<String>,
}

/// Generate outside the transaction so the sender can bind its HMAC to this ID.
pub fn new_public_id() -> String {
    nanoid::nanoid!()
}

/// Product policy: email addresses are case-insensitive. Do not strip dots or
/// plus suffixes. The HTTP boundary must separately validate email syntax.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

/// Store a 10-minute challenge, with a database-enforced 60-second resend floor.
/// Returns false during cooldown (including after successful use). The caller
/// sends email only after true, and must add IP/global abuse and daily send caps.
/// `code_hash` must be HMAC-SHA-256(key, unambiguous email + public_id + code),
/// NOT an unkeyed hash. The server-only HMAC key must survive process restarts.
pub async fn issue_challenge(
    pool: &PgPool,
    email: &str,
    public_id: &str,
    code_hash: &[u8; 32],
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO login_challenges (public_id, email, code_hash, expires_at)
         VALUES ($1, $2, $3, now() + interval '10 minutes')
         ON CONFLICT (email) DO UPDATE SET public_id = EXCLUDED.public_id,
             code_hash = EXCLUDED.code_hash, attempts = 0, consumed_at = NULL,
             created_at = now(), expires_at = EXCLUDED.expires_at
         WHERE login_challenges.created_at <= now() - interval '60 seconds'",
    )
    .bind(public_id)
    .bind(normalize_email(email))
    .bind(code_hash.as_slice())
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Verify once and atomically create/find the account and issue a 30-day session.
/// Failed guesses persist; after five attempts the challenge cannot be used.
/// `candidate_hash` follows issue_challenge's HMAC contract; `token_hash` is
/// SHA-256 of a fresh CSPRNG token with at least 256 bits of entropy. Token
/// generation, hashing, and delivery to the client belong to the auth layer.
pub async fn complete_login(
    pool: &PgPool,
    public_id: &str,
    candidate_hash: &[u8; 32],
    token_hash: &[u8; 32],
) -> Result<Option<User>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    // UPDATE locks the row and rechecks eligibility after concurrent updates.
    // Wrong guesses still commit; successful concurrent claims cannot both win.
    let challenge = sqlx::query_as::<_, (String, Vec<u8>)>(
        "UPDATE login_challenges SET attempts = attempts + 1
         WHERE public_id = $1 AND consumed_at IS NULL AND attempts < 5
             AND expires_at > clock_timestamp()
         RETURNING email, code_hash",
    )
    .bind(public_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((email, expected_hash)) = challenge else {
        tx.commit().await?;
        return Ok(None);
    };
    if !bool::from(expected_hash.as_slice().ct_eq(candidate_hash.as_slice())) {
        tx.commit().await?;
        return Ok(None);
    }
    sqlx::query("UPDATE login_challenges SET consumed_at = clock_timestamp() WHERE public_id = $1")
        .bind(public_id)
        .execute(&mut *tx)
        .await?;
    let user = find_or_create_user(&mut tx, &email).await?;
    sqlx::query(
        "INSERT INTO sessions (user_id, token_hash, expires_at)
         VALUES ($1, $2, now() + interval '30 days')",
    )
    .bind(user.id)
    .bind(token_hash.as_slice())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(user))
}

async fn find_or_create_user(conn: &mut PgConnection, email: &str) -> Result<User, sqlx::Error> {
    // DO NOTHING handles either unique constraint without aborting the enclosing
    // transaction, so a rare public-ID collision can be retried safely.
    for _ in 0..3 {
        if let Some(user) = sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = $1")
            .bind(email)
            .fetch_optional(&mut *conn)
            .await?
        {
            return Ok(user);
        }
        if let Some(user) = sqlx::query_as::<_, User>(
            "INSERT INTO users (public_id, email) VALUES ($1, $2)
             ON CONFLICT DO NOTHING RETURNING *",
        )
        .bind(new_public_id())
        .bind(email)
        .fetch_optional(&mut *conn)
        .await?
        {
            return Ok(user);
        }
    }
    Err(sqlx::Error::Protocol(
        "could not allocate account ID".into(),
    ))
}

/// The caller supplies an authenticated internal user ID, never a client ID.
/// Username conflicts and invalid profiles are database constraint errors.
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

/// Same lookup for browser cookies and native bearer tokens, after hashing.
/// An account with no username still requires onboarding before app access.
pub async fn session_user(
    pool: &PgPool,
    token_hash: &[u8; 32],
) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as(
        "SELECT users.* FROM sessions JOIN users ON users.id = sessions.user_id
         WHERE token_hash = $1 AND expires_at > clock_timestamp()",
    )
    .bind(token_hash.as_slice())
    .fetch_optional(pool)
    .await
}

pub async fn revoke_session(pool: &PgPool, token_hash: &[u8; 32]) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(token_hash.as_slice())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn revoke_all_sessions(pool: &PgPool, user_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Explicit maintenance hook; expiry is enforced on reads even before cleanup.
pub async fn delete_expired(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM sessions WHERE expires_at <= now()")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM login_challenges WHERE expires_at <= now()")
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}
