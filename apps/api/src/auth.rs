use crate::{ApiError, accounts};
use axum::http::StatusCode;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_IDENTITY_CACHE: usize = 1024;

#[derive(Clone)]
pub(crate) struct AuthVerifier {
    inner: Option<Arc<Configured>>,
    #[cfg(test)]
    bypass: bool,
}

struct Configured {
    client_id: String,
    api_key: String,
    http: Client,
    jwks: Mutex<Option<CachedKeys>>,
    identities: Mutex<HashMap<String, CachedIdentity>>,
}

struct CachedKeys {
    set: JwkSet,
    fetched: Instant,
}
struct CachedIdentity {
    email: String,
    fetched: Instant,
}

#[derive(Clone)]
pub(crate) struct Principal {
    pub user: accounts::User,
    pub expires_at: u64,
}

#[derive(Debug, Deserialize, Serialize)]
struct Claims {
    iss: String,
    sub: String,
    sid: String,
    client_id: String,
    exp: u64,
    iat: u64,
    #[serde(default)]
    aud: Option<Audience>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
struct WorkosUser {
    id: String,
    email: String,
    email_verified: bool,
}

impl AuthVerifier {
    pub fn new(client_id: Option<String>, api_key: Option<String>) -> Self {
        let inner = match (client_id, api_key) {
            (Some(client_id), Some(api_key)) => Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(3))
                .https_only(true)
                .build()
                .ok()
                .map(|http| {
                    Arc::new(Configured {
                        client_id,
                        api_key,
                        http,
                        jwks: Mutex::new(None),
                        identities: Mutex::new(HashMap::new()),
                    })
                }),
            _ => None,
        };
        Self {
            inner,
            #[cfg(test)]
            bypass: false,
        }
    }

    #[cfg(test)]
    pub fn test_bypass() -> Self {
        Self {
            inner: None,
            bypass: true,
        }
    }

    #[cfg(test)]
    pub fn is_test_bypass(&self) -> bool {
        self.bypass
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
                    public_id: "123456789012345678901".into(),
                    workos_user_id: "user_test".into(),
                    email: "test@example.com".into(),
                    username: Some("test".into()),
                    display_name: Some("Test User".into()),
                },
                expires_at: u64::MAX,
            });
        }
        let configured = self.inner.as_ref().ok_or_else(unavailable)?;
        let pool = pool.ok_or_else(unavailable)?;
        let claims = configured.verify(token).await?;
        let email = configured.verified_email(&claims.sub).await?;
        let user = accounts::sync_workos_user(pool, &claims.sub, &email)
            .await
            .map_err(|_| {
                tracing::error!("account synchronization failed");
                unavailable()
            })?;
        Ok(Principal {
            user,
            expires_at: claims.exp,
        })
    }
}

impl Configured {
    async fn verify(&self, token: &str) -> Result<Claims, ApiError> {
        let header = decode_header(token).map_err(|_| unauthorized())?;
        if header.alg != Algorithm::RS256 {
            return Err(unauthorized());
        }
        let kid = header.kid.ok_or_else(unauthorized)?;
        let mut cache = self.jwks.lock().await;
        let expired = cache
            .as_ref()
            .is_none_or(|c| c.fetched.elapsed() >= CACHE_TTL);
        if expired {
            let url = format!("https://api.workos.com/sso/jwks/{}", self.client_id);
            let set = self
                .http
                .get(url)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(|_| unavailable())?
                .json::<JwkSet>()
                .await
                .map_err(|_| unavailable())?;
            if set.keys.len() > 16 {
                return Err(unavailable());
            }
            *cache = Some(CachedKeys {
                set,
                fetched: Instant::now(),
            });
        }
        let key = cache
            .as_ref()
            .and_then(|c| c.set.find(&kid))
            .map(DecodingKey::from_jwk)
            .transpose()
            .map_err(|_| unauthorized())?
            .ok_or_else(unauthorized)?;
        drop(cache);
        validate(token, &key, &self.client_id)
    }

    async fn verified_email(&self, subject: &str) -> Result<String, ApiError> {
        if let Some(email) = self
            .identities
            .lock()
            .await
            .get(subject)
            .filter(|v| v.fetched.elapsed() < CACHE_TTL)
            .map(|v| v.email.clone())
        {
            return Ok(email);
        }
        let response = self
            .http
            .get(format!(
                "https://api.workos.com/user_management/users/{subject}"
            ))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| unavailable())?;
        let user: WorkosUser = response.json().await.map_err(|_| unavailable())?;
        if user.id != subject
            || !user.email_verified
            || accounts::normalize_email(&user.email).len() > 254
        {
            return Err(unauthorized());
        }
        let mut cache = self.identities.lock().await;
        if cache.len() >= MAX_IDENTITY_CACHE {
            cache.retain(|_, v| v.fetched.elapsed() < CACHE_TTL);
        }
        if cache.len() >= MAX_IDENTITY_CACHE {
            cache.clear();
        }
        cache.insert(
            subject.into(),
            CachedIdentity {
                email: user.email.clone(),
                fetched: Instant::now(),
            },
        );
        Ok(user.email)
    }
}

