//! Immutable release history for this exact client revision. Acknowledgment
//! belongs to the API origin/platform, never the account or the source SHA.

use serde::Deserialize;

pub const REVISION: &str = env!("CAPER_SOURCE_REVISION");
pub const PLATFORM: &str = if cfg!(windows) { "windows" } else { "linux" };
pub const MAX_BYTES: usize = 64 * 1024;
pub const MAX_SEQUENCE: u64 = 9_007_199_254_740_991;
const INVALID: &str = "Caper returned invalid release highlights. Please try again.";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub sequence: u64,
    pub revision: String,
    pub version: Option<String>,
    pub published_at: String,
    pub highlights: Vec<Highlight>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Highlight {
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Snapshot {
    schema: u64,
    platform: String,
    revision: String,
    pub sequence: u64,
    pub releases: Vec<Release>,
}

impl Snapshot {
    pub fn decode(bytes: &[u8], platform: &str, revision: &str) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err(INVALID.into());
        }
        let snapshot: Self = serde_json::from_slice(bytes).map_err(|_| INVALID.to_owned())?;
        let sha = |value: &str| {
            value.len() == 40
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        };
        let text =
            |value: &str, max: usize| !value.trim().is_empty() && value.chars().count() <= max;
        let mut previous = 0;
        let mut revisions = std::collections::BTreeSet::new();
        if snapshot.schema != 1
            || !matches!(platform, "windows" | "linux")
            || snapshot.platform != platform
            || snapshot.revision != revision
            || !sha(revision)
            || snapshot.releases.is_empty()
            || snapshot.releases.len() > 30
            || snapshot.releases.iter().any(|release| {
                let valid = release.sequence > previous
                    && release.sequence <= MAX_SEQUENCE
                    && sha(&release.revision)
                    && revisions.insert(&release.revision)
                    && release
                        .version
                        .as_ref()
                        .is_none_or(|version| text(version, 64))
                    && chrono::DateTime::parse_from_rfc3339(&release.published_at).is_ok()
                    && release.highlights.len() <= 5
                    && release
                        .highlights
                        .iter()
                        .all(|highlight| text(&highlight.title, 100) && text(&highlight.body, 500));
                previous = release.sequence;
                !valid
            })
            || !snapshot.releases.last().is_some_and(|release| {
                release.sequence == snapshot.sequence && release.revision == snapshot.revision
            })
        {
            return Err(INVALID.into());
        }
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(sequences: &[u64], highlighted: &[u64]) -> serde_json::Value {
        let last = sequences.last().unwrap();
        serde_json::json!({
            "schema": 1, "platform": "linux", "revision": format!("{last:040x}"), "sequence": last,
            "releases": sequences.iter().map(|sequence| serde_json::json!({
                "sequence": sequence, "revision": format!("{sequence:040x}"),
                "publishedAt": "2026-10-11T19:00:00Z",
                "highlights": if highlighted.contains(sequence) {
                    vec![serde_json::json!({"title": format!("Release {sequence}"), "body": "A shipped improvement."})]
                } else { vec![] }
            })).collect::<Vec<_>>()
        })
    }

    #[test]
    fn mismatched_unordered_and_unbounded_snapshots_are_rejected() {
        let valid = payload(&[1, 4], &[4]);
        let decode = |value: &serde_json::Value| {
            Snapshot::decode(
                &serde_json::to_vec(value).unwrap(),
                "linux",
                &format!("{:040x}", 4),
            )
        };
        assert!(decode(&valid).is_ok());
        for (pointer, value) in [
            ("/schema", serde_json::json!(2)),
            ("/platform", serde_json::json!("windows")),
            ("/revision", serde_json::json!("a".repeat(40))),
            ("/sequence", serde_json::json!(3)),
            ("/releases/0/sequence", serde_json::json!(0)),
            ("/releases/0/sequence", serde_json::json!(4)),
            ("/releases/1/sequence", serde_json::json!(4.5)),
            ("/releases/1/revision", serde_json::json!("A".repeat(40))),
            ("/releases/1/publishedAt", serde_json::json!("invalid")),
            (
                "/releases/1/highlights/0/title",
                serde_json::json!("é".repeat(101)),
            ),
            (
                "/releases/1/highlights/0/body",
                serde_json::json!("🫛".repeat(501)),
            ),
            (
                "/releases/1/highlights",
                serde_json::json!(vec![valid["releases"][1]["highlights"][0].clone(); 6]),
            ),
            ("/releases", serde_json::json!([])),
        ] {
            let mut wrong = valid.clone();
            *wrong.pointer_mut(pointer).unwrap() = value;
            assert!(decode(&wrong).is_err(), "{pointer}");
        }
        let mut boundary = valid.clone();
        boundary["releases"][1]["highlights"][0]["title"] = serde_json::json!("é".repeat(100));
        boundary["releases"][1]["highlights"][0]["body"] = serde_json::json!("🫛".repeat(500));
        assert!(
            decode(&boundary).is_ok(),
            "bounds count Unicode code points, not UTF-8 bytes"
        );
        let too_many = payload(&(1..=31).collect::<Vec<_>>(), &[]);
        assert!(
            Snapshot::decode(
                &serde_json::to_vec(&too_many).unwrap(),
                "linux",
                &format!("{:040x}", 31)
            )
            .is_err()
        );
        let unsafe_sequence = payload(&[MAX_SEQUENCE + 1], &[]);
        assert!(
            Snapshot::decode(
                &serde_json::to_vec(&unsafe_sequence).unwrap(),
                "linux",
                &format!("{:040x}", MAX_SEQUENCE + 1)
            )
            .is_err()
        );
        assert!(Snapshot::decode(&vec![b' '; MAX_BYTES + 1], "linux", REVISION).is_err());
    }
}
