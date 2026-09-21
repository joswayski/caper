use crate::{ApiError, RuntimeEnvironment, accounts, email};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use hmac::{Hmac, Mac};
use rand::Rng;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use std::{net::IpAddr, sync::Arc};
use subtle::ConstantTimeEq;
use uuid::Uuid;

const CODE_LIFETIME: Duration = Duration::minutes(10);
const SESSION_LIFETIME: Duration = Duration::days(30);
// Excludes visually ambiguous characters: 0/O, 1/I/L, and U/V.
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTWXYZ23456789";
const EXTERNAL_ID_ALPHABET: &[u8] =
    b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const EXTERNAL_ID_LENGTH: usize = 12;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub(crate) struct AuthVerifier {
    enabled: Option<EnabledAuth>,
    #[cfg(test)]
    bypass: bool,
}

#[derive(Clone)]
struct EnabledAuth {
    secret: Arc<[u8]>,
    sender: Arc<dyn email::EmailSender>,
    limits: AuthLimits,
}

#[derive(Clone, Copy)]
struct AuthLimits {
    code_attempts: i16,
    email_15m: i64,
    email_daily: i64,
    ip_hourly: i64,
    global_hourly: i64,
}

impl AuthLimits {
    fn from_env(environment: &RuntimeEnvironment) -> Result<Self, String> {
        Ok(Self {
            code_attempts: parse_limit(environment, "AUTH_CODE_ATTEMPTS", 3, 10)? as i16,
            email_15m: parse_limit(environment, "AUTH_EMAIL_15M_LIMIT", 3, 100)?,
            email_daily: parse_limit(environment, "AUTH_EMAIL_DAILY_LIMIT", 5, 1_000)?,
            ip_hourly: parse_limit(environment, "AUTH_IP_HOURLY_LIMIT", 10, 10_000)?,
            global_hourly: parse_limit(environment, "AUTH_GLOBAL_HOURLY_LIMIT", 500, 1_000_000)?,
        })
    }
}

impl Default for AuthLimits {
    fn default() -> Self {
        Self {
            code_attempts: 3,
            email_15m: 3,
            email_daily: 5,
            ip_hourly: 10,
            global_hourly: 500,
        }
    }
}

#[derive(Clone)]
pub(crate) struct Principal {
    pub user: accounts::User,
    pub token_hash: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct VerifiedSession {
    pub user: accounts::User,
    pub token: String,
    pub user_created: bool,
}

#[derive(FromRow)]
struct Challenge {
    email: String,
    code_hash: Vec<u8>,
    attempts_remaining: i16,
    expires_at: chrono::DateTime<Utc>,
    consumed_at: Option<chrono::DateTime<Utc>>,
}

#[derive(FromRow)]
struct VerifiedUser {
    id: i64,
    external_id: String,
    email: Option<String>,
    username: Option<String>,
    display_name: Option<String>,
    created: bool,
}

impl AuthVerifier {
    pub fn new() -> Self {
        Self {
            enabled: None,
            #[cfg(test)]
            bypass: false,
        }
    }

    pub async fn from_env(environment: &RuntimeEnvironment) -> Result<Self, String> {
        let Some(secret) = environment
            .get("AUTH_SECRET")
            .filter(|value| !value.trim().is_empty())
        else {
            tracing::info!("AUTH_SECRET unset; account login remains disabled");
            return Ok(Self::new());
        };
        if secret.len() < 32 {
            return Err("AUTH_SECRET must contain at least 32 bytes".into());
        }
        let limits = AuthLimits::from_env(environment)?;
        let sender = email::SesEmailSender::from_env(environment).await?;
        Ok(Self {
            enabled: Some(EnabledAuth {
                secret: Arc::from(secret.into_bytes()),
                sender: Arc::new(sender),
                limits,
            }),
            #[cfg(test)]
            bypass: false,
        })
    }

    #[cfg(test)]
    pub fn test_bypass() -> Self {
        Self {
            enabled: None,
            bypass: true,
        }
    }

    #[cfg(test)]
    pub fn is_test_bypass(&self) -> bool {
        self.bypass
    }

    fn enabled(&self) -> Result<&EnabledAuth, ApiError> {
        self.enabled.as_ref().ok_or_else(unavailable)
    }

