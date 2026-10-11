//! `@feedback` in place: the Caper team's inbox (`GET /api/feedback`) and
//! visits, which open a conversation where someone asked for feedback without
//! joining it. The team reads it for context and replies there as themselves.

use crate::model::History;
use chrono::{DateTime, Datelike, Local, Utc};
use serde::Deserialize;

/// Who wrote a report.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Reporter {
    pub id: String,
    pub username: String,
    pub display_name: String,
    #[serde(default)]
    pub avatar_id: Option<i32>,
}

/// Where a report was written. A DM is named after both people in it ("Alice
/// & Bob"), or one for personal notes.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ReportChannel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub direct: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ReportSpace {
    pub id: String,
    pub name: String,
}

/// One `@feedback` message, newest first in the inbox.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub id: String,
    /// The message that said `@feedback`.
    pub message_id: String,
    /// Where replies go: the message itself when it was top-level.
    pub thread_root_id: String,
    pub created_at: String,
    /// At most 280 characters; may be empty for a files-only message.
    #[serde(default)]
    pub text: String,
    /// The message's file count.
    #[serde(default)]
    pub attachments: u32,
    pub reporter: Reporter,
    pub channel: ReportChannel,
    /// `null` for DMs.
    #[serde(default)]
    pub space: Option<ReportSpace>,
    #[serde(default)]
    pub reply_count: u64,
    /// Someone on the team replied in its thread after the report.
    #[serde(default)]
    pub answered: bool,
}

/// `GET /api/feedback[?before=<cursor>]`: 50 reports and the next cursor.
#[derive(Clone, Debug, Deserialize)]
pub struct Page {
    pub reports: Vec<Report>,
    #[serde(default)]
    pub cursor: Option<String>,
}

impl Page {
    /// Every report opens a conversation by id; a cursor is a number.
    pub fn valid(&self) -> bool {
        self.reports.iter().all(|report| {
            !report.id.is_empty()
                && !report.message_id.is_empty()
                && !report.thread_root_id.is_empty()
                && !report.channel.id.is_empty()
        }) && self.cursor.as_deref().is_none_or(|cursor| {
            !cursor.is_empty() && cursor.bytes().all(|byte| byte.is_ascii_digit())
        })
    }
}

impl Report {
    /// "#general · Studio", or "DM · Alice & Bob".
    pub fn place(&self) -> String {
        match &self.space {
            Some(space) if !self.channel.direct => {
                format!("#{} · {}", self.channel.name, space.name)
            }
            _ => format!("DM · {}", self.channel.name),
        }
    }

    /// The text, or "Sent N files" for a files-only message.
    pub fn preview(&self) -> String {
        let text = self.text.split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.is_empty() {
            return text;
        }
        match self.attachments {
            0 => String::new(),
            1 => "Sent 1 file".into(),
            count => format!("Sent {count} files"),
        }
    }

    /// "Answered", or "Waiting" (drawn in terracotta).
    pub fn status(&self) -> &'static str {
        if self.answered { "Answered" } else { "Waiting" }
    }

    /// Opens at the report, in its thread, so replies land there by default.
    pub fn target(&self) -> VisitTarget {
        VisitTarget {
            channel: self.channel.id.clone(),
            message: self.message_id.clone(),
            thread: Some(self.thread_root_id.clone()),
            reporter: Some(self.reporter.clone()),
        }
    }
}

/// A compact age: "now", "5m", "3h", "2d", then the date ("Oct 3", with the
/// year once it differs).
pub fn relative_time(created_at: &str, now: DateTime<Utc>) -> String {
    let Ok(date) = DateTime::parse_from_rfc3339(created_at) else {
        return String::new();
    };
    let date = date.with_timezone(&Utc);
    let seconds = (now - date).num_seconds().max(0);
    match seconds {
        0..60 => "now".into(),
        60..3_600 => format!("{}m", seconds / 60),
        3_600..86_400 => format!("{}h", seconds / 3_600),
        86_400..604_800 => format!("{}d", seconds / 86_400),
        _ => {
            let local = date.with_timezone(&Local);
            if local.year() == now.with_timezone(&Local).year() {
                local.format("%b %-d").to_string()
            } else {
                local.format("%b %-d, %Y").to_string()
            }
        }
    }
}

