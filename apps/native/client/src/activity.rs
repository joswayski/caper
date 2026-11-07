//! Account-wide notifications: the inbox rows and pages the API returns.
use serde::Deserialize;
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub kind: String,
    pub actor_name: String,
    pub body: String,
    pub created_at: String,
    pub unread: bool,
    pub space_id: Option<String>,
    pub space_name: Option<String>,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub thread_root_id: Option<String>,
    pub emoji: Option<String>,
}

impl Item {
    pub fn channel(&self) -> Option<&str> {
        self.conversation_id
            .as_deref()
            .or(self.channel_id.as_deref())
    }
    pub fn can_reply(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "message" | "mention" | "reply" | "reaction"
        ) && self.message_id.as_ref().is_some_and(|id| !id.is_empty())
            && self.channel().is_some_and(|id| !id.is_empty())
    }
    pub fn draft_key(&self) -> String {
        format!(
            "inbox:{}",
            [
                self.id.as_str(),
                self.channel().unwrap_or_default(),
                self.message_id.as_deref().unwrap_or_default(),
                self.thread_root_id.as_deref().unwrap_or_default()
            ]
            .map(|part| format!("{}:{part}", part.len()))
            .join("")
        )
    }
    pub fn quote_id(&self) -> Option<&str> {
        self.thread_root_id
            .is_none()
            .then_some(self.message_id.as_deref())
            .flatten()
    }
    pub fn reply_label(&self) -> String {
        if self.thread_root_id.is_some() {
            "Reply in thread".into()
        } else if self.conversation_id.is_some() {
            "Reply in DM".into()
        } else {
            format!(
                "Reply in #{}",
                self.channel_name.as_deref().unwrap_or("channel")
            )
        }
    }
    /// Only existing destinations. Never resolve the actor's username or accept a request.
    pub fn destination(&self, api: &crate::api::Api, token: &str) -> Result<String, String> {
        if !self.can_reply() {
            return Err("This notification cannot be replied to.".into());
        }
        if let Some(id) = &self.conversation_id {
            let directs = api
                .direct_conversations(token)
                .map_err(|error| error.to_string())?;
            if !directs.conversations.iter().any(|direct| {
                &direct.id == id
                    && !direct.blocked
                    && direct.status != crate::model::DirectStatus::Incoming
            }) {
                return Err("This conversation is not available for replies. Open it to review access or requests.".into());
            }
        }
        Ok(self.channel().unwrap().to_owned())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub items: Vec<Item>,
    pub next_cursor: Option<String>,
    pub unread_count: u64,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Read {
    pub unread_count: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Filter {
    #[default]
    All,
    Dms,
    Mentions,
    Threads,
    Reactions,
    Invitations,
}
impl Filter {
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::Dms,
        Self::Mentions,
        Self::Threads,
        Self::Reactions,
        Self::Invitations,
    ];
    pub fn wire(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Dms => "dms",
            Self::Mentions => "mentions",
            Self::Threads => "threads",
            Self::Reactions => "reactions",
            Self::Invitations => "invitations",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Dms => "DMs",
            Self::Mentions => "Mentions",
            Self::Threads => "Threads",
            Self::Reactions => "Reactions",
            Self::Invitations => "Invitations",
        }
    }
}

pub fn path(filter: Filter, unread: bool, cursor: Option<&str>) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query
        .append_pair("filter", filter.wire())
        .append_pair("unread", if unread { "true" } else { "false" });
    if let Some(cursor) = cursor {
        query.append_pair("cursor", cursor);
    }
    format!("api/notifications/inbox?{}", query.finish())
}

