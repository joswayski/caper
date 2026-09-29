//! Short-lived credentials for a coturn server configured with
//! `use-auth-secret` (the TURN REST API scheme): the username is the expiry
//! time and the password is an HMAC of it under the shared secret.

use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

/// Cloudflare's maximum, which the Caper API requests.
const MAX_TTL: u64 = 48 * 60 * 60;
const DEFAULT_TTL: u64 = 24 * 60 * 60;

pub struct Turn {
    secret: Option<String>,
    stun: Vec<String>,
    turn: Vec<String>,
}

impl Turn {
    #[must_use]
    pub fn new(secret: Option<String>, stun: Vec<String>, turn: Vec<String>) -> Self {
        Self { secret, stun, turn }
    }

    /// Cloudflare-shaped `iceServers`. Without TURN configured, only STUN URLs
    /// (if any) are returned; the SFU's public address is reachable directly.
    #[must_use]
    pub fn ice_servers(&self, ttl: Option<u64>) -> Vec<Value> {
        let mut servers = vec![];
        if !self.stun.is_empty() {
            servers.push(json!({ "urls": self.stun }));
        }
        if let Some(secret) = self.secret.as_deref().filter(|_| !self.turn.is_empty()) {
            let ttl = ttl.unwrap_or(DEFAULT_TTL).clamp(60, MAX_TTL);
            let expiry = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
                + ttl;
            let (username, credential) = credential(secret, expiry);
            servers.push(json!({ "urls": self.turn, "username": username, "credential": credential }));
        }
        servers
    }
}

fn credential(secret: &str, expiry: u64) -> (String, String) {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let username = format!("{expiry}:{}", &nonce[..12]);
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(username.as_bytes());
    let password = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    (username, password)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_follow_the_turn_rest_scheme() {
        let (username, password) = credential("secret", 1_700_000_000);
        assert!(username.starts_with("1700000000:"));
        let mut mac = Hmac::<sha1::Sha1>::new_from_slice(b"secret").unwrap();
        mac.update(username.as_bytes());
        let expected = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        assert_eq!(password, expected);
    }

    #[test]
    fn without_turn_only_stun_is_offered() {
        let turn = Turn::new(None, vec!["stun:sfu.example:3478".into()], vec![]);
        assert_eq!(
            turn.ice_servers(Some(10)),
            vec![json!({"urls": ["stun:sfu.example:3478"]})]
        );
        assert!(Turn::new(None, vec![], vec![]).ice_servers(None).is_empty());
    }

    #[test]
    fn ttl_is_bounded() {
        let turn = Turn::new(Some("s".into()), vec![], vec!["turn:sfu.example:3478".into()]);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let servers = turn.ice_servers(Some(u64::MAX));
        let expiry: u64 = servers[0]["username"]
            .as_str()
            .unwrap()
            .split(':')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(expiry <= now + MAX_TTL + 1);
    }
}