/// The inbox's reports, paging and load state.
#[derive(Default)]
pub struct Inbox {
    pub reports: Vec<Report>,
    /// The next page's `before`, while there is one.
    pub cursor: Option<String>,
    /// A first page has arrived since sign-in.
    pub loaded: bool,
    /// A refresh (first page) is in flight.
    pub loading: bool,
    /// An older page is in flight.
    pub loading_more: bool,
    pub error: Option<String>,
    request: u64,
}

impl Inbox {
    /// Starts a refresh (`more` false), which replaces the list and overtakes
    /// an older page in flight, or the next page. Returns the request and
    /// its `before`, or `None` when there is nothing to start.
    pub fn start(&mut self, more: bool) -> Option<(u64, Option<String>)> {
        let before = if more {
            if self.loading || self.loading_more {
                return None;
            }
            Some(self.cursor.clone()?)
        } else {
            None
        };
        self.request += 1;
        self.loading = !more;
        self.loading_more = more;
        self.error = None;
        Some((self.request, before))
    }

    /// Takes a page for the latest request; older answers are dropped. A
    /// failure keeps the reports already shown.
    pub fn finish(&mut self, request: u64, result: Result<Page, String>) {
        if request != self.request {
            return;
        }
        let more = self.loading_more;
        self.loading = false;
        self.loading_more = false;
        match result {
            Ok(page) if page.valid() => {
                if more {
                    for report in page.reports {
                        if !self.reports.iter().any(|known| known.id == report.id) {
                            self.reports.push(report);
                        }
                    }
                } else {
                    self.reports = page.reports;
                }
                self.cursor = page.cursor;
                self.loaded = true;
            }
            Ok(_) => self.error = Some("Caper returned an invalid feedback list.".into()),
            Err(error) => self.error = Some(error),
        }
    }
}

/// What to open: a conversation by channel id, at `message`, and the thread
/// replies should go to (if any).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisitTarget {
    pub channel: String,
    pub message: String,
    pub thread: Option<String>,
    /// From the inbox: someone to suggest for `@` without a member list.
    pub reporter: Option<Reporter>,
}

/// After the conversation's history around `anchor` lands: the thread to
/// open, at `anchor` when the report was a reply in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadFocus {
    pub root: String,
    pub anchor: Option<String>,
}

impl VisitTarget {
    /// The main history's anchor, then the thread. A top-level report opens
    /// around itself and then its own thread; a reply opens around its root
    /// and then the thread around the reply.
    pub fn plan(&self) -> (String, Option<ThreadFocus>) {
        match &self.thread {
            Some(root) if *root != self.message => (
                root.clone(),
                Some(ThreadFocus {
                    root: root.clone(),
                    anchor: Some(self.message.clone()),
                }),
            ),
            Some(root) => (
                self.message.clone(),
                Some(ThreadFocus {
                    root: root.clone(),
                    anchor: None,
                }),
            ),
            None => (self.message.clone(), None),
        }
    }
}

/// Where Back returns from a visit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Inbox,
    /// The conversation open before a notification opened the visit.
    Conversation {
        space: Option<String>,
        channel: String,
    },
    Nowhere,
}

/// A visit that is opening: consumed by the navigation for its channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisitRequest {
    pub target: VisitTarget,
    pub origin: Origin,
}

/// The open conversation, read as the Caper team (`visitor: true`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Visit {
    pub channel: String,
    /// "plans", or "Alice & Bob" for a DM.
    pub name: String,
    pub direct: bool,
    /// "#plans · Studio", or "Alice & Bob".
    pub title: String,
    pub reporter: Option<Reporter>,
    pub origin: Origin,
}

impl Visit {
    pub fn new(request: VisitRequest, history: &History) -> Self {
        let direct = history.space.id.is_empty();
        let name = history.channel.name.clone();
        Self {
            channel: history.channel.id.clone(),
            title: if direct {
                name.clone()
            } else {
                format!("#{name} · {}", history.space.name)
            },
            name,
            direct,
            reporter: request.target.reporter,
            origin: request.origin,
        }
    }
}

