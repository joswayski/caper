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
    /// Where it happened. A direct message has no space, and its channel
    /// name is only the internal "direct".
    pub fn place(&self) -> String {
        match self.space_name.as_deref().filter(|name| !name.is_empty()) {
            None => "Direct message".into(),
            Some(space) => [Some(space), self.channel_name.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" \u{b7} "),
        }
    }
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

/// The most notifications one row (and one read request) carries.
pub const GROUP_MAX: usize = 100;

/// Notifications shown as one inbox row: neighbours in the same
/// conversation and thread (or reactions to one message) with the same
/// read state. Newest first, never empty.
#[derive(Clone, Debug)]
pub struct Group {
    pub items: Vec<Item>,
}

/// What a notification groups by; `None` never groups.
fn group_key(item: &Item) -> Option<(&str, &str, &str, bool)> {
    match item.kind.as_str() {
        "message" | "reply" | "mention" => Some((
            item.kind.as_str(),
            item.channel().unwrap_or_default(),
            item.thread_root_id.as_deref().unwrap_or_default(),
            item.unread,
        )),
        "reaction" => Some((
            "reaction",
            item.message_id.as_deref().unwrap_or_default(),
            "",
            item.unread,
        )),
        _ => None,
    }
}

/// Groups `items` (newest first) without reordering them: an item joins
/// the group right before it when both share a key and it has room.
pub fn group(items: Vec<Item>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for item in items {
        if let Some(last) = groups.last_mut()
            && last.items.len() < GROUP_MAX
            && group_key(&item).is_some_and(|key| group_key(last.newest()) == Some(key))
        {
            last.items.push(item);
        } else {
            groups.push(Group { items: vec![item] });
        }
    }
    groups
}

