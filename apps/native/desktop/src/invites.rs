//! Shareable space invite links (`https://caper.chat/invite/<code>`): the API
//! shapes, the pasted-link parser and the owner list's wording, shared with
//! web's invite page and settings.
use crate::model::{Inviter, Space};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use url::Url;

/// Codes are exactly ten ASCII letters or digits.
const CODE_LENGTH: usize = 10;

/// "Expire after" units; `Never` sends no expiry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpiryUnit {
    Minutes,
    Hours,
    Days,
    Never,
}

pub const EXPIRY_UNITS: [(ExpiryUnit, &str); 4] = [
    (ExpiryUnit::Minutes, "minutes"),
    (ExpiryUnit::Hours, "hours"),
    (ExpiryUnit::Days, "days"),
    (ExpiryUnit::Never, "Never"),
];
/// Sixteen days from creation.
pub const DEFAULT_EXPIRY: (&str, ExpiryUnit) = ("16", ExpiryUnit::Days);
/// The API's bounds: one minute to a year, and at most 10,000 uses.
const MAX_AGE_SECONDS: std::ops::RangeInclusive<u64> = 60..=31_536_000;
const MAX_USES: std::ops::RangeInclusive<u64> = 1..=10_000;
pub const EXPIRY_ERROR: &str = "Expiry must be between 1 minute and 365 days.";
pub const USES_ERROR: &str = "Max uses must be a whole number from 1 to 10,000.";

fn whole_number(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    // Saturates: an absurdly long number is simply out of range.
    Some(text.parse().unwrap_or(u64::MAX))
}

/// "Expire after" as the API's `maxAge` seconds; `None` never expires.
pub fn expiry_seconds(amount: &str, unit: ExpiryUnit) -> Result<Option<u32>, &'static str> {
    let unit_seconds = match unit {
        ExpiryUnit::Never => return Ok(None),
        ExpiryUnit::Minutes => 60,
        ExpiryUnit::Hours => 3_600,
        ExpiryUnit::Days => 86_400,
    };
    whole_number(amount)
        .map(|amount| amount.saturating_mul(unit_seconds))
        .filter(|seconds| MAX_AGE_SECONDS.contains(seconds))
        .map(|seconds| Some(seconds as u32))
        .ok_or(EXPIRY_ERROR)
}

/// "Max uses"; empty means no limit.
pub fn max_uses(text: &str) -> Result<Option<u32>, &'static str> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    whole_number(text)
        .filter(|uses| MAX_USES.contains(uses))
        .map(|uses| Some(uses as u32))
        .ok_or(USES_ERROR)
}

pub const INVALID_LINK: &str = "This invite link is invalid or has expired.";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct NamedChannel {
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct LinkChannel {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpaceName {
    pub name: String,
    /// The space's custom icon; old servers omit it.
    #[serde(default)]
    pub icon_asset: Option<String>,
    #[serde(default)]
    pub icon_url: Option<String>,
}

impl SpaceName {
    pub fn icon(&self) -> Option<crate::pictures::Source<'_>> {
        crate::pictures::Source::new(&self.icon_asset, &self.icon_url)
    }
}

/// One live link from `GET /api/spaces/{space}/invite-links`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InviteLink {
    pub code: String,
    pub channel: Option<LinkChannel>,
    pub created_by: Inviter,
    pub uses: u32,
    pub max_uses: Option<u32>,
    pub expires_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct InviteLinks {
    pub links: Vec<InviteLink>,
}

/// The public preview behind `GET /api/invites/{code}`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InvitePreview {
    pub code: String,
    pub space: SpaceName,
    pub channel: Option<NamedChannel>,
    pub inviter: Inviter,
    pub member_count: u64,
    pub expires_at: Option<String>,
    #[serde(default)]
    pub already_member: bool,
}

/// `POST /api/invites/{code}/join`. Its `joined` flag (false for an existing
/// member) is not needed: both cases open the returned space.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinedInvite {
    pub space: Space,
    pub channel_id: Option<String>,
}

