use crate::model::{History, PersonUpdate, Space, SpaceDetail};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

const PREFETCH_LIMIT: usize = 4;
const VISITED_LIMIT: usize = 20;
const PREFETCH_TTL: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    pub space: Option<String>,
    pub channel: Option<String>,
}

/// The space and channel an account last opened, kept across launches like
/// web's `caper:space-landing:<account>`. IDs only, per account and device;
/// direct messages, visits and unjoined previews are never stored.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SpaceLanding {
    pub space: String,
    #[serde(default)]
    pub channel: Option<String>,
}

#[derive(Default)]
pub struct SpaceLandings(HashMap<String, SpaceLanding>);

impl SpaceLandings {
    pub const STORAGE_KEY: &'static str = "space-landing-v1";

    /// Missing or corrupt storage falls back to the first joined channel.
    pub fn restore(json: &str) -> Self {
        Self(serde_json::from_str(json).unwrap_or_default())
    }

    pub fn to_json(&self) -> Option<String> {
        serde_json::to_string(&self.0).ok()
    }

    pub fn get(&self, account: &str) -> Option<&SpaceLanding> {
        self.0
            .get(account)
            .filter(|landing| !landing.space.is_empty())
    }

    pub fn set(&mut self, account: &str, landing: SpaceLanding) {
        if !account.is_empty() && !landing.space.is_empty() {
            self.0.insert(account.to_owned(), landing);
        }
    }
}

/// Launch opens the saved landing's space while it is still listed, else the first.
pub fn initial_space(spaces: &[&str], saved: Option<&SpaceLanding>) -> Option<String> {
    saved
        .map(|landing| landing.space.as_str())
        .filter(|space| spaces.contains(space))
        .or_else(|| spaces.first().copied())
        .map(str::to_owned)
}

#[derive(Clone)]
pub struct Read {
    pub detail: Option<SpaceDetail>,
    pub history: Option<History>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefetchState {
    Missing,
    Pending,
    Ready,
}

struct Prefetch {
    request: u64,
    expires: Instant,
    read: Option<Read>,
}

pub struct NavigationCache {
    prefetches: HashMap<Target, Prefetch>,
    prefetch_order: VecDeque<Target>,
    visited: HashMap<Target, History>,
    visited_order: VecDeque<Target>,
    last_channel: HashMap<String, String>,
    next_request: u64,
}

impl Default for NavigationCache {
    fn default() -> Self {
        Self {
            prefetches: HashMap::new(),
            prefetch_order: VecDeque::new(),
            visited: HashMap::new(),
            visited_order: VecDeque::new(),
            last_channel: HashMap::new(),
            next_request: 1,
        }
    }
}

impl NavigationCache {
    pub fn begin_prefetch(&mut self, target: Target, now: Instant) -> Option<u64> {
        self.expire(now);
        if self.prefetches.contains_key(&target) {
            return None;
        }
        let request = self.next_request;
        self.next_request += 1;
        self.prefetch_order.push_back(target.clone());
        self.prefetches.insert(
            target,
            Prefetch {
                request,
                expires: now + PREFETCH_TTL,
                read: None,
            },
        );
        while self.prefetches.len() > PREFETCH_LIMIT {
            if let Some(oldest) = self.prefetch_order.pop_front() {
                self.prefetches.remove(&oldest);
            }
        }
        Some(request)
    }

    pub fn finish_prefetch(
        &mut self,
        target: &Target,
        request: u64,
        read: Read,
        now: Instant,
    ) -> bool {
        self.expire(now);
        if let Some(entry) = self.prefetches.get_mut(target)
            && entry.request == request
        {
            entry.read = Some(read);
            return true;
        }
        false
    }

    pub fn prefetch_state(&mut self, target: &Target, now: Instant) -> PrefetchState {
        self.expire(now);
        match self.prefetches.get(target) {
            Some(entry) if entry.read.is_some() => PrefetchState::Ready,
            Some(_) => PrefetchState::Pending,
            None => PrefetchState::Missing,
        }
    }

    pub fn cancel_prefetch(&mut self, target: &Target, request: u64, now: Instant) -> bool {
        self.expire(now);
        if self
            .prefetches
            .get(target)
            .is_some_and(|entry| entry.request == request)
        {
            self.prefetch_order.retain(|entry| entry != target);
            self.prefetches.remove(target);
            return true;
        }
        false
    }

    pub fn take_prefetch(&mut self, target: &Target, now: Instant) -> Option<Read> {
        self.expire(now);
        self.prefetch_order.retain(|entry| entry != target);
        self.prefetches.remove(target).and_then(|entry| entry.read)
    }