    pub async fn request_code(
        &self,
        pool: Option<&PgPool>,
        email: &str,
        ip: IpAddr,
    ) -> Result<Uuid, ApiError> {
        let auth = self.enabled()?;
        let pool = pool.ok_or_else(unavailable)?;
        let email = normalize_email(email)?;
        let ip_hash = keyed_hash(&auth.secret, b"ip", ip.to_string().as_bytes());
        let email_lock_hash = keyed_hash(&auth.secret, b"email-lock", email.as_bytes());
        let ip_lock_hash = keyed_hash(&auth.secret, b"ip-lock", &ip_hash);
        let global_lock_hash = keyed_hash(&auth.secret, b"global-send-lock", b"caper");
        let mut lock_ids = [
            i64::from_be_bytes(email_lock_hash[..8].try_into().expect("hash length")),
            i64::from_be_bytes(ip_lock_hash[..8].try_into().expect("hash length")),
            i64::from_be_bytes(global_lock_hash[..8].try_into().expect("hash length")),
        ];
        lock_ids.sort_unstable();
        let mut transaction = pool.begin().await.map_err(database_unavailable)?;
        for lock_id in lock_ids {
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(lock_id)
                .execute(&mut *transaction)
                .await
                .map_err(database_unavailable)?;
        }
        sqlx::query(
            "DELETE FROM public.auth_email_challenges
             WHERE created_at < now() - interval '7 days'",
        )
        .execute(&mut *transaction)
        .await
        .map_err(database_unavailable)?;
        sqlx::query(
            "DELETE FROM public.account_sessions
             WHERE expires_at < now() - interval '7 days'",
        )
        .execute(&mut *transaction)
        .await
        .map_err(database_unavailable)?;

        let email_recent: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.auth_email_challenges
             WHERE email = $1 AND created_at > now() - interval '15 minutes'",
        )
        .bind(&email)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_unavailable)?;
        let email_daily: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.auth_email_challenges
             WHERE email = $1 AND created_at > now() - interval '24 hours'",
        )
        .bind(&email)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_unavailable)?;
        let ip_hourly: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.auth_email_challenges
             WHERE request_ip_hash = $1 AND created_at > now() - interval '1 hour'",
        )
        .bind(&ip_hash)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_unavailable)?;
        let global_hourly: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM public.auth_email_challenges
             WHERE created_at > now() - interval '1 hour'",
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_unavailable)?;

        let id = Uuid::new_v4();
        if email_recent >= auth.limits.email_15m
            || email_daily >= auth.limits.email_daily
            || ip_hourly >= auth.limits.ip_hourly
            || global_hourly >= auth.limits.global_hourly
        {
            transaction.commit().await.map_err(database_unavailable)?;
            return Ok(id);
        }

        let code = random_code();
        let code_hash = code_hash(&auth.secret, id, &email, &code);
        sqlx::query(
            "UPDATE public.auth_email_challenges SET consumed_at = now()
             WHERE email = $1 AND consumed_at IS NULL",
        )
        .bind(&email)
        .execute(&mut *transaction)
        .await
        .map_err(database_unavailable)?;
        sqlx::query(
            "INSERT INTO public.auth_email_challenges
             (id, email, code_hash, request_ip_hash, attempts_remaining, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(&email)
        .bind(code_hash)
        .bind(ip_hash)
        .bind(auth.limits.code_attempts)
        .bind(Utc::now() + CODE_LIFETIME)
        .execute(&mut *transaction)
        .await
        .map_err(database_unavailable)?;
        transaction.commit().await.map_err(database_unavailable)?;

        if auth.sender.send_login_code(&email, &code).await.is_err() {
            let _ = sqlx::query(
                "UPDATE public.auth_email_challenges SET consumed_at = now() WHERE id = $1",
            )
            .bind(id)
            .execute(pool)
            .await;
            return Err(unavailable());
        }
        Ok(id)
    }

    pub async fn verify_code(
        &self,
        pool: Option<&PgPool>,
        challenge_id: Uuid,
        code: &str,
    ) -> Result<VerifiedSession, ApiError> {
        let auth = self.enabled()?;
        let pool = pool.ok_or_else(unavailable)?;
        if code.len() != 6 || !code.bytes().all(|byte| CODE_ALPHABET.contains(&byte)) {
            return Err(invalid_code(None));
        }
        let mut transaction = pool.begin().await.map_err(database_unavailable)?;
        let challenge: Option<Challenge> = sqlx::query_as(
            "SELECT email, code_hash, attempts_remaining, expires_at, consumed_at
             FROM public.auth_email_challenges WHERE id = $1 FOR UPDATE",
        )
        .bind(challenge_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_unavailable)?;
        let Some(challenge) = challenge else {
            return Err(invalid_code(None));
        };
        if challenge.consumed_at.is_some()
            || challenge.expires_at <= Utc::now()
            || challenge.attempts_remaining <= 0
        {
            transaction.commit().await.map_err(database_unavailable)?;
            return Err(invalid_code(Some(0)));
        }
        let expected = code_hash(&auth.secret, challenge_id, &challenge.email, code);
        if !bool::from(challenge.code_hash.ct_eq(&expected)) {
            let attempts_remaining = challenge.attempts_remaining - 1;
            sqlx::query(
                "UPDATE public.auth_email_challenges
                 SET attempts_remaining = $2,
                     consumed_at = CASE WHEN $2 = 0 THEN now() ELSE consumed_at END
                 WHERE id = $1",
            )
            .bind(challenge_id)
            .bind(attempts_remaining)
            .execute(&mut *transaction)
            .await
            .map_err(database_unavailable)?;
            transaction.commit().await.map_err(database_unavailable)?;
            return Err(invalid_code(Some(attempts_remaining as u8)));
        }

        sqlx::query("UPDATE public.auth_email_challenges SET consumed_at = now() WHERE id = $1")
            .bind(challenge_id)
            .execute(&mut *transaction)
            .await
            .map_err(database_unavailable)?;
        let external_id = random_external_id();
        let user: Option<VerifiedUser> = sqlx::query_as(
            "WITH inserted AS (
                INSERT INTO public.users (external_id, email, email_verified_at)
                VALUES ($1, $2, now())
                ON CONFLICT (email) DO NOTHING
                RETURNING id, external_id, email, username, display_name, true AS created
             ), existing AS (
                UPDATE public.users SET
                    email_verified_at = COALESCE(email_verified_at, now()), updated_at = now()
                WHERE email = $2 AND deleted_at IS NULL
                    AND NOT EXISTS (SELECT 1 FROM inserted)
                RETURNING id, external_id, email, username, display_name, false AS created
             )
             SELECT * FROM inserted UNION ALL SELECT * FROM existing",
        )
        .bind(external_id)
        .bind(&challenge.email)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_unavailable)?;
        let Some(user) = user else {
            transaction.commit().await.map_err(database_unavailable)?;
            return Err(invalid_code(None));
        };
        let token_bytes: [u8; 32] = rand::random();
        let token = URL_SAFE_NO_PAD.encode(token_bytes);
        let token_hash = Sha256::digest(token.as_bytes()).to_vec();
        sqlx::query(
            "INSERT INTO public.account_sessions (token_hash, user_id, expires_at)
             VALUES ($1, $2, $3)",
        )
        .bind(token_hash)
        .bind(user.id)
        .bind(Utc::now() + SESSION_LIFETIME)
        .execute(&mut *transaction)
        .await
        .map_err(database_unavailable)?;
        transaction.commit().await.map_err(database_unavailable)?;
        Ok(VerifiedSession {
            user: accounts::User {
                id: user.id,
                external_id: user.external_id,
                email: user.email,
                username: user.username,
                display_name: user.display_name,
            },
            token,
            user_created: user.created,
        })
    }

    pub async fn authenticate(
        &self,
        token: &str,
        pool: Option<&PgPool>,
    ) -> Result<Principal, ApiError> {
        #[cfg(test)]
        if self.bypass {
            return Ok(Principal {
                user: accounts::User {
                    id: 1,
                    external_id: "V1StGXR8_Z5jdHi6B-myT".into(),
                    email: None,
                    username: Some("test".into()),
                    display_name: Some("Test User".into()),
                },
                token_hash: vec![],
            });
        }
        self.enabled()?;
        let pool = pool.ok_or_else(unavailable)?;
        let token_hash = Sha256::digest(token.as_bytes()).to_vec();
        let user = sqlx::query_as(
            "SELECT u.id, u.external_id, u.email, u.username, u.display_name
             FROM public.account_sessions s
             JOIN public.users u ON u.id = s.user_id
             WHERE s.token_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > now()
               AND u.deleted_at IS NULL",
        )
        .bind(&token_hash)
        .fetch_optional(pool)
        .await
        .map_err(database_unavailable)?
        .ok_or_else(|| ApiError::new(axum::http::StatusCode::UNAUTHORIZED, "unauthorized"))?;
        Ok(Principal { user, token_hash })
    }

    pub async fn logout(&self, pool: Option<&PgPool>, token_hash: &[u8]) -> Result<(), ApiError> {
        let pool = pool.ok_or_else(unavailable)?;
        sqlx::query(
            "UPDATE public.account_sessions SET revoked_at = now()
             WHERE token_hash = $1 AND revoked_at IS NULL",
        )
        .bind(token_hash)
        .execute(pool)
        .await
        .map_err(database_unavailable)?;
        Ok(())
    }
}