pub fn valid_code(code: &str) -> bool {
    code.len() == CODE_LENGTH && code.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

/// The code in a pasted invite: a full link on any host (`/invite/<code>`,
/// optionally with a trailing slash, query or fragment), the same without a
/// scheme such as `caper.chat/invite/<code>`, or a bare ten-character code.
pub fn invite_code(input: &str) -> Option<String> {
    let input = input.trim();
    if valid_code(input) {
        return Some(input.into());
    }
    if input.is_empty() || input.chars().any(char::is_whitespace) {
        return None;
    }
    let url = if input.contains("://") {
        Url::parse(input).ok()?
    } else {
        Url::parse(&format!("https://{input}")).ok()?
    };
    if url.scheme() == "caper" {
        // `caper://invite/<code>`, from the web invite page's "Open in the Caper app".
        let segments: Vec<_> = url.path_segments()?.collect();
        return match (url.host_str(), segments.as_slice()) {
            (Some("invite"), [code] | [code, ""]) if valid_code(code) => Some((*code).into()),
            _ => None,
        };
    }
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
        return None;
    }
    let mut segments = url.path_segments()?;
    let (Some("invite"), Some(code)) = (segments.next(), segments.next()) else {
        return None;
    };
    let rest: Vec<_> = segments.collect();
    (valid_code(code) && (rest.is_empty() || rest == [""])).then(|| code.into())
}

/// The shareable link on the configured API origin, so production reads
/// `https://caper.chat/invite/<code>`.
pub fn invite_url(base: &Url, code: &str) -> String {
    format!("{}/invite/{code}", base.origin().ascii_serialization())
}

/// "#feedback · 3 of 10 uses · Expires in 6 days", or "2 uses · Never expires".
pub fn link_meta(link: &InviteLink, now: DateTime<Utc>) -> String {
    let mut parts = Vec::new();
    if let Some(channel) = &link.channel {
        parts.push(format!("#{}", channel.name));
    }
    parts.push(match link.max_uses {
        Some(max) => format!(
            "{} of {}",
            grouped(u64::from(link.uses)),
            plural(u64::from(max), "use")
        ),
        None => plural(u64::from(link.uses), "use"),
    });
    parts.push(match &link.expires_at {
        None => "Never expires".into(),
        Some(expires) => DateTime::parse_from_rfc3339(expires).map_or_else(
            |_| "Expires soon".into(),
            |expires| expires_in(expires.with_timezone(&Utc) - now),
        ),
    });
    parts.join(" · ")
}

fn expires_in(remaining: chrono::TimeDelta) -> String {
    let seconds = remaining.num_seconds();
    if seconds <= 0 {
        return "Expired".into();
    }
    // Round to whole minutes first so a fresh one-hour link reads "1 hour",
    // not "60 minutes".
    let minutes = ((seconds + 30) / 60).max(1) as u64;
    let text = if minutes >= 1_440 {
        plural((minutes + 720) / 1_440, "day")
    } else if minutes >= 60 {
        plural((minutes + 30) / 60, "hour")
    } else {
        plural(minutes, "minute")
    };
    format!("Expires in {text}")
}

/// "1 member", "3 members".
pub fn plural(count: u64, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{} {noun}s", grouped(count))
    }
}