    pub fn remember(&mut self, target: Target, history: History) {
        if let (Some(space), Some(channel)) = (&target.space, &target.channel) {
            self.last_channel.insert(space.clone(), channel.clone());
        }
        self.visited_order.retain(|entry| entry != &target);
        self.visited_order.push_back(target.clone());
        self.visited.insert(target, history);
        while self.visited.len() > VISITED_LIMIT {
            if let Some(oldest) = self.visited_order.pop_front() {
                self.visited.remove(&oldest);
            }
        }
        self.last_channel.retain(|space, channel| {
            self.visited.contains_key(&Target {
                space: Some(space.clone()),
                channel: Some(channel.clone()),
            })
        });
    }

    pub fn history(&mut self, target: &Target) -> Option<History> {
        let target = self.resolve(target);
        let history = self.visited.get(&target)?.clone();
        self.visited_order.retain(|entry| entry != &target);
        self.visited_order.push_back(target);
        Some(history)
    }

    pub fn resolve(&self, target: &Target) -> Target {
        if target.channel.is_some() {
            return target.clone();
        }
        Target {
            space: target.space.clone(),
            channel: target
                .space
                .as_ref()
                .and_then(|space| self.last_channel.get(space).cloned()),
        }
    }

    /// A person's saved name and picture in retained conversations and
    /// prefetched member lists, so navigating back never shows the old ones.
    pub fn apply_person(&mut self, update: &PersonUpdate) {
        let (histories, details) = self.retained();
        for history in histories {
            for message in history
                .messages
                .iter_mut()
                .chain(history.pinned_messages.iter_mut())
            {
                message.apply_person(update);
            }
        }
        for member in details.into_iter().flat_map(|detail| &mut detail.members) {
            member.apply_person(update);
        }
    }

    /// A space's new name and icon in prefetched details and the retained
    /// conversations' headers.
    pub fn apply_space(&mut self, update: &Space) {
        let (histories, details) = self.retained();
        for history in histories {
            if history.space.id == update.id {
                history.space.name.clone_from(&update.name);
            }
        }
        for detail in details {
            detail.space.apply_space(update);
        }
    }

    fn retained(&mut self) -> (Vec<&mut History>, Vec<&mut SpaceDetail>) {
        let mut histories: Vec<&mut History> = self.visited.values_mut().collect();
        let mut details = Vec::new();
        for read in self
            .prefetches
            .values_mut()
            .filter_map(|prefetch| prefetch.read.as_mut())
        {
            histories.extend(read.history.as_mut());
            details.extend(read.detail.as_mut());
        }
        (histories, details)
    }

    pub fn forget_space(&mut self, space: &str) {
        self.prefetches
            .retain(|target, _| target.space.as_deref() != Some(space));
        self.prefetch_order
            .retain(|target| target.space.as_deref() != Some(space));
        self.visited
            .retain(|target, _| target.space.as_deref() != Some(space));
        self.visited_order
            .retain(|target| target.space.as_deref() != Some(space));
        self.last_channel.remove(space);
    }

    pub fn forget_channel(&mut self, channel: &str) {
        self.prefetches.retain(|target, _| {
            target.channel.is_some() && target.channel.as_deref() != Some(channel)
        });
        self.prefetch_order
            .retain(|target| self.prefetches.contains_key(target));
        self.visited
            .retain(|target, _| target.channel.as_deref() != Some(channel));
        self.visited_order
            .retain(|target| target.channel.as_deref() != Some(channel));
        self.last_channel.retain(|_, value| value != channel);
    }

    pub fn clear(&mut self) {
        *self = Self {
            next_request: self.next_request,
            ..Self::default()
        };
    }

