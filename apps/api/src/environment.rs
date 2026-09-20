use aws_sdk_secretsmanager::Client;
use serde_json::Value;
use std::{collections::HashMap, time::Duration};

const LOAD_TIMEOUT: Duration = Duration::from_secs(10);
const SECRET_KEYS: &[&str] = &[
    "DATABASE_URL",
    "MIGRATION_DATABASE_URL",
    "AUTH_SECRET",
    "AUTH_CODE_ATTEMPTS",
    "AUTH_EMAIL_15M_LIMIT",
    "AUTH_EMAIL_DAILY_LIMIT",
    "AUTH_IP_HOURLY_LIMIT",
    "AUTH_GLOBAL_HOURLY_LIMIT",
    "SES_FROM_ADDRESS",
    "SES_CONFIGURATION_SET",
    "MEDIA_ENABLED",
    "CF_SFU_APP_ID",
    "CF_SFU_APP_SECRET",
    "CF_TURN_KEY_ID",
    "CF_TURN_API_TOKEN",
    "AXIOM_TOKEN",
    "AXIOM_DATASET",
    "AXIOM_ENDPOINT",
];

/// Application settings loaded from Secrets Manager with process-environment fallback.
///
/// AWS bootstrap credentials deliberately remain process-only: the record cannot
/// replace the workload identity used to read itself. Application secrets, including
/// database URLs, prefer Secrets Manager and fall back to the process environment.
#[derive(Default)]
pub struct RuntimeEnvironment {
    secret_values: HashMap<&'static str, String>,
    secret_loaded: bool,
}

impl RuntimeEnvironment {
    pub async fn load() -> Result<Self, String> {
        let Some(secret_id) = std::env::var("APP_SECRET_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(Self::default());
        };

        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let result = tokio::time::timeout(
            LOAD_TIMEOUT,
            Client::new(&config)
                .get_secret_value()
                .secret_id(&secret_id)
                .send(),
        )
        .await;
        let output = match result {
            Ok(Ok(output)) => output,
            Ok(Err(_)) | Err(_) => {
                eprintln!(
                    "{secret_id} is unavailable; using the process environment and local .env fallback"
                );
                return Ok(Self::default());
            }
        };
        let value = output
            .secret_string()
            .ok_or_else(|| format!("{secret_id} must contain a JSON string"))?;
        Self::from_secret_json(value).map_err(|error| format!("{secret_id} {error}"))
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.secret_values
            .get(key)
            .cloned()
            .or_else(|| std::env::var(key).ok())
    }

    #[must_use]
    pub fn secret_loaded(&self) -> bool {
        self.secret_loaded
    }

    fn from_secret_json(json: &str) -> Result<Self, &'static str> {
        let value: Value = serde_json::from_str(json).map_err(|_| "must contain a JSON object")?;
        let object = value.as_object().ok_or("must contain a JSON object")?;
        let secret_values = SECRET_KEYS
            .iter()
            .filter_map(|key| {
                object
                    .get(*key)
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(|value| (*key, value.to_owned()))
            })
            .collect();
        Ok(Self {
            secret_values,
            secret_loaded: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_allowlisted_string_values() {
        let environment = RuntimeEnvironment::from_secret_json(
            r#"{
                "AUTH_SECRET":"remote-secret",
                "MEDIA_ENABLED":"true",
                "DATABASE_URL":"postgres://runtime",
                "MIGRATION_DATABASE_URL":"postgres://migration",
                "AUTH_CODE_ATTEMPTS":"3",
                "SES_FROM_ADDRESS":"Caper <noreply@example.com>",
                "AWS_ACCESS_KEY_ID":"attacker",
                "CF_SFU_APP_SECRET":42
            }"#,
        )
        .unwrap();

        assert_eq!(
            environment.get("AUTH_SECRET").as_deref(),
            Some("remote-secret")
        );
        assert_eq!(environment.get("MEDIA_ENABLED").as_deref(), Some("true"));
        assert_eq!(
            environment.get("DATABASE_URL").as_deref(),
            Some("postgres://runtime")
        );
        assert_eq!(
            environment.get("MIGRATION_DATABASE_URL").as_deref(),
            Some("postgres://migration")
        );
        assert_eq!(environment.get("AUTH_CODE_ATTEMPTS").as_deref(), Some("3"));
        assert!(!environment.secret_values.contains_key("AWS_ACCESS_KEY_ID"));
        assert!(!environment.secret_values.contains_key("CF_SFU_APP_SECRET"));
    }

    #[test]
    fn rejects_non_object_json() {
        assert!(RuntimeEnvironment::from_secret_json("[]").is_err());
    }
}