/// "10,000"
fn grouped(count: u64) -> String {
    let digits = count.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_codes_come_from_links_hosts_and_bare_codes() {
        let code = Some("AbC123xYz9".to_owned());
        for input in [
            "AbC123xYz9",
            "  AbC123xYz9\n",
            "https://caper.chat/invite/AbC123xYz9",
            "https://caper.chat/invite/AbC123xYz9/",
            "https://caper.chat/invite/AbC123xYz9?utm=chat#top",
            "http://localhost:3000/invite/AbC123xYz9",
            "https://staging.example.test/invite/AbC123xYz9",
            "caper.chat/invite/AbC123xYz9",
            "caper.chat/invite/AbC123xYz9/",
            "HTTPS://CAPER.CHAT/invite/AbC123xYz9",
            "caper://invite/AbC123xYz9",
            "caper://invite/AbC123xYz9/",
        ] {
            assert_eq!(invite_code(input), code, "{input:?}");
        }
    }

    #[test]
    fn invite_codes_reject_anything_else() {
        for input in [
            "",
            "   ",
            "AbC123xYz",
            "AbC123xYz99",
            "AbC-23xYz9",
            "AbC123 xYz9",
            "https://caper.chat/AbC123xYz9",
            "https://caper.chat/invite/",
            "https://caper.chat/invite/AbC123xYz",
            "https://caper.chat/invite/AbC123xYz9/extra",
            "https://caper.chat/spaces/invite/AbC123xYz9",
            "https://caper.chat/invites/AbC123xYz9",
            "ftp://caper.chat/invite/AbC123xYz9",
            "caper.chat/invite/AbC123xYz9 now",
            "/invite/AbC123xYz9",
            "https://caper.chat/invite/%41bC123xYz9",
            "caper://invite/AbC123xYz",
            "caper://join/AbC123xYz9",
            "caper://invite/AbC123xYz9/extra",
        ] {
            assert_eq!(invite_code(input), None, "{input:?}");
        }
    }

    #[test]
    fn links_use_the_configured_origin() {
        let production = Url::parse("https://caper.chat/").unwrap();
        assert_eq!(
            invite_url(&production, "AbC123xYz9"),
            "https://caper.chat/invite/AbC123xYz9"
        );
        let local = Url::parse("http://127.0.0.1:3001/").unwrap();
        assert_eq!(
            invite_url(&local, "AbC123xYz9"),
            "http://127.0.0.1:3001/invite/AbC123xYz9"
        );
        assert_eq!(
            invite_code(&invite_url(&local, "AbC123xYz9")).as_deref(),
            Some("AbC123xYz9")
        );
    }

    #[test]
    fn invite_json_decodes() {
        let links: InviteLinks = serde_json::from_str(
            r#"{"links":[
                {"code":"AbC123xYz9","channel":{"id":"chan00000002","name":"feedback"},
                 "createdBy":{"username":"owner","displayName":"Owner","avatarId":3},
                 "uses":3,"maxUses":10,"expiresAt":"2026-10-18T12:00:00Z","createdAt":"2026-10-11T12:00:00Z"},
                {"code":"Zz9Yy8Xx7W","channel":null,
                 "createdBy":{"username":"owner","displayName":"Owner","avatarId":3},
                 "uses":2,"maxUses":null,"expiresAt":null,"createdAt":"2026-10-11T12:00:00Z"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(links.links.len(), 2);
        let first = &links.links[0];
        assert_eq!(first.channel.as_ref().unwrap().name, "feedback");
        assert_eq!(first.created_by.display_name, "Owner");
        assert_eq!((first.uses, first.max_uses), (3, Some(10)));
        assert_eq!(links.links[1].channel, None);
        assert_eq!(links.links[1].expires_at, None);

        let preview: InvitePreview = serde_json::from_str(
            r#"{"code":"AbC123xYz9","space":{"name":"Studio"},"channel":{"name":"feedback"},
                "inviter":{"username":"owner","displayName":"Owner","avatarId":3},
                "memberCount":12,"expiresAt":null,"alreadyMember":true}"#,
        )
        .unwrap();
        assert_eq!(preview.space.name, "Studio");
        assert_eq!(preview.channel.unwrap().name, "feedback");
        assert_eq!(preview.inviter.display_name, "Owner");
        assert_eq!(preview.member_count, 12);
        assert!(preview.already_member);

        let joined: JoinedInvite = serde_json::from_str(
            r#"{"space":{"id":"space0000001","name":"Studio","ownerId":"owner01"},
                "channelId":null,"joined":false}"#,
        )
        .unwrap();
        assert_eq!(joined.space.id, "space0000001");
        assert_eq!(joined.space.owner_id, "owner01");
        assert!(joined.space.inviter.is_none());
        assert_eq!(joined.channel_id, None);
    }

    #[test]
    fn meta_lines_match_web() {
        let now = DateTime::parse_from_rfc3339("2026-10-11T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut link = InviteLink {
            code: "AbC123xYz9".into(),
            channel: Some(LinkChannel {
                id: "chan00000002".into(),
                name: "feedback".into(),
            }),
            created_by: Inviter {
                avatar_id: None,
                avatar_asset: None,
                avatar_url: None,
                username: "owner".into(),
                display_name: "Owner".into(),
            },
            uses: 3,
            max_uses: Some(10),
            expires_at: Some("2026-10-17T13:00:00Z".into()),
            created_at: "2026-10-10T13:00:00Z".into(),
        };
        assert_eq!(
            link_meta(&link, now),
            "#feedback · 3 of 10 uses · Expires in 6 days"
        );
        link.channel = None;
        link.max_uses = None;
        link.uses = 2;
        link.expires_at = None;
        assert_eq!(link_meta(&link, now), "2 uses · Never expires");
        link.uses = 1;
        link.expires_at = Some("2026-10-11T12:59:59Z".into());
        assert_eq!(link_meta(&link, now), "1 use · Expires in 1 hour");
        link.expires_at = Some("2026-10-11T12:29:50Z".into());
        assert_eq!(link_meta(&link, now), "1 use · Expires in 30 minutes");
        link.expires_at = Some("2026-10-18T11:59:59Z".into());
        assert_eq!(link_meta(&link, now), "1 use · Expires in 7 days");
        link.expires_at = Some("2026-10-11T18:00:00Z".into());
        assert_eq!(link_meta(&link, now), "1 use · Expires in 6 hours");
        link.expires_at = Some("2026-10-11T11:00:00Z".into());
        assert_eq!(link_meta(&link, now), "1 use · Expired");
    }

    #[test]
    fn expiry_and_max_uses_are_free_form_within_the_api_bounds() {
        assert_eq!(
            expiry_seconds(DEFAULT_EXPIRY.0, DEFAULT_EXPIRY.1),
            Ok(Some(1_382_400))
        );
        assert_eq!(expiry_seconds(" 90 ", ExpiryUnit::Minutes), Ok(Some(5_400)));
        assert_eq!(expiry_seconds("1", ExpiryUnit::Minutes), Ok(Some(60)));
        assert_eq!(
            expiry_seconds("365", ExpiryUnit::Days),
            Ok(Some(31_536_000))
        );
        assert_eq!(expiry_seconds("", ExpiryUnit::Never), Ok(None));
        for (amount, unit) in [
            ("0", ExpiryUnit::Minutes),
            ("", ExpiryUnit::Hours),
            ("366", ExpiryUnit::Days),
            ("8761", ExpiryUnit::Hours),
            ("1.5", ExpiryUnit::Days),
            ("-1", ExpiryUnit::Days),
            ("99999999999999999999999", ExpiryUnit::Days),
        ] {
            assert_eq!(expiry_seconds(amount, unit), Err(EXPIRY_ERROR), "{amount}");
        }
        assert_eq!(max_uses(""), Ok(None));
        assert_eq!(max_uses(" 7 "), Ok(Some(7)));
        assert_eq!(max_uses("10000"), Ok(Some(10_000)));
        for text in ["0", "10001", "2.5", "abc", "-3", "1,000"] {
            assert_eq!(max_uses(text), Err(USES_ERROR), "{text}");
        }
    }

    #[test]
    fn plurals() {
        assert_eq!(plural(1, "member"), "1 member");
        assert_eq!(plural(0, "member"), "0 members");
        assert_eq!(plural(12, "member"), "12 members");
        assert_eq!(plural(10_000, "use"), "10,000 uses");
        assert_eq!(grouped(1_234_567), "1,234,567");
        assert_eq!(grouped(999), "999");
    }
}