fn validate(token: &str, key: &DecodingKey, client_id: &str) -> Result<Claims, ApiError> {
    let mut validation = Validation::new(Algorithm::RS256);
    // Verified against an actual free-domain staging AuthKit token. The hosted
    // authkit.app UI hostname is NOT the access-token issuer.
    let issuer = format!("https://api.workos.com/user_management/{client_id}");
    validation.set_issuer(&[&issuer]);
    validation.set_required_spec_claims(&["exp", "iss", "sub"]);
    validation.leeway = 0;
    validation.validate_nbf = true;
    validation.validate_aud = false;
    let claims = decode::<Claims>(token, key, &validation)
        .map_err(|_| unauthorized())?
        .claims;
    let audience_ok = match &claims.aud {
        None => true,
        Some(Audience::One(value)) => value == client_id,
        Some(Audience::Many(values)) => values.iter().any(|value| value == client_id),
    };
    if claims.iss != issuer
        || claims.client_id != client_id
        || !claims.sub.starts_with("user_")
        || !claims
            .sub
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        || !claims.sid.starts_with("session_")
        || claims.iat == 0
        || !audience_ok
    {
        return Err(unauthorized());
    }
    Ok(claims)
}

fn unauthorized() -> ApiError {
    ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized")
}
fn unavailable() -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "account service unavailable",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use rsa::{
        RsaPrivateKey,
        pkcs8::{EncodePrivateKey, EncodePublicKey},
    };

    fn keys() -> (EncodingKey, DecodingKey) {
        let private = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
        let private_pem = private.to_pkcs8_pem(Default::default()).unwrap();
        let public_pem = private
            .to_public_key()
            .to_public_key_pem(Default::default())
            .unwrap();
        (
            EncodingKey::from_rsa_pem(private_pem.as_bytes()).unwrap(),
            DecodingKey::from_rsa_pem(public_pem.as_bytes()).unwrap(),
        )
    }

    fn claims() -> Claims {
        Claims {
            iss: "https://api.workos.com/user_management/client_fixture".into(),
            sub: "user_fixture".into(),
            sid: "session_fixture".into(),
            client_id: "client_fixture".into(),
            exp: 4_000_000_000,
            iat: 1,
            aud: None,
        }
    }

    #[test]
    fn validates_workos_claims_and_optional_audience() {
        let (encoding, decoding) = keys();
        let token = encode(&Header::new(Algorithm::RS256), &claims(), &encoding).unwrap();
        assert!(validate(&token, &decoding, "client_fixture").is_ok());
        let mut claims = claims();
        claims.aud = Some(Audience::Many(vec![
            "other".into(),
            "client_fixture".into(),
        ]));
        let token = encode(&Header::new(Algorithm::RS256), &claims, &encoding).unwrap();
        assert!(validate(&token, &decoding, "client_fixture").is_ok());
        claims.aud = Some(Audience::One("wrong".into()));
        let token = encode(&Header::new(Algorithm::RS256), &claims, &encoding).unwrap();
        assert_eq!(
            validate(&token, &decoding, "client_fixture")
                .unwrap_err()
                .status,
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn rejects_wrong_signature_algorithm_and_missing_claims() {
        let (encoding, decoding) = keys();
        let (_, other_key) = keys();
        let token = encode(&Header::new(Algorithm::RS256), &claims(), &encoding).unwrap();
        assert!(validate(&token, &other_key, "client_fixture").is_err());
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims(),
            &EncodingKey::from_secret(b"not-an-rsa-key"),
        )
        .unwrap();
        assert!(validate(&token, &decoding, "client_fixture").is_err());
        for field in ["exp", "iss", "sub", "sid", "client_id", "iat"] {
            let mut value = serde_json::to_value(claims()).unwrap();
            value.as_object_mut().unwrap().remove(field);
            let token = encode(&Header::new(Algorithm::RS256), &value, &encoding).unwrap();
            assert!(validate(&token, &decoding, "client_fixture").is_err());
        }
    }

    #[test]
    fn rejects_wrong_scope_issuer_session_and_expiry() {
        let (encoding, decoding) = keys();
        for mutate in [0, 1, 2, 3] {
            let mut claims = claims();
            match mutate {
                0 => claims.client_id = "wrong".into(),
                1 => claims.iss = "https://example.com/".into(),
                2 => claims.sid.clear(),
                _ => claims.exp = 1,
            }
            let token = encode(&Header::new(Algorithm::RS256), &claims, &encoding).unwrap();
            assert!(validate(&token, &decoding, "client_fixture").is_err());
        }
    }
}
