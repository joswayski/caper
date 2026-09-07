use crate::{ApiError, accounts};
use axum::http::StatusCode;
use sqlx::PgPool;

#[derive(Clone)]
pub(crate) struct AuthVerifier {
    #[cfg(test)]
    bypass: bool,
}

#[derive(Clone)]
pub(crate) struct Principal {
    pub user: accounts::User,
    pub expires_at: u64,
}

impl AuthVerifier {
    pub fn new() -> Self {
        Self {
            #[cfg(test)]
            bypass: false,
        }
    }

    #[cfg(test)]
    pub fn test_bypass() -> Self {
        Self { bypass: true }
    }

    #[cfg(test)]
    pub fn is_test_bypass(&self) -> bool {
        self.bypass
    }

    pub async fn authenticate(
        &self,
        _token: &str,
        _pool: Option<&PgPool>,
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
                expires_at: u64::MAX,
            });
        }

        Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "account service unavailable",
        ))
    }
}
