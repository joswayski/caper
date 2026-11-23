use chrono::{DateTime, Utc};
use std::time::{Duration, Instant};

pub const AVATAR_COUNT: usize = 800;

/// Deterministic branding character for an empty DM or channel (matches web/Apple/Android).
pub fn conversation_index(seed: &str) -> usize {
    let mut hash: u32 = 0x811c_9dc5;
    for &byte in seed.as_bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x85eb_ca6b);
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(0xc2b2_ae35);
    hash ^= hash >> 16;
    (hash as usize) % AVATAR_COUNT
}

pub fn select(
    day: &str,
    saved_day: Option<&str>,
    saved_index: Option<usize>,
    random: u64,
) -> usize {
    if saved_day == Some(day) && saved_index.is_some_and(|index| index < AVATAR_COUNT) {
        return saved_index.unwrap();
    }
    if let Some(previous) = saved_index.filter(|&index| index < AVATAR_COUNT) {
        let candidate = (random % (AVATAR_COUNT - 1) as u64) as usize;
        return candidate + usize::from(candidate >= previous);
    }
    (random % AVATAR_COUNT as u64) as usize
}

pub struct DailyIcon {
    pub day: String,
    pub index: usize,
    last_check: Instant,
}

impl DailyIcon {
    /// Today's icon, given the day and index saved last time.
    pub fn load(saved_day: Option<&str>, saved_index: Option<usize>) -> Self {
        let now = Utc::now();
        let day = now.format("%Y-%m-%d").to_string();
        let index = select(&day, saved_day, saved_index, random());
        Self {
            day,
            index,
            last_check: Instant::now(),
        }
    }

    pub fn refresh(&mut self, now: DateTime<Utc>) -> bool {
        if self.last_check.elapsed() < Duration::from_secs(60) {
            return false;
        }
        self.last_check = Instant::now();
        let day = now.format("%Y-%m-%d").to_string();
        if day == self.day {
            return false;
        }
        self.index = select(&day, Some(&self.day), Some(self.index), random());
        self.day = day;
        true
    }
}

fn random() -> u64 {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    u64::from_le_bytes(bytes[..8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_same_day_full_range_and_no_repeat() {
        assert_eq!(select("today", Some("today"), Some(42), 799), 42);
        let selected: std::collections::BTreeSet<_> = (0..800)
            .map(|value| select("new", None, None, value))
            .collect();
        assert_eq!(selected.len(), 800);
        for old in [0, 42, 799] {
            let selected: std::collections::BTreeSet<_> = (0..799)
                .map(|value| select("new", Some("old"), Some(old), value))
                .collect();
            assert_eq!(selected, (0..800).filter(|&index| index != old).collect());
        }
    }

    #[test]
    fn refresh_keeps_same_day_and_rotates_at_utc_boundary() {
        let before = DateTime::from_timestamp(86_399, 0).unwrap();
        let after = DateTime::from_timestamp(86_400, 0).unwrap();
        let mut icon = DailyIcon {
            day: "1970-01-01".into(),
            index: 42,
            last_check: Instant::now() - Duration::from_secs(61),
        };
        assert!(!icon.refresh(before));
        assert_eq!(icon.index, 42);
        assert!(!icon.refresh(after), "checks are throttled between frames");
        icon.last_check = Instant::now() - Duration::from_secs(61);
        assert!(icon.refresh(after));
        assert_eq!(icon.day, "1970-01-02");
        assert_ne!(icon.index, 42);
    }

    #[test]
    fn conversation_index_matches_shared_vectors_and_spreads() {
        assert_eq!(conversation_index("general"), 315);
        assert_eq!(conversation_index("random"), 700);
        assert_eq!(conversation_index("dm-alice"), 161);
        assert_eq!(conversation_index("dm-bob"), 480);
        assert_eq!(
            conversation_index("0194b1e0-0000-7000-8000-000000000001"),
            639
        );
        assert_eq!(
            conversation_index("0194b1e0-0000-7000-8000-000000000002"),
            334
        );
        let spread: std::collections::BTreeSet<_> = (0..100)
            .map(|i| conversation_index(&format!("conv-{i}")))
            .collect();
        assert!(spread.len() > 85);
        assert!(spread.iter().all(|&index| index < AVATAR_COUNT));
    }
}
