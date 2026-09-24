use crate::model::{History, SpaceDetail};
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

#[derive(Clone)]
pub struct Read {
    pub detail: Option<SpaceDetail>,
    pub history: Option<History>,
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

    pub fn finish_prefetch(&mut self, target: &Target, request: u64, read: Read, now: Instant) {
        self.expire(now);
        if let Some(entry) = self.prefetches.get_mut(target)
            && entry.request == request
        {
            entry.read = Some(read);
        }
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
}