/// The inbox as every client keeps it: rows, paging, filters and optimistic
/// reads. Acknowledging a row never accepts an invitation.
pub struct Inbox {
    pub shown: bool,
    pub items: Vec<Item>,
    pub cursor: Option<String>,
    pub unread_count: u64,
    pub filter: Filter,
    pub unread_only: bool,
    pub loading: bool,
    /// The row whose open is in flight; only it is disabled, and a second open waits.
    pub opening: Option<String>,
    /// Optimistic mark-reads in flight, with each row's original position for undo.
    pub reads: Vec<(String, Option<usize>)>,
    /// After a filter change the previous rows stay, dimmed and inert, until the new
    /// page arrives: emptying the list first collapsed it and reset the scroll.
    pub stale: bool,
    pub error: Option<String>,
    pub request: u64,
    pub refreshed: Option<Instant>,
    pub expanded: HashMap<String, Item>,
    pub targets: HashMap<String, Item>,
    pub reacting: Option<Item>,
    pub reaction_request: u64,
    /// Each filter's loaded window, as web keeps them while the inbox is
    /// open: choosing that filter again shows it at once.
    windows: HashMap<(Filter, bool), Window>,
    /// First pages loading ahead for a hovered or focused filter, by request.
    prefetching: HashMap<u64, (Filter, bool)>,
    /// The prefetch the current filter waits for instead of its own load.
    adopted: Option<u64>,
    prefetched: u64,
}

/// A filter's rows and paging, and when they were loaded.
#[derive(Clone)]
struct Window {
    items: Vec<Item>,
    cursor: Option<String>,
    unread_count: u64,
    at: Instant,
}

/// How long a filter's window is shown without refreshing it, as the
/// open inbox's poll.
const FRESH: Duration = Duration::from_secs(15);
/// Prefetch requests count up from here, apart from the current filter's.
const PREFETCH: u64 = 1 << 63;

