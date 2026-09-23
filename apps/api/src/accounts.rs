//! Provider-neutral account persistence. Database errors must be logged without parameters or PII.
use serde::Serialize;
use sqlx::{FromRow, PgPool};
use std::collections::HashSet;

use crate::RuntimeEnvironment;

#[derive(Clone, Debug, Default)]
pub(crate) struct DebugUsers(HashSet<String>);

impl DebugUsers {
    pub(crate) fn from_env(environment: &RuntimeEnvironment) -> Self {
        Self(
            environment
                .get("DEBUG_USERS")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|username| !username.is_empty())
                .map(str::to_ascii_lowercase)
                .collect(),
        )
    }

    pub(crate) fn contains(&self, username: Option<&str>) -> bool {
        username.is_some_and(|username| self.0.contains(&username.to_ascii_lowercase()))
    }
}

#[derive(Clone, Debug, FromRow)]
pub struct User {
    pub id: i64,
    pub external_id: String,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OwnAccount<'a> {
    #[serde(flatten)]
    account: PublicAccount<'a>,
    debug_enabled: bool,
}

impl User {
    pub fn public(&self) -> PublicAccount<'_> {
        PublicAccount {
            id: &self.external_id,
            username: self.username.as_deref(),
            display_name: self.display_name.as_deref(),
        }
    }

    pub(crate) fn own(&self, debug_users: &DebugUsers) -> OwnAccount<'_> {
        OwnAccount {
            account: self.public(),
            debug_enabled: debug_users.contains(self.username.as_deref()),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(value: &str) -> RuntimeEnvironment {
        RuntimeEnvironment::from_values_for_test([("DEBUG_USERS", value)])
    }

    #[test]
    fn debug_users_are_trimmed_and_matched_exactly_without_case_sensitivity() {
        let users = DebugUsers::from_env(&environment(" Alice,BOB , carol "));
        assert!(users.contains(Some("alice")));
        assert!(users.contains(Some("Bob")));
        assert!(users.contains(Some("CAROL")));
        assert!(!users.contains(Some("ali")));
        assert!(!users.contains(Some("alice2")));
        assert!(!users.contains(None));
    }

    #[test]
    fn absent_blank_and_wildcard_values_enable_nobody() {
        assert!(!DebugUsers::from_env(&RuntimeEnvironment::default()).contains(Some("alice")));
        assert!(!DebugUsers::from_env(&environment(" ,  ,")).contains(Some("alice")));
        assert!(!DebugUsers::from_env(&environment("*")).contains(Some("alice")));
    }

    #[test]
    fn own_account_always_reports_eligibility_while_public_account_does_not() {
        let user = User {
            id: 1,
            external_id: "public-id".into(),
            email: None,
            username: Some("alice".into()),
            display_name: Some("Alice".into()),
        };
        let absent = DebugUsers::from_env(&RuntimeEnvironment::default());
        let member = DebugUsers::from_env(&environment("ALICE"));
        assert_eq!(
            serde_json::to_value(user.own(&absent)).unwrap()["debugEnabled"],
            false
        );
        assert_eq!(
            serde_json::to_value(user.own(&member)).unwrap()["debugEnabled"],
            true
        );
        assert!(serde_json::to_value(user.public()).unwrap()["debugEnabled"].is_null());
    }
}