    fn expire(&mut self, now: Instant) {
        self.prefetches.retain(|_, entry| entry.expires > now);
        self.prefetch_order
            .retain(|target| self.prefetches.contains_key(target));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HistoryPlace;

    fn history(id: usize, cursor: &str) -> History {
        History {
            messages: vec![],
            pinned_messages: vec![],
            cursor: cursor.into(),
            has_more: false,
            space: HistoryPlace {
                id: "s".into(),
                name: "S".into(),
            },
            channel: HistoryPlace {
                id: format!("c{id}"),
                name: format!("C{id}"),
            },
            visitor: false,
        }
    }

    fn target(id: usize) -> Target {
        Target {
            space: Some("s".into()),
            channel: Some(format!("c{id}")),
        }
    }

    #[test]
    fn prefetch_is_four_entry_five_second_single_use() {
        let now = Instant::now();
        let mut cache = NavigationCache::default();
        for id in 0..5 {
            cache.begin_prefetch(target(id), now).unwrap();
        }
        assert!(cache.take_prefetch(&target(0), now).is_none());
        let request = cache.begin_prefetch(target(9), now).unwrap();
        cache.finish_prefetch(
            &target(9),
            request,
            Read {
                detail: None,
                history: Some(history(9, "8")),
            },
            now,
        );
        assert_eq!(
            cache
                .take_prefetch(&target(9), now)
                .unwrap()
                .history
                .unwrap()
                .cursor,
            "8"
        );
        assert!(cache.take_prefetch(&target(9), now).is_none());
        let request = cache.begin_prefetch(target(10), now).unwrap();
        cache.finish_prefetch(
            &target(10),
            request,
            Read {
                detail: None,
                history: Some(history(10, "9")),
            },
            now,
        );
        assert!(
            cache
                .take_prefetch(&target(10), now + Duration::from_secs(5))
                .is_none()
        );
    }

    #[test]
    fn visited_is_bounded_and_keeps_latest_cursor_and_space_channel() {
        let mut cache = NavigationCache::default();
        for id in 0..21 {
            cache.remember(target(id), history(id, &id.to_string()));
        }
        assert!(cache.history(&target(0)).is_none());
        assert_eq!(cache.history(&target(20)).unwrap().cursor, "20");
        assert_eq!(
            cache
                .resolve(&Target {
                    space: Some("s".into()),
                    channel: None
                })
                .channel
                .as_deref(),
            Some("c20")
        );
    }

    #[test]
    fn cleared_completion_cannot_repopulate_cache() {
        let now = Instant::now();
        let mut cache = NavigationCache::default();
        let request = cache.begin_prefetch(target(1), now).unwrap();
        cache.clear();
        let replacement = cache.begin_prefetch(target(1), now).unwrap();
        assert_ne!(request, replacement);
        cache.finish_prefetch(
            &target(1),
            request,
            Read {
                detail: None,
                history: Some(history(1, "1")),
            },
            now,
        );
        assert!(cache.take_prefetch(&target(1), now).is_none());
    }

    #[test]
    fn pending_prefetch_is_retained_until_its_matching_completion() {
        let now = Instant::now();
        let mut cache = NavigationCache::default();
        let request = cache.begin_prefetch(target(1), now).unwrap();
        assert_eq!(
            cache.prefetch_state(&target(1), now),
            PrefetchState::Pending
        );
        assert!(cache.finish_prefetch(
            &target(1),
            request,
            Read {
                detail: None,
                history: Some(history(1, "7")),
            },
            now,
        ));
        assert_eq!(cache.prefetch_state(&target(1), now), PrefetchState::Ready);
        assert_eq!(
            cache
                .take_prefetch(&target(1), now)
                .unwrap()
                .history
                .unwrap()
                .cursor,
            "7"
        );
    }

    #[test]
    fn space_landing_round_trips_per_account_and_picks_a_listed_space() {
        let mut landings = SpaceLandings::default();
        landings.set(
            "a",
            SpaceLanding {
                space: "s2".into(),
                channel: Some("c2".into()),
            },
        );
        let restored = SpaceLandings::restore(&landings.to_json().unwrap());
        let saved = restored.get("a").cloned();
        assert_eq!(
            saved.as_ref().and_then(|l| l.channel.as_deref()),
            Some("c2")
        );
        assert!(
            restored.get("b").is_none(),
            "another account never inherits it"
        );
        assert!(SpaceLandings::restore("not json").get("a").is_none());
        assert_eq!(
            initial_space(&["s1", "s2"], saved.as_ref()).as_deref(),
            Some("s2")
        );
        let gone = SpaceLanding {
            space: "s9".into(),
            channel: None,
        };
        assert_eq!(
            initial_space(&["s1", "s2"], Some(&gone)).as_deref(),
            Some("s1")
        );
        assert_eq!(initial_space(&[], saved.as_ref()), None);
    }

    #[test]
    fn direct_conversation_target_is_global_and_not_resolved_through_a_space() {
        let mut cache = NavigationCache::default();
        let direct = Target {
            space: None,
            channel: Some("dm0000000001".into()),
        };
        let mut direct_history = history(1, "4");
        direct_history.space.id.clear();
        direct_history.space.name = "Direct messages".into();
        direct_history.channel.id = "dm0000000001".into();
        cache.remember(direct.clone(), direct_history);

        assert_eq!(cache.resolve(&direct), direct);
        assert_eq!(
            cache.history(&direct).unwrap().space.name,
            "Direct messages"
        );
    }
}
