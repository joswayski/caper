//! Immutable release history for this exact client revision. Acknowledgment
//! belongs to the API origin/platform, never the account or the source SHA.

use serde::Deserialize;
use std::time::{Duration, Instant};

pub const REVISION: &str = env!("CAPER_SOURCE_REVISION");
pub const PLATFORM: &str = if cfg!(windows) { "windows" } else { "linux" };
pub const MAX_BYTES: usize = 64 * 1024;
const MAX_SEQUENCE: u64 = 9_007_199_254_740_991;
const RETRY: Duration = Duration::from_secs(60);
const INVALID: &str = "Caper returned invalid release highlights. Please try again.";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub sequence: u64,
    revision: String,
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
    sequence: u64,
    releases: Vec<Release>,
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

pub struct WhatsNew {
    key: String,
    acknowledged: Option<u64>,
    snapshot: Option<Snapshot>,
    last_attempt: Option<Instant>,
    pub loading: bool,
    pub error: Option<String>,
    /// Cutoff frozen when a view opens: zero for manually retained history.
    viewing: Option<u64>,
    shown_sequence: Option<u64>,
}

impl WhatsNew {
    pub fn new(origin: &str, platform: &str) -> Self {
        Self {
            key: format!("whats-new-v1:{origin}:{platform}"),
            acknowledged: None,
            snapshot: None,
            last_attempt: None,
            loading: false,
            error: None,
            viewing: None,
            shown_sequence: None,
        }
    }

    pub fn restore(&mut self, storage: &dyn eframe::Storage) {
        self.acknowledged = storage
            .get_string(&self.key)
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| (1..=MAX_SEQUENCE).contains(value));
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        if let Some(sequence) = self.acknowledged {
            storage.set_string(&self.key, sequence.to_string());
        }
    }

    /// Foreground retries are bounded too. Explicit Retry can try immediately,
    /// but never overlaps a request or refetches a successful immutable snapshot.
    pub fn begin(&mut self, now: Instant, manual: bool) -> bool {
        if self.loading
            || self.snapshot.is_some()
            || (!manual
                && self
                    .last_attempt
                    .is_some_and(|at| now.duration_since(at) < RETRY))
        {
            return false;
        }
        self.last_attempt = Some(now);
        self.loading = true;
        self.error = None;
        true
    }

    pub fn finish(&mut self, result: Result<Snapshot, String>) {
        self.loading = false;
        match result {
            Ok(snapshot) => {
                let unread = self.acknowledged.is_some_and(|ack| {
                    snapshot
                        .releases
                        .iter()
                        .any(|release| release.sequence > ack && !release.highlights.is_empty())
                });
                if self.acknowledged.is_none() || !unread {
                    self.acknowledge(snapshot.sequence);
                }
                self.snapshot = Some(snapshot);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn acknowledge(&mut self, sequence: u64) {
        self.acknowledged = Some(self.acknowledged.unwrap_or(0).max(sequence));
    }

    pub fn open_unread(&mut self) -> bool {
        if self.viewing.is_some() {
            return false;
        }
        let ack = self.acknowledged.unwrap_or(0);
        let unread = self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .releases
                .iter()
                .any(|release| release.sequence > ack && !release.highlights.is_empty())
        });
        if unread {
            self.viewing = Some(ack);
        }
        unread
    }

    pub fn open_history(&mut self) {
        self.viewing = Some(0);
    }

    /// Called only while drawing the modal, so closing a pending/failed load
    /// never acknowledges metadata that was not displayed.
    pub fn displayed(&mut self) -> Vec<Release> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        self.shown_sequence = Some(snapshot.sequence);
        snapshot
            .releases
            .iter()
            .rev()
            .filter(|release| {
                release.sequence > self.viewing.unwrap_or(0) && !release.highlights.is_empty()
            })
            .cloned()
            .collect()
    }

    pub fn dismiss(&mut self) {
        if let Some(sequence) = self.shown_sequence.take() {
            self.acknowledge(sequence);
        }
        self.viewing = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Storage(std::collections::BTreeMap<String, String>);
    impl eframe::Storage for Storage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn flush(&mut self) {}
    }

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

    fn load(state: &mut WhatsNew, sequences: &[u64], highlighted: &[u64]) {
        let value = payload(sequences, highlighted);
        state.finish(Snapshot::decode(
            &serde_json::to_vec(&value).unwrap(),
            "linux",
            value["revision"].as_str().unwrap(),
        ));
    }

    #[test]
    fn baseline_catch_up_silent_rollback_and_origin_storage() {
        let mut storage = Storage::default();
        let mut first = WhatsNew::new("https://caper.test", "linux");
        load(&mut first, &[1, 4], &[1, 4]);
        assert!(!first.open_unread(), "first install seeds quietly");
        first.save(&mut storage);

        let mut updated = WhatsNew::new("https://caper.test", "linux");
        updated.restore(&storage);
        load(&mut updated, &[1, 4, 6, 8, 10], &[1, 4, 6, 8]);
        assert!(updated.open_unread());
        assert_eq!(
            updated
                .displayed()
                .iter()
                .map(|release| release.sequence)
                .collect::<Vec<_>>(),
            [8, 6]
        );
        updated.dismiss();
        assert!(!updated.open_unread());
        updated.save(&mut storage);

        let mut rollback = WhatsNew::new("https://caper.test", "linux");
        rollback.restore(&storage);
        load(&mut rollback, &[1, 4], &[1, 4]);
        rollback.open_history();
        assert_eq!(rollback.displayed().len(), 2);
        rollback.dismiss();
        rollback.save(&mut storage);
        let mut next = WhatsNew::new("https://caper.test", "linux");
        next.restore(&storage);
        load(&mut next, &[8, 10, 11], &[8]);
        assert!(
            !next.open_unread(),
            "silent releases and rollback never replay old highlights"
        );
        next.save(&mut storage);
        let mut catch_up = WhatsNew::new("https://caper.test", "linux");
        catch_up.restore(&storage);
        load(&mut catch_up, &[10, 11, 12], &[10, 11, 12]);
        assert!(catch_up.open_unread());
        assert_eq!(catch_up.displayed()[0].sequence, 12);
        assert_eq!(catch_up.displayed().len(), 1);

        for (origin, platform) in [
            ("https://staging.test", "linux"),
            ("https://caper.test", "windows"),
        ] {
            let mut other = WhatsNew::new(origin, platform);
            other.restore(&storage);
            load(&mut other, &[12], &[12]);
            assert!(
                !other.open_unread(),
                "new origin/platform gets its own baseline"
            );
        }
    }

    #[test]
    fn failures_and_unseen_loads_do_not_acknowledge_and_retry_is_bounded() {
        let mut storage = Storage::default();
        let mut state = WhatsNew::new("https://caper.test", "linux");
        let now = Instant::now();
        assert!(state.begin(now, false));
        assert!(!state.begin(now, true), "no overlapping requests");
        state.open_history();
        state.finish(Err("not published".into()));
        state.dismiss();
        state.save(&mut storage);
        assert!(storage.0.is_empty());
        assert!(!state.begin(now + Duration::from_secs(59), false));
        assert!(state.begin(now + Duration::from_secs(60), false));
        load(&mut state, &[1], &[1]);
        state.save(&mut storage);
        let mut updated = WhatsNew::new("https://caper.test", "linux");
        updated.restore(&storage);
        load(&mut updated, &[1, 2], &[2]);
        assert!(updated.open_unread());
        updated.dismiss(); // Closed before any frame displayed the snapshot.
        updated.save(&mut storage);
        assert!(updated.open_unread(), "unseen highlights remain unread");
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