/// The slim line over a visited conversation.
pub const BANNER: &str = "Visiting as the Caper team — everyone here sees your replies.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HistoryPlace;

    fn page() -> Page {
        serde_json::from_value(serde_json::json!({
            "reports": [{
                "id": "msg000000002", "messageId": "msg000000002",
                "threadRootId": "msg000000001",
                "createdAt": "2026-10-11T19:00:00.123456+00:00",
                "text": "@feedback uploads\n  fail", "attachments": 2,
                "reporter": {"id": "user00000001", "username": "alice",
                             "displayName": "Alice", "avatarId": 12},
                "channel": {"id": "chan00000001", "name": "general", "direct": false},
                "space": {"id": "space0000001", "name": "Studio"},
                "replyCount": 3, "answered": true
            }, {
                "id": "msg000000003", "messageId": "msg000000003",
                "threadRootId": "msg000000003", "createdAt": "2026-10-11T18:00:00Z",
                "text": "", "attachments": 1,
                "reporter": {"id": "user00000002", "username": "bob", "displayName": "Bob"},
                "channel": {"id": "dm0000000001", "name": "Alice & Bob", "direct": true},
                "space": null, "replyCount": 0, "answered": false
            }],
            "cursor": "123"
        }))
        .unwrap()
    }

    #[test]
    fn reports_decode_and_describe_where_and_what() {
        let page = page();
        assert!(page.valid());
        assert_eq!(page.cursor.as_deref(), Some("123"));
        let [channel, direct] = &page.reports[..] else {
            panic!("two reports");
        };
        assert_eq!(channel.place(), "#general · Studio");
        assert_eq!(channel.preview(), "@feedback uploads fail");
        assert_eq!(channel.status(), "Answered");
        assert_eq!(channel.reporter.avatar_id, Some(12));
        assert_eq!((channel.reply_count, channel.attachments), (3, 2));
        assert_eq!(direct.place(), "DM · Alice & Bob");
        assert_eq!(direct.preview(), "Sent 1 file");
        assert_eq!(direct.status(), "Waiting");
        assert_eq!(direct.space, None);
        assert_eq!(direct.reporter.avatar_id, None);
        let mut files = direct.clone();
        files.attachments = 3;
        assert_eq!(files.preview(), "Sent 3 files");
        assert_eq!(
            channel.target(),
            VisitTarget {
                channel: "chan00000001".into(),
                message: "msg000000002".into(),
                thread: Some("msg000000001".into()),
                reporter: Some(channel.reporter.clone()),
            }
        );
        let empty: Page =
            serde_json::from_value(serde_json::json!({"reports": [], "cursor": null})).unwrap();
        assert!(empty.valid() && empty.cursor.is_none());
        let mut bad = page.clone();
        bad.cursor = Some("12a".into());
        assert!(!bad.valid());
        let mut bad = page;
        bad.reports[0].channel.id.clear();
        assert!(!bad.valid());
    }

    #[test]
    fn plans_open_the_report_and_then_its_thread() {
        let target = |message: &str, thread: Option<&str>| VisitTarget {
            channel: "c".into(),
            message: message.into(),
            thread: thread.map(Into::into),
            reporter: None,
        };
        assert_eq!(
            target("m1", Some("m1")).plan(),
            (
                "m1".into(),
                Some(ThreadFocus {
                    root: "m1".into(),
                    anchor: None
                })
            ),
            "a top-level report opens around itself, then its own thread"
        );
        assert_eq!(
            target("m2", Some("m1")).plan(),
            (
                "m1".into(),
                Some(ThreadFocus {
                    root: "m1".into(),
                    anchor: Some("m2".into())
                })
            ),
            "a reply opens around its root, then the thread around the reply"
        );
        assert_eq!(target("m3", None).plan(), ("m3".into(), None));
    }

    #[test]
    fn inbox_pages_refreshes_and_drops_stale_answers() {
        let mut inbox = Inbox::default();
        assert_eq!(inbox.start(true), None, "no cursor yet");
        let (first, before) = inbox.start(false).unwrap();
        assert_eq!(before, None);
        assert!(inbox.loading);
        inbox.finish(first, Ok(page()));
        assert!(inbox.loaded && !inbox.loading);
        assert_eq!(inbox.reports.len(), 2);
        let (more, before) = inbox.start(true).unwrap();
        assert_eq!(before.as_deref(), Some("123"));
        assert_eq!(inbox.start(true), None, "one page at a time");
        // A refresh overtakes the older page in flight.
        let (refresh, _) = inbox.start(false).unwrap();
        inbox.finish(more, Ok(page()));
        assert!(inbox.loading, "the stale page is dropped");
        let mut fresh = page();
        fresh.reports.truncate(1);
        fresh.cursor = None;
        inbox.finish(refresh, Ok(fresh));
        assert_eq!(inbox.reports.len(), 1);
        assert_eq!(inbox.cursor, None);
        assert_eq!(inbox.start(true), None, "no more pages");
        let (failed, _) = inbox.start(false).unwrap();
        inbox.finish(failed, Err("offline".into()));
        assert_eq!(inbox.reports.len(), 1, "a failure keeps the list");
        assert_eq!(inbox.error.as_deref(), Some("offline"));
        // Appending skips reports already listed.
        inbox.cursor = Some("9".into());
        let (more, _) = inbox.start(true).unwrap();
        inbox.finish(more, Ok(page()));
        assert_eq!(
            inbox
                .reports
                .iter()
                .map(|report| report.id.as_str())
                .collect::<Vec<_>>(),
            ["msg000000002", "msg000000003"]
        );
    }

    #[test]
    fn relative_times_are_compact() {
        let now = DateTime::parse_from_rfc3339("2026-10-11T19:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        for (at, label) in [
            ("2026-10-11T18:59:30Z", "now"),
            ("2026-10-11T19:00:30Z", "now"),
            ("2026-10-11T18:55:00Z", "5m"),
            ("2026-10-11T16:00:00+00:00", "3h"),
            ("2026-10-09T19:00:00Z", "2d"),
        ] {
            assert_eq!(relative_time(at, now), label, "{at}");
        }
        assert!(!relative_time("2026-09-01T12:00:00Z", now).is_empty());
        assert!(relative_time("2024-09-01T12:00:00Z", now).ends_with("2024"));
        assert_eq!(relative_time("yesterday", now), "");
    }

    #[test]
    fn visits_are_titled_like_their_conversation() {
        let history = |space: &str, space_name: &str, name: &str| History {
            messages: vec![],
            pinned_messages: vec![],
            cursor: "1".into(),
            has_more: false,
            space: HistoryPlace {
                id: space.into(),
                name: space_name.into(),
            },
            channel: HistoryPlace {
                id: "chan00000001".into(),
                name: name.into(),
            },
            visitor: true,
        };
        let request = VisitRequest {
            target: page().reports[0].target(),
            origin: Origin::Inbox,
        };
        let channel = Visit::new(request.clone(), &history("space0000001", "Studio", "plans"));
        assert_eq!(
            (
                channel.title.as_str(),
                channel.name.as_str(),
                channel.direct
            ),
            ("#plans · Studio", "plans", false)
        );
        assert_eq!(channel.origin, Origin::Inbox);
        assert_eq!(channel.reporter.unwrap().username, "alice");
        let direct = Visit::new(request, &history("", "Direct messages", "Alice & Bob"));
        assert_eq!(
            (direct.title.as_str(), direct.name.as_str(), direct.direct),
            ("Alice & Bob", "Alice & Bob", true)
        );
        let decoded: History = serde_json::from_value(serde_json::json!({
            "messages": [], "pinnedMessages": [], "cursor": "4", "hasMore": false,
            "space": {"id": "", "name": "Direct messages"},
            "channel": {"id": "dm0000000001", "name": "Alice & Bob", "direct": true},
            "visitor": true
        }))
        .unwrap();
        assert!(decoded.visitor);
        let member: History = serde_json::from_value(serde_json::json!({
            "messages": [], "cursor": "4", "hasMore": false,
            "space": {"id": "s", "name": "Studio"}, "channel": {"id": "c", "name": "plans"}
        }))
        .unwrap();
        assert!(!member.visitor, "members never get the flag");
    }
}
