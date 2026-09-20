use crate::{ApiError, accounts, email};
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
const CODE_ATTEMPTS: i16 = 5;

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
}

#[derive(FromRow)]
struct Challenge {
    email: String,
    code_hash: Vec<u8>,
    attempts_remaining: i16,
    expires_at: chrono::DateTime<Utc>,
    consumed_at: Option<chrono::DateTime<Utc>>,
}

impl AuthVerifier {
    pub fn new() -> Self {
        Self {
            enabled: None,
            #[cfg(test)]
            bypass: false,
        }
    }

    pub async fn from_env() -> Result<Self, String> {
        let Some(secret) = std::env::var("AUTH_SECRET")
            .ok()
            .filter(|value| !value.trim().is_empty())
        else {
            tracing::info!("AUTH_SECRET unset; account login remains disabled");
            return Ok(Self::new());
        };
        if secret.len() < 32 {
            return Err("AUTH_SECRET must contain at least 32 bytes".into());
        }
        let sender = email::SesEmailSender::from_env().await?;
        Ok(Self {
            enabled: Some(EnabledAuth {
                secret: Arc::from(secret.into_bytes()),
                sender: Arc::new(sender),
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
        if email_recent >= 3 || email_daily >= 10 || ip_hourly >= 20 || global_hourly >= 500 {
            transaction.commit().await.map_err(database_unavailable)?;
            return Ok(id);
        }

        let code = format!("{:06}", rand::rng().random_range(0..1_000_000_u32));
        let code_hash = code_hash(&auth.secret, id, &email, &code);
        sqlx::query(
            "INSERT INTO public.auth_email_challenges
             (id, email, code_hash, request_ip_hash, attempts_remaining, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(&email)
        .bind(code_hash)
        .bind(ip_hash)
        .bind(CODE_ATTEMPTS)
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
        if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid_code());
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
            return Err(invalid_code());
        };
        let expected = code_hash(&auth.secret, challenge_id, &challenge.email, code);
        let valid = challenge.consumed_at.is_none()
            && challenge.expires_at > Utc::now()
            && challenge.attempts_remaining > 0
            && bool::from(challenge.code_hash.ct_eq(&expected));
        if !valid {
            sqlx::query(
                "UPDATE public.auth_email_challenges
                 SET attempts_remaining = GREATEST(attempts_remaining - 1, 0),
                     consumed_at = CASE WHEN attempts_remaining <= 1 THEN now() ELSE consumed_at END
                 WHERE id = $1 AND consumed_at IS NULL",
            )
            .bind(challenge_id)
            .execute(&mut *transaction)
            .await
            .map_err(database_unavailable)?;
            transaction.commit().await.map_err(database_unavailable)?;
            return Err(invalid_code());
        }

        sqlx::query("UPDATE public.auth_email_challenges SET consumed_at = now() WHERE id = $1")
            .bind(challenge_id)
            .execute(&mut *transaction)
            .await
            .map_err(database_unavailable)?;
        let user: Option<accounts::User> = sqlx::query_as(
            "INSERT INTO public.users (email, email_verified_at)
             VALUES ($1, now())
             ON CONFLICT (email) DO UPDATE SET
                email_verified_at = COALESCE(users.email_verified_at, now()), updated_at = now()
             WHERE users.deleted_at IS NULL
             RETURNING id, email, username, display_name",
        )
        .bind(&challenge.email)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_unavailable)?;
        let Some(user) = user else {
            transaction.commit().await.map_err(database_unavailable)?;
            return Err(invalid_code());
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
        Ok(VerifiedSession { user, token })
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
            "SELECT u.id, u.email, u.username, u.display_name
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

fn invalid_code() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::UNAUTHORIZED,
        "invalid or expired code",
    )
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
            }),
            bypass: false,
        };
        let ip = "192.0.2.10".parse().unwrap();

        let challenge = verifier
            .request_code(Some(&pool), " Person@Example.COM ", ip)
            .await
            .unwrap();
        for _ in 0..3 {
            verifier
                .request_code(Some(&pool), "person@example.com", ip)
                .await
                .unwrap();
        }
        let deliveries = sender.deliveries.lock().unwrap().clone();
        assert_eq!(
            deliveries.len(),
            3,
            "fourth request must be silently throttled"
        );
        assert_eq!(deliveries[0].0, "person@example.com");
        let code = &deliveries[0].1;

        if code != "000000" {
            let wrong = verifier
                .verify_code(Some(&pool), challenge, "000000")
                .await
                .unwrap_err();
            assert_eq!(wrong.status, axum::http::StatusCode::UNAUTHORIZED);
        }
        let session = verifier
            .verify_code(Some(&pool), challenge, code)
            .await
            .unwrap();
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
    }
}