impl Group {
    pub fn newest(&self) -> &Item {
        &self.items[0]
    }
    /// Where opening the row lands, so the conversation starts at the run.
    pub fn oldest(&self) -> &Item {
        &self.items[self.items.len() - 1]
    }
    /// The row's stable identity: the newest item's ID.
    pub fn id(&self) -> &str {
        &self.newest().id
    }
    pub fn ids(&self) -> Vec<&str> {
        self.items.iter().map(|item| item.id.as_str()).collect()
    }
    pub fn unread(&self) -> bool {
        self.newest().unread
    }
    /// "A", "A and B", or "A and N others", newest actor first.
    pub fn names(&self) -> String {
        let mut names: Vec<&str> = Vec::new();
        for item in &self.items {
            if !names.contains(&item.actor_name.as_str()) {
                names.push(&item.actor_name);
            }
        }
        match names.len() {
            0 | 1 => self.newest().actor_name.clone(),
            2 => format!("{} and {}", names[0], names[1]),
            count => format!("{} and {} others", names[0], count - 1),
        }
    }
    /// What happened, in the place of a row's description.
    pub fn summary(&self) -> Option<String> {
        let item = self.newest();
        let count = self.items.len();
        if count == 1 {
            return match item.kind.as_str() {
                "reaction" => Some(format!(
                    "Reacted {} to your message",
                    item.emoji.as_deref().unwrap_or_default()
                )),
                "direct.request" => Some("Wants to send you messages".into()),
                "space.invitation" => Some("Invited you to a space".into()),
                "channel.invitation" => Some("Invited you to a private channel".into()),
                _ => None,
            };
        }
        match item.kind.as_str() {
            "message" => Some(format!("{count} messages")),
            "reply" => Some(format!("{count} replies")),
            "mention" => Some(format!("Mentioned you {count} times")),
            "reaction" => {
                let mut emojis: Vec<&str> = Vec::new();
                for emoji in self.items.iter().filter_map(|item| item.emoji.as_deref()) {
                    if !emoji.is_empty() && !emojis.contains(&emoji) {
                        emojis.push(emoji);
                    }
                }
                Some(format!("Reacted {} to your message", emojis.join(" ")))
            }
            _ => None,
        }
    }
    /// The mark-read control's accessible label.
    pub fn mark_label(&self) -> String {
        match self.items.len() {
            1 => format!("Mark notification from {} read", self.newest().actor_name),
            count => format!("Mark {count} notifications from {} read", self.names()),
        }
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
    /// The visible rows as the inbox shows them: runs of neighbours grouped.
    pub fn visible_groups(&self) -> Vec<Group> {
        group(self.visible_items())
    }
    /// Whether rows are loaded or on their way: opening the inbox then
    /// refreshes them in place instead of reloading behind them.
    pub fn ready(&self) -> bool {
        self.loading || !self.items.is_empty()
    }
    /// Unread notifications, as the inbox and its badge show them.
    pub fn unread_count(&self) -> u64 {
        self.unread_count
    }
    /// An unread snapshot's count. Returns whether the closed inbox should
    /// load ahead (as a refresh), so opening it shows these notifications at
    /// once rather than a loading state.
    pub fn count_changed(&mut self, count: u64) -> bool {
        let changed = count != self.unread_count;
        self.unread_count = count;
        changed && !self.shown
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
    /// Marks `items` (a row's group) read at once; `read_finished` keeps or
    /// reverts them. The count drops by the unread ones.
    pub fn mark_read(&mut self, items: &[Item]) {
        let unread = items.iter().filter(|item| item.unread).collect::<Vec<_>>();
        if unread.is_empty() {
            return;
        }
        self.forget();
        // Every position is taken before any row leaves, so a failure puts
        // them back where they were.
        for item in &unread {
            let index = self.items.iter().position(|row| row.id == item.id);
            self.reads.push((item.id.clone(), index));
        }
        for item in &unread {
            self.set_read(&item.id);
        }
        self.unread_count = self.unread_count.saturating_sub(unread.len() as u64);
    }
    /// `items` are the rows as they were when acted on, all sent in one read.
    /// Returns whether the read succeeded.
    pub fn read_finished(&mut self, items: &[Item], result: Result<Read, String>) -> bool {
        self.forget();
        let mut pending = items
            .iter()
            .filter(|item| item.unread)
            .filter_map(|item| {
                let at = self.reads.iter().position(|(id, _)| *id == item.id)?;
                Some((item, self.reads.remove(at).1))
            })
            .collect::<Vec<_>>();
        match result {
            Ok(read) => {
                for item in items {
                    self.set_read(&item.id);
                }
                // With several reads in flight, only the last reply's count includes them all.
                if self.reads.is_empty() {
                    self.unread_count = read.unread_count;
                }
                true
            }
            Err(error) => {
                // In list order, so each goes back to its own place.
                pending.sort_by_key(|(_, index)| *index);
                for (item, index) in &pending {
                    if let Some(row) = self.items.iter_mut().find(|row| row.id == item.id) {
                        row.unread = true;
                    } else if let Some(index) = index {
                        self.items
                            .insert((*index).min(self.items.len()), (*item).clone());
                    }
                    if let Some(row) = self.expanded.get_mut(&item.id) {
                        row.unread = true;
                    }
                }
                self.unread_count += pending.len() as u64;
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
        row.channel_name = Some("direct".into());
        assert_eq!(row.place(), "Direct message");
        row.space_name = Some("Studio".into());
        row.channel_name = Some("ideas".into());
        assert_eq!(row.place(), "Studio \u{b7} ideas");
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
        inbox.mark_read(&[thread]);
        inbox.filter = Filter::All;
        assert!(inbox.switched());
    }

    #[test]
    fn a_closed_inbox_loads_ahead_when_its_count_changes() {
        let mut inbox = Inbox::default();
        assert!(!inbox.ready());
        assert!(
            !inbox.count_changed(0),
            "an unchanged count asks for nothing"
        );
        assert!(inbox.count_changed(2));
        assert_eq!(inbox.unread_count, 2);
        assert!(!inbox.count_changed(2));
        inbox.finish(0, false, page(&["a", "b"], 2));
        assert!(inbox.ready(), "loaded rows open in place");
        inbox.shown = true;
        assert!(
            !inbox.count_changed(3),
            "an open inbox refreshes on its own schedule"
        );
        assert_eq!(inbox.unread_count, 3);
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
        inbox.mark_read(std::slice::from_ref(&b));
        inbox.mark_read(std::slice::from_ref(&c));
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
        assert!(!inbox.read_finished(std::slice::from_ref(&b), Err("offline".into())));
        assert_eq!(ids(&inbox), ["a", "b"]);
        assert!(inbox.items[1].unread);
        assert_eq!(inbox.unread_count, 4);
        assert_eq!(inbox.error.as_deref(), Some("offline"));
        assert!(inbox.read_finished(std::slice::from_ref(&c), Ok(Read { unread_count: 2 })));
        assert!(inbox.reads.is_empty() && inbox.due());
        assert_eq!(
            inbox.unread_count, 2,
            "the last read takes the server count"
        );
    }

    fn note(id: &str, kind: &str, actor: &str, extra: serde_json::Value) -> Item {
        let mut value = serde_json::json!({
            "id": id, "kind": kind, "actorName": actor, "body": id,
            "createdAt": "2026-11-04T12:00:00Z", "unread": true,
            "channelId": "channel00001", "messageId": format!("message-{id}")
        });
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(value).unwrap()
    }

    fn shape(groups: &[Group]) -> Vec<Vec<&str>> {
        groups.iter().map(Group::ids).collect()
    }

    #[test]
    fn only_neighbours_in_one_conversation_thread_and_read_state_group() {
        let none = serde_json::json!({});
        let thread = serde_json::json!({"threadRootId": "root1"});
        let other = serde_json::json!({"channelId": "channel00002"});
        let dm = serde_json::json!({"conversationId": "dm0000000001", "channelId": "channel00001"});
        let mut read = note("r", "message", "Ann", serde_json::json!({}));
        read.unread = false;
        let groups = group(vec![
            note("a", "message", "Ann", none.clone()),
            note("b", "message", "Bo", none.clone()),
            note("c", "message", "Ann", none.clone()),
            // A thread in the same channel is its own run.
            note("t1", "message", "Ann", thread.clone()),
            note("t2", "message", "Bo", thread),
            // Not adjacent to a..c: never merged back into them.
            note("d", "message", "Ann", none.clone()),
            note("o", "message", "Ann", other),
            note("e", "message", "Ann", none.clone()),
            read,
            note("f", "message", "Ann", none.clone()),
            // Same channel, different kind; then a DM keyed by its conversation.
            note("m", "mention", "Ann", none.clone()),
            note("x", "message", "Ann", dm.clone()),
            note("y", "message", "Ann", dm),
        ]);
        assert_eq!(
            shape(&groups),
            [
                vec!["a", "b", "c"],
                vec!["t1", "t2"],
                vec!["d"],
                vec!["o"],
                vec!["e"],
                vec!["r"],
                vec!["f"],
                vec!["m"],
                vec!["x", "y"],
            ]
        );
        let run = &groups[0];
        assert_eq!((run.id(), run.oldest().id.as_str()), ("a", "c"));
        assert_eq!(run.names(), "Ann and Bo");
        assert_eq!(run.summary().as_deref(), Some("3 messages"));
        assert_eq!(
            run.mark_label(),
            "Mark 3 notifications from Ann and Bo read"
        );
        assert_eq!(groups[2].summary(), None, "a single message reads as today");
        assert_eq!(groups[2].mark_label(), "Mark notification from Ann read");
    }

    #[test]
    fn reactions_group_by_message_and_invitations_never_group() {
        let on =
            |message: &str, emoji: &str| serde_json::json!({"messageId": message, "emoji": emoji});
        let groups = group(vec![
            note("a", "reaction", "Ann", on("m1", "🎉")),
            note("b", "reaction", "Bo", on("m1", "👍")),
            note("c", "reaction", "Cy", on("m1", "🎉")),
            note("d", "reaction", "Di", on("m1", "❤️")),
            note("e", "reaction", "Ann", on("m2", "👍")),
            note("i1", "space.invitation", "Ann", serde_json::json!({})),
            note("i2", "space.invitation", "Ann", serde_json::json!({})),
            note("q1", "direct.request", "Ann", serde_json::json!({})),
            note("q2", "direct.request", "Ann", serde_json::json!({})),
            note("r1", "reply", "Ann", serde_json::json!({})),
            note("r2", "reply", "Ann", serde_json::json!({})),
        ]);
        assert_eq!(
            shape(&groups),
            [
                vec!["a", "b", "c", "d"],
                vec!["e"],
                vec!["i1"],
                vec!["i2"],
                vec!["q1"],
                vec!["q2"],
                vec!["r1", "r2"],
            ]
        );
        assert_eq!(groups[0].names(), "Ann and 3 others");
        assert_eq!(
            groups[0].summary().as_deref(),
            Some("Reacted 🎉 👍 ❤️ to your message")
        );
        assert_eq!(
            groups[1].summary().as_deref(),
            Some("Reacted 👍 to your message")
        );
        assert_eq!(groups[6].names(), "Ann", "one distinct name");
        assert_eq!(groups[6].summary().as_deref(), Some("2 replies"));
    }

    #[test]
    fn a_group_holds_at_most_a_hundred_items() {
        let items = (0..GROUP_MAX + 1)
            .map(|k| note(&k.to_string(), "mention", "Ann", serde_json::json!({})))
            .collect();
        let groups = group(items);
        assert_eq!(
            groups.iter().map(|g| g.items.len()).collect::<Vec<_>>(),
            [100, 1]
        );
        assert_eq!(
            groups[0].summary().as_deref(),
            Some("Mentioned you 100 times")
        );
    }

    #[test]
    fn a_group_read_is_one_optimistic_read_and_a_failure_restores_every_row() {
        let none = serde_json::json!({});
        let rows = vec![
            note("x", "mention", "Ann", none.clone()),
            note("a", "message", "Ann", none.clone()),
            note("b", "message", "Bo", none.clone()),
            note("c", "message", "Cy", none.clone()),
            note("z", "mention", "Ann", none),
        ];
        let mut inbox = Inbox {
            shown: true,
            items: rows.clone(),
            unread_count: 9,
            ..Default::default()
        };
        let run = inbox.visible_groups().remove(1);
        assert_eq!(run.ids(), ["a", "b", "c"]);
        inbox.mark_read(&run.items);
        assert_eq!(ids(&inbox), ["x", "z"]);
        assert_eq!(inbox.unread_count, 6);
        assert_eq!(inbox.reads.len(), 3);
        assert!(!inbox.read_finished(&run.items, Err("offline".into())));
        assert_eq!(ids(&inbox), ["x", "a", "b", "c", "z"]);
        assert!(inbox.items.iter().all(|row| row.unread));
        assert_eq!(inbox.unread_count, 9);
        assert!(inbox.reads.is_empty());

        // With every row shown, they turn read in place and the server count wins.
        inbox.unread_only = false;
        inbox.mark_read(&run.items);
        assert_eq!(ids(&inbox), ["x", "a", "b", "c", "z"]);
        assert_eq!(inbox.unread_count, 6);
        assert!(inbox.read_finished(&run.items, Ok(Read { unread_count: 5 })));
        assert_eq!(inbox.unread_count, 5);
        assert_eq!(
            shape(&inbox.visible_groups()),
            [vec!["x"], vec!["a", "b", "c"], vec!["z"]],
            "read rows group apart from unread neighbours"
        );
    }
}