fn normalize_email(value: &str) -> Result<String, ApiError> {
    let email = value.trim().to_ascii_lowercase();
    if email.len() > 254 || !email_address::EmailAddress::is_valid(&email) {
        return Err(ApiError::new(
            axum::http::StatusCode::BAD_REQUEST,
            "invalid email",
        ));
    }
    Ok(email)
}

fn keyed_hash(secret: &[u8], domain: &[u8], value: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(domain);
    mac.update(&[0]);
    mac.update(value);
    mac.finalize().into_bytes().to_vec()
}

fn code_hash(secret: &[u8], id: Uuid, email: &str, code: &str) -> Vec<u8> {
    let mut value = Vec::with_capacity(16 + email.len() + code.len() + 2);
    value.extend_from_slice(id.as_bytes());
    value.push(0);
    value.extend_from_slice(email.as_bytes());
    value.push(0);
    value.extend_from_slice(code.as_bytes());
    keyed_hash(secret, b"login-code", &value)
}

fn random_code() -> String {
    let mut rng = rand::rng();
    (0..6)
        .map(|_| CODE_ALPHABET[rng.random_range(0..CODE_ALPHABET.len())] as char)
        .collect()
}

fn random_external_id() -> String {
    let mut rng = rand::rng();
    (0..EXTERNAL_ID_LENGTH)
        .map(|_| EXTERNAL_ID_ALPHABET[rng.random_range(0..EXTERNAL_ID_ALPHABET.len())] as char)
        .collect()
}

