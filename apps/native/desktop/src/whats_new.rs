//! What's new in this client revision (`caper_client::whats_new`), with the
//! acknowledgment saved in eframe storage.

use caper_client::whats_new::MAX_SEQUENCE;
pub use caper_client::whats_new::{PLATFORM, REVISION, Release, Snapshot};
use std::time::{Duration, Instant};

const RETRY: Duration = Duration::from_secs(60);

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

    /// The update review already showed this build's highlights; installing
    /// from it marks them read so the new build does not open them again.
    pub fn mark_read(&mut self, sequence: u64) {
        if (1..=MAX_SEQUENCE).contains(&sequence) {
            self.acknowledge(sequence);
        }
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
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
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
    fn installing_from_the_update_review_does_not_reopen_the_same_highlights() {
        let mut storage = Storage::default();
        let mut state = WhatsNew::new("https://caper.test", "linux");
        load(&mut state, &[1], &[1]);
        state.mark_read(3);
        state.mark_read(0);
        state.save(&mut storage);
        // The restarted build fetches its own snapshot, the one the review showed.
        let mut updated = WhatsNew::new("https://caper.test", "linux");
        updated.restore(&storage);
        load(&mut updated, &[1, 2, 3], &[2, 3]);
        assert!(!updated.open_unread(), "already read in the update review");
        // A later release the review never showed still opens.
        let mut later = WhatsNew::new("https://caper.test", "linux");
        later.restore(&storage);
        load(&mut later, &[1, 2, 3, 4], &[4]);
        assert!(later.open_unread());
    }
}