impl Default for Inbox {
    fn default() -> Self {
        Self {
            shown: false,
            items: Vec::new(),
            cursor: None,
            unread_count: 0,
            filter: Filter::All,
            unread_only: true,
            loading: false,
            opening: None,
            reads: Vec::new(),
            stale: false,
            error: None,
            request: 0,
            refreshed: None,
            expanded: HashMap::new(),
            targets: HashMap::new(),
            reacting: None,
            reaction_request: 0,
            windows: HashMap::new(),
            prefetching: HashMap::new(),
            adopted: None,
            prefetched: 0,
        }
    }
}
impl Inbox {
    pub fn visible_items(&self) -> Vec<Item> {
        let mut items = self.items.clone();
        let pinned = self
            .expanded
            .values()
            .filter(|item| !items.iter().any(|row| row.id == item.id))
            .cloned()
            .collect::<Vec<_>>();
        items.extend(pinned);
        items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        items
    }
    pub fn due(&self) -> bool {
        self.shown
            && !self.loading
            && self.reads.is_empty()
            && self.opening.is_none()
            && self.refreshed.is_none_or(|at| at.elapsed() >= FRESH)
    }
    fn key(&self) -> (Filter, bool) {
        (self.filter, self.unread_only)
    }
    /// After `filter` or `unread_only` changed: shows that filter's loaded
    /// window at once (`due` then refreshes it once it is 15 seconds old)
    /// or waits for its prefetch. Returns whether the client must load it,
    /// keeping the previous rows dimmed meanwhile.
    pub fn switched(&mut self) -> bool {
        let key = self.key();
        // Whatever the previous filter was waiting for no longer applies.
        self.request += 1;
        self.adopted = None;
        self.error = None;
        if let Some(window) = self.windows.get(&key).cloned() {
            self.items = window.items;
            self.cursor = window.cursor;
            self.unread_count = window.unread_count;
            self.refreshed = Some(window.at);
            self.loading = false;
            self.stale = false;
            return false;
        }
        if let Some((&request, _)) = self
            .prefetching
            .iter()
            .find(|(_, pending)| **pending == key)
        {
            self.adopted = Some(request);
            self.loading = true;
            self.stale = !self.items.is_empty();
            self.refreshed = Some(Instant::now());
            return false;
        }
        true
    }
    /// A first page for `filter` (with the current Unread only) to load ahead,
    /// when it has none loaded or loading and nothing is being marked read
    /// or opened. Returns its request; its page arrives through `finish`.
    pub fn prefetch(&mut self, filter: Filter) -> Option<u64> {
        let key = (filter, self.unread_only);
        if key == self.key()
            || !self.reads.is_empty()
            || self.opening.is_some()
            || self.windows.contains_key(&key)
            || self.prefetching.values().any(|pending| *pending == key)
        {
            return None;
        }
        self.prefetched += 1;
        let request = PREFETCH + self.prefetched;
        self.prefetching.insert(request, key);
        Some(request)
    }
    /// Reads change what every filter shows, so loaded windows and prefetches
    /// are dropped (the current rows stay).
    fn forget(&mut self) {
        self.windows.clear();
        let adopted = self.adopted;
        self.prefetching
            .retain(|request, _| Some(*request) == adopted);
    }
    /// The inbox closed: it starts afresh next time.
    pub fn close(&mut self) {
        self.shown = false;
        self.forget();
        self.prefetching.clear();
        self.adopted = None;
    }
    fn set_read(&mut self, id: &str) {
        for row in self.items.iter_mut().chain(self.expanded.values_mut()) {
            if row.id == id {
                row.unread = false;
            }
        }
        if self.unread_only {
            self.items.retain(|row| row.id != id);
        }
    }
    /// Marks an unread row read at once; `read_finished` keeps or reverts it.
    pub fn mark_read(&mut self, item: &Item) {
        if !item.unread {
            return;
        }
        self.forget();
        let index = self.items.iter().position(|row| row.id == item.id);
        self.reads.push((item.id.clone(), index));
        self.set_read(&item.id);
        self.unread_count = self.unread_count.saturating_sub(1);
    }
    /// `item` is the row as it was when acted on. Returns whether the read succeeded.
    pub fn read_finished(&mut self, item: &Item, result: Result<Read, String>) -> bool {
        self.forget();
        let pending = item
            .unread
            .then(|| self.reads.iter().position(|(id, _)| *id == item.id))
            .flatten()
            .map(|at| self.reads.remove(at).1);
        match result {
            Ok(read) => {
                self.set_read(&item.id);
                // With several reads in flight, only the last reply's count includes them all.
                if self.reads.is_empty() {
                    self.unread_count = read.unread_count;
                }
                true
            }
            Err(error) => {
                if let Some(index) = pending {
                    if let Some(row) = self.items.iter_mut().find(|row| row.id == item.id) {
                        row.unread = true;
                    } else if let Some(index) = index {
                        self.items.insert(index.min(self.items.len()), item.clone());
                    }
                    if let Some(row) = self.expanded.get_mut(&item.id) {
                        row.unread = true;
                    }
                    self.unread_count += 1;
                }
                self.error = Some(error);
                false
            }
        }
    }
    pub fn finish(&mut self, request: u64, more: bool, result: Result<Page, String>) {
        let mut request = request;
        if let Some(key) = self.prefetching.remove(&request) {
            if self.adopted != Some(request) {
                if let Ok(page) = result {
                    self.windows.insert(
                        key,
                        Window {
                            items: page.items,
                            cursor: page.next_cursor,
                            unread_count: page.unread_count,
                            at: Instant::now(),
                        },
                    );
                }
                return;
            }
            // The current filter waited for this page: it is its own.
            self.adopted = None;
            request = self.request;
        }
        if request != self.request {
            return;
        }
        self.loading = false;
        let stale = std::mem::take(&mut self.stale);
        match result {
            Ok(page) => {
                if !more {
                    self.items.clear();
                }
                for item in page.items {
                    if !self.items.iter().any(|existing| existing.id == item.id) {
                        self.items.push(item);
                    }
                }
                self.cursor = page.next_cursor;
                self.unread_count = page.unread_count;
                let window = Window {
                    items: self.items.clone(),
                    cursor: self.cursor.clone(),
                    unread_count: self.unread_count,
                    at: Instant::now(),
                };
                self.windows.insert(self.key(), window);
            }
            Err(error) => {
                // Rows kept from the previous filter must not stand in for this one.
                if stale {
                    self.items.clear();
                    self.cursor = None;
                }
                self.error = Some(error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_cursor_is_encoded_and_filters_are_explicit() {
        assert_eq!(
            path(Filter::Dms, true, Some("a+/=&?")),
            "api/notifications/inbox?filter=dms&unread=true&cursor=a%2B%2F%3D%26%3F"
        );
        assert_eq!(
            Filter::ALL.map(Filter::wire),
            [
                "all",
                "dms",
                "mentions",
                "threads",
                "reactions",
                "invitations"
            ]
        );
    }
    fn item(id: &str) -> Item {
        serde_json::from_value(serde_json::json!({
            "id": id, "kind": "message", "actorName": "Test", "body": "Preview",
            "createdAt": "2026-11-04T12:00:00Z", "unread": true,
            "conversationId": "dm0000000001", "messageId": "message00000001"
        }))
        .unwrap()
    }

    #[test]
    fn reply_targets_exclude_invitations_and_keep_drafting_rows_out_of_pagination() {
        let mut row = item("opaque:/id");
        assert!(row.can_reply());
        assert_eq!(row.quote_id(), row.message_id.as_deref());
        let key = row.draft_key();
        row.thread_root_id = Some("root".into());
        assert_ne!(key, row.draft_key());
        assert_eq!(row.quote_id(), None);
        assert_eq!(row.reply_label(), "Reply in thread");
        let mut inbox = Inbox::default();
        inbox.expanded.insert(row.id.clone(), row.clone());
        inbox.finish(
            0,
            false,
            Ok(Page {
                items: vec![],
                next_cursor: None,
                unread_count: 0,
            }),
        );
        assert!(
            inbox.items.is_empty(),
            "retained drafts must not extend pagination polling"
        );
        assert_eq!(inbox.visible_items()[0].id, row.id);
        for kind in ["direct.request", "space.invitation", "channel.invitation"] {
            row.kind = kind.into();
            assert!(!row.can_reply());
        }
    }

    #[test]
    fn pagination_keeps_refreshing_and_replaces_the_loaded_window() {
        let mut inbox = Inbox {
            shown: true,
            ..Default::default()
        };
        inbox.finish(
            0,
            false,
            Ok(Page {
                items: vec![item("removed"), item("kept")],
                next_cursor: Some("page2".into()),
                unread_count: 3,
            }),
        );
        inbox.finish(
            0,
            true,
            Ok(Page {
                items: vec![item("older")],
                next_cursor: Some("page3".into()),
                unread_count: 3,
            }),
        );
        assert!(
            inbox.due(),
            "older pages must not disable automatic updates"
        );
        inbox.finish(
            0,
            false,
            Ok(Page {
                items: vec![item("new"), item("kept"), item("displaced"), item("older")],
                next_cursor: Some("page3".into()),
                unread_count: 4,
            }),
        );
        assert_eq!(
            inbox
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["new", "kept", "displaced", "older"]
        );
        assert_eq!(inbox.cursor.as_deref(), Some("page3"));
        assert_eq!(inbox.unread_count, 4);
    }

    #[test]
    fn stale_pages_do_not_replace_current_filter_or_clear_loading() {
        // A filter change keeps the previous rows (dimmed) until its own page answers.
        let mut inbox = Inbox {
            request: 2,
            loading: true,
            stale: true,
            items: vec![item("previous")],
            cursor: Some("previous".into()),
            ..Default::default()
        };
        inbox.finish(
            1,
            false,
            Ok(Page {
                items: vec![],
                next_cursor: Some("old".into()),
                unread_count: 9,
            }),
        );
        assert!(inbox.loading && inbox.stale);
        assert_eq!(inbox.items[0].id, "previous");
        assert_eq!(inbox.cursor.as_deref(), Some("previous"));
        assert_eq!(inbox.unread_count, 0);
        inbox.finish(2, false, Err("offline".into()));
        assert!(!inbox.loading && !inbox.stale);
        assert!(inbox.items.is_empty() && inbox.cursor.is_none());
        assert_eq!(inbox.error.as_deref(), Some("offline"));
    }

    fn page(ids: &[&str], unread: u64) -> Result<Page, String> {
        Ok(Page {
            items: ids.iter().map(|id| item(id)).collect(),
            next_cursor: None,
            unread_count: unread,
        })
    }

    fn ids(inbox: &Inbox) -> Vec<&str> {
        inbox.items.iter().map(|item| item.id.as_str()).collect()
    }

    #[test]
    fn filters_keep_their_windows_and_prefetches_until_a_read() {
        let mut inbox = Inbox {
            shown: true,
            ..Default::default()
        };
        inbox.request = 1;
        inbox.finish(1, false, page(&["all"], 4));
        // Mentions has nothing loaded: the client loads it (rows stay dimmed).
        inbox.filter = Filter::Mentions;
        assert!(inbox.switched());
        inbox.request += 1;
        let abandoned = inbox.request;
        // Back to All before Mentions answers: shown at once, not reloaded,
        // and the late Mentions page doesn't replace it.
        inbox.filter = Filter::All;
        assert!(!inbox.switched());
        assert!(!inbox.loading && !inbox.stale && !inbox.due());
        inbox.finish(abandoned, false, page(&["mention"], 9));
        assert_eq!(ids(&inbox), ["all"]);
        assert_eq!(inbox.unread_count, 4);

        // A hovered tab loads ahead, once; its page waits in its window.
        let dms = inbox.prefetch(Filter::Dms).unwrap();
        assert_eq!(inbox.prefetch(Filter::Dms), None);
        assert_eq!(inbox.prefetch(Filter::All), None, "the current filter");
        inbox.finish(dms, false, page(&["dm"], 4));
        assert_eq!(ids(&inbox), ["all"], "a prefetch never replaces the rows");
        inbox.filter = Filter::Dms;
        assert!(!inbox.switched());
        assert_eq!(ids(&inbox), ["dm"]);

        // Choosing a tab whose prefetch is still loading waits for it.
        let threads = inbox.prefetch(Filter::Threads).unwrap();
        inbox.filter = Filter::Threads;
        assert!(!inbox.switched());
        assert!(inbox.loading && inbox.stale);
        inbox.finish(threads, false, page(&["thread"], 4));
        assert!(!inbox.loading && !inbox.stale);
        assert_eq!(ids(&inbox), ["thread"]);

        // A read changes every filter: windows are dropped, so All loads again.
        let thread = inbox.items[0].clone();
        inbox.mark_read(&thread);
        inbox.filter = Filter::All;
        assert!(inbox.switched());
    }

    #[test]
    fn an_old_window_shows_and_then_refreshes() {
        let mut inbox = Inbox {
            shown: true,
            ..Default::default()
        };
        inbox.finish(0, false, page(&["all"], 1));
        inbox.windows.get_mut(&(Filter::All, true)).unwrap().at -= FRESH;
        inbox.filter = Filter::Dms;
        assert!(inbox.switched());
        inbox.filter = Filter::All;
        assert!(!inbox.switched());
        assert_eq!(ids(&inbox), ["all"]);
        assert!(inbox.due(), "a 15-second-old window refreshes in place");
        inbox.close();
        inbox.shown = true;
        inbox.filter = Filter::Dms;
        inbox.switched();
        inbox.filter = Filter::All;
        assert!(inbox.switched(), "closing forgets the windows");
    }

    #[test]
    fn mark_read_is_optimistic_and_restores_the_row_and_count_on_failure() {
        let mut inbox = Inbox {
            shown: true,
            items: vec![item("a"), item("b"), item("c")],
            unread_count: 5,
            ..Default::default()
        };
        let (b, c) = (item("b"), item("c"));
        inbox.mark_read(&b);
        inbox.mark_read(&c);
        let ids = |inbox: &Inbox| {
            inbox
                .items
                .iter()
                .map(|row| row.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&inbox), ["a"]);
        assert_eq!(inbox.unread_count, 3);
        assert!(!inbox.due(), "polling waits for reads in flight");
        assert!(!inbox.read_finished(&b, Err("offline".into())));
        assert_eq!(ids(&inbox), ["a", "b"]);
        assert!(inbox.items[1].unread);
        assert_eq!(inbox.unread_count, 4);
        assert_eq!(inbox.error.as_deref(), Some("offline"));
        assert!(inbox.read_finished(&c, Ok(Read { unread_count: 2 })));
        assert!(inbox.reads.is_empty() && inbox.due());
        assert_eq!(
            inbox.unread_count, 2,
            "the last read takes the server count"
        );
    }
}