fn parse_limit(
    environment: &RuntimeEnvironment,
    name: &'static str,
    default: i64,
    maximum: i64,
) -> Result<i64, String> {
    parse_limit_value(name, environment.get(name), default, maximum)
}

fn parse_limit_value(
    name: &'static str,
    value: Option<String>,
    default: i64,
    maximum: i64,
) -> Result<i64, String> {
    let Some(value) = value else {
        return Ok(default);
    };
    let parsed = value
        .parse::<i64>()
        .ok()
        .filter(|value| (1..=maximum).contains(value))
        .ok_or_else(|| format!("{name} must be an integer between 1 and {maximum}"))?;
    Ok(parsed)
}

fn invalid_code(attempts_remaining: Option<u8>) -> ApiError {
    let error = ApiError::new(
        axum::http::StatusCode::UNAUTHORIZED,
        "invalid or expired code",
    );
    match attempts_remaining {
        Some(attempts_remaining) => error.with_attempts_remaining(attempts_remaining),
        None => error,
    }
}

fn unavailable() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "account service unavailable",
    )
}

fn database_unavailable(_: sqlx::Error) -> ApiError {
    tracing::error!("account database operation failed");
    unavailable()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RecordingSender {
        deliveries: std::sync::Mutex<Vec<(String, String)>>,
    }

    #[async_trait::async_trait]
    impl email::EmailSender for RecordingSender {
        async fn send_login_code(&self, recipient: &str, code: &str) -> Result<(), ()> {
            self.deliveries
                .lock()
                .unwrap()
                .push((recipient.into(), code.into()));
            Ok(())
        }
    }

    #[test]
    fn email_normalization_is_strict_and_code_hash_is_challenge_bound() {
        assert_eq!(
            normalize_email(" Person@Example.COM ").unwrap(),
            "person@example.com"
        );
        assert!(normalize_email("not-an-email").is_err());
        let id = Uuid::new_v4();
        let secret = b"a sufficiently long test-only secret";
        let hash = code_hash(secret, id, "person@example.com", "123456");
        assert_ne!(
            hash,
            code_hash(secret, Uuid::new_v4(), "person@example.com", "123456")
        );
        assert_ne!(hash, code_hash(secret, id, "other@example.com", "123456"));
        assert_ne!(hash, code_hash(secret, id, "person@example.com", "654321"));
    }

    #[test]
    fn generated_codes_use_only_unambiguous_characters() {
        for _ in 0..100 {
            let code = random_code();
            assert_eq!(code.len(), 6);
            assert!(code.bytes().all(|byte| CODE_ALPHABET.contains(&byte)));
        }
    }

    #[test]
    fn generated_external_ids_are_twelve_alphanumeric_characters() {
        for _ in 0..100 {
            let external_id = random_external_id();
            assert_eq!(external_id.len(), EXTERNAL_ID_LENGTH);
            assert!(external_id.bytes().all(|byte| byte.is_ascii_alphanumeric()));
        }
    }

    #[test]
    fn auth_limits_use_defaults_and_reject_unsafe_values() {
        assert_eq!(parse_limit_value("LIMIT", None, 3, 10).unwrap(), 3);
        assert_eq!(
            parse_limit_value("LIMIT", Some("7".into()), 3, 10).unwrap(),
            7
        );
        assert!(parse_limit_value("LIMIT", Some("0".into()), 3, 10).is_err());
        assert!(parse_limit_value("LIMIT", Some("11".into()), 3, 10).is_err());
        assert!(parse_limit_value("LIMIT", Some("many".into()), 3, 10).is_err());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires disposable Postgres DATABASE_URL"]
    async fn email_code_creates_revocable_session_and_throttles_resends(pool: PgPool) {
        let sender = Arc::new(RecordingSender {
            deliveries: std::sync::Mutex::new(vec![]),
        });
        let verifier = AuthVerifier {
            enabled: Some(EnabledAuth {
                secret: Arc::from(b"a sufficiently long test-only auth secret".as_slice()),
                sender: sender.clone(),
                limits: AuthLimits::default(),
            }),
            bypass: false,
        };
        let ip = "192.0.2.10".parse().unwrap();

        let challenge = verifier
            .request_code(Some(&pool), " Person@Example.COM ", ip)
            .await
            .unwrap();
        let deliveries = sender.deliveries.lock().unwrap().clone();
        assert_eq!(deliveries[0].0, "person@example.com");
        let code = &deliveries[0].1;
        let wrong_code = if code == "AAAAAA" { "BBBBBB" } else { "AAAAAA" };
        let wrong = verifier
            .verify_code(Some(&pool), challenge, wrong_code)
            .await
            .unwrap_err();
        assert_eq!(wrong.status, axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(wrong.attempts_remaining, Some(2));
        let session = verifier
            .verify_code(Some(&pool), challenge, code)
            .await
            .unwrap();
        assert!(session.user_created);
        assert_eq!(session.user.email.as_deref(), Some("person@example.com"));
        assert!(session.user.username.is_none());

        let principal = verifier
            .authenticate(&session.token, Some(&pool))
            .await
            .unwrap();
        assert_eq!(principal.user.id, session.user.id);
        verifier
            .logout(Some(&pool), &principal.token_hash)
            .await
            .unwrap();
        assert!(
            verifier
                .authenticate(&session.token, Some(&pool))
                .await
                .is_err()
        );

        for _ in 0..3 {
            verifier
                .request_code(Some(&pool), "person@example.com", ip)
                .await
                .unwrap();
        }
        assert_eq!(
            sender.deliveries.lock().unwrap().len(),
            3,
            "the fourth request in 15 minutes must be silently throttled"
        );

        let exhausted = verifier
            .request_code(Some(&pool), "attempts@example.com", ip)
            .await
            .unwrap();
        let exhausted_code = sender.deliveries.lock().unwrap().last().unwrap().1.clone();
        let incorrect = if exhausted_code == "AAAAAA" {
            "BBBBBB"
        } else {
            "AAAAAA"
        };
        for expected in [2, 1, 0] {
            let error = verifier
                .verify_code(Some(&pool), exhausted, incorrect)
                .await
                .unwrap_err();
            assert_eq!(error.attempts_remaining, Some(expected));
        }
        let consumed = verifier
            .verify_code(Some(&pool), exhausted, &exhausted_code)
            .await
            .unwrap_err();
        assert_eq!(consumed.attempts_remaining, Some(0));

        let old = verifier
            .request_code(Some(&pool), "replacement@example.com", ip)
            .await
            .unwrap();
        let old_code = sender.deliveries.lock().unwrap().last().unwrap().1.clone();
        let replacement = verifier
            .request_code(Some(&pool), "replacement@example.com", ip)
            .await
            .unwrap();
        let replacement_code = sender.deliveries.lock().unwrap().last().unwrap().1.clone();
        let superseded = verifier
            .verify_code(Some(&pool), old, &old_code)
            .await
            .unwrap_err();
        assert_eq!(superseded.attempts_remaining, Some(0));
        verifier
            .verify_code(Some(&pool), replacement, &replacement_code)
            .await
            .unwrap();
    }
}
