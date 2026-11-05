//! Real conversations through the shared client core's background worker
//! (`caper_client::worker`): restores the stored session, opens the
//! account's first space and its first joined channel, follows it live
//! through the gateway, and sends messages. Signs in by email code when
//! no session is stored. Only used when `CAPER_API_URL` is set; the rest
//! of the egui client's commands come later.
use crate::model::{self, Conversation, Person, Reaction, Span};
use caper_client::worker::{Command, Event, Worker};
use caper_client::{Host, api::Api, gateway, grouping, model as core};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// Posted to the window when the worker has news; drained on the UI thread.
pub const WM_LIVE: u32 = WM_APP + 1;

/// What the worker asked of the window, applied on the UI thread (timers
/// and titles belong to the window's own thread).
#[derive(Default)]
pub struct Requests {
    pub wake_after: Option<Duration>,
    pub front: bool,
    pub title: Option<String>,
}

/// The window as the worker's host: every call posts `WM_LIVE`.
#[derive(Clone)]
pub struct WinHost {
    hwnd: isize,
    pub requests: Arc<Mutex<Requests>>,
}

impl WinHost {
    fn post(&self) {
        unsafe {
            let _ = PostMessageW(Some(HWND(self.hwnd as _)), WM_LIVE, WPARAM(0), LPARAM(0));
        }
    }

    fn request(&self, f: impl FnOnce(&mut Requests)) {
        if let Ok(mut r) = self.requests.lock() {
            f(&mut r);
        }
        self.post();
    }
}

impl Host for WinHost {
    fn wake(&self) {
        self.post();
    }

    fn wake_after(&self, delay: Duration) {
        self.request(|r| r.wake_after = Some(r.wake_after.map_or(delay, |d| d.min(delay))));
    }

    fn bring_to_front(&self) {
        self.request(|r| r.front = true);
    }

    fn set_title(&self, title: String) {
        self.request(|r| r.title = Some(title));
    }
}

pub enum Update {
    /// No stored session (or restoring it failed, with the reason): sign in.
    SignedOut(Option<String>),
    /// A sign-in code was emailed (the challenge), or why not.
    CodeRequested(Result<String, String>),
    /// The code was wrong, expired or used up, with attempts left.
    VerifyFailed(String, Option<u64>),
    /// Signed in: the account's spaces are loading.
    SignedIn,
    Loaded(Box<(Conversation, Ids)>),
    /// A new message, or the latest state of a shown one (its reactions,
    /// text or deletion changed), whose row is then rebuilt.
    Message(Box<core::Message>),
    /// Online account ids in the open space.
    Presence(HashSet<String>),
    /// The gateway's state while not live ("Connecting…"); None once live.
    Connection(Option<String>),
    Failed(String),
}

/// Server ids behind the displayed conversation.
#[derive(Clone, Default)]
pub struct Ids {
    /// Account id of each `Conversation::members` entry.
    pub members: Vec<String>,
    /// Id of each `Conversation::spaces` and `Conversation::channels` entry.
    pub spaces: Vec<String>,
    pub channels: Vec<String>,
    /// The newest row, for grouping the next message under it.
    last: Option<grouping::Row>,
    /// Messages shown, so a send's reply and its gateway echo appear once.
    seen: HashSet<String>,
    /// Each shown message's index in `Conversation::messages`.
    pub rows: HashMap<String, usize>,
}

impl Ids {
    /// Where a new message goes: its author's index (one past the end for a
    /// new author, whom the caller adds) and whether it groups under the
    /// previous one. `None` when it is already shown.
    pub fn place(&mut self, m: &core::Message) -> Option<(usize, bool)> {
        if !self.seen.insert(m.id.clone()) {
            return None;
        }
        self.rows.insert(m.id.clone(), self.rows.len());
        let author = self
            .members
            .iter()
            .position(|id| *id == m.author.id)
            .unwrap_or_else(|| {
                self.members.push(m.author.id.clone());
                self.members.len() - 1
            });
        let row = grouping::Row::new(&m.author.id, &m.created_at, false);
        let grouped = grouping::grouped(self.last.as_ref(), &row);
        self.last = Some(row);
        Some((author, grouped))
    }
}

pub struct Live {
    worker: Worker<WinHost>,
    pub host: WinHost,
    generation: u64,
    /// The latest navigation; results for earlier ones are dropped.
    navigation: u64,
    token: Option<String>,
    name: String,
    spaces: Option<core::Spaces>,
    space: Option<String>,
    channel: Option<String>,
    chat_token: Option<String>,
    /// The signed-in author's public id, for "your" reactions.
    pub me: Option<String>,
    /// The open channel's history as the core keeps it: gateway messages,
    /// reactions and edits apply here in sequence, then reach the rows.
    timeline: core::Timeline,
}

/// Starts the worker and restores the stored session; `None` without
/// `CAPER_API_URL`.
pub fn start(hwnd: HWND) -> Option<Live> {
    let url = std::env::var("CAPER_API_URL").ok()?;
    let host = WinHost {
        hwnd: hwnd.0 as isize,
        requests: Arc::default(),
    };
    let api = match Api::new(&url) {
        Ok(api) => api,
        Err(e) => {
            eprintln!("CAPER_API_URL: {e}");
            return None;
        }
    };
    let worker = Worker::new(api, host.clone());
    let generation = 1;
    worker.send(Command::Restore { generation });
    Some(Live {
        worker,
        host,
        generation,
        navigation: 0,
        token: None,
        name: String::new(),
        spaces: None,
        space: None,
        channel: None,
        chat_token: None,
        me: None,
        timeline: core::Timeline::default(),
    })
}

impl Live {
    pub fn request_code(&self, email: String) {
        self.worker.send(Command::RequestCode {
            generation: self.generation,
            email,
        });
    }

    pub fn verify(&self, challenge: String, code: String) {
        self.worker.send(Command::VerifyCode {
            generation: self.generation,
            challenge,
            code,
        });
    }

    /// Opens the account's first space; its first joined channel follows.
    fn open(
        &mut self,
        token: String,
        account: core::Account,
        spaces: core::Spaces,
        out: &mut Vec<Update>,
    ) {
        self.name = account
            .display_name
            .or(account.username)
            .unwrap_or_default();
        let Some(space) = spaces.spaces.first().map(|s| s.id.clone()) else {
            out.push(Update::Failed("This account has no spaces.".into()));
            return;
        };
        self.token = Some(token);
        self.spaces = Some(spaces);
        self.navigate(Some(space), None);
    }

    /// Opens a space (its first joined channel unless `channel` is given)
    /// or a channel. The open conversation stays until the new one loads.
    pub fn navigate(&mut self, space: Option<String>, channel: Option<String>) {
        self.navigation += 1;
        self.worker.send(Command::PrepareNavigation {
            generation: self.generation,
            navigation: self.navigation,
            token: self.token.clone(),
            space,
            channel,
            name: self.name.clone(),
            cached: None,
            visit: false,
            preferred: None,
        });
    }

    pub fn send(&self, text: String) {
        let (Some(channel), Some(chat_token)) = (&self.channel, &self.chat_token) else {
            return;
        };
        let client_id = uuid::Uuid::new_v4().to_string();
        self.worker.send(Command::Send {
            generation: self.generation,
            token: self.token.clone(),
            chat_token: chat_token.clone(),
            channel: channel.clone(),
            client_id,
            text,
            thread_root_id: None,
            broadcast: false,
            reply_to_id: None,
            attachment_ids: Vec::new(),
            spoiler_attachment_ids: Vec::new(),
            gif: None,
            suppress_embeds: false,
        });
    }

    /// The open channel's gateway position.
    fn seq(&self) -> u64 {
        self.timeline.cursor().parse().unwrap_or(0)
    }

    /// After a gateway update for message `id`: the messages it (and any
    /// buffered ones it released) made visible, past sequence `old`, and the
    /// message's own new state. A gap the core cannot bridge reloads the
    /// channel, as the egui client does.
    fn publish(
        &mut self,
        old: u64,
        applied: Result<core::Apply, String>,
        id: &str,
        out: &mut Vec<Update>,
    ) {
        match applied {
            Ok(core::Apply::Resync) | Err(_) => return self.reload(),
            Ok(core::Apply::Buffered) => return,
            Ok(core::Apply::Applied | core::Apply::Duplicate) => {}
        }
        for m in self.timeline.messages() {
            if m.seq.parse::<u64>().is_ok_and(|seq| seq > old) && m.id != id {
                out.push(Update::Message(Box::new(m.clone())));
            }
        }
        if let Some(m) = self.timeline.messages().find(|m| m.id == id) {
            out.push(Update::Message(Box::new(m.clone())));
        }
    }

    /// Reloads the open channel from history.
    fn reload(&mut self) {
        if let Some(channel) = self.channel.clone() {
            self.navigate(self.space.clone(), Some(channel));
        }
    }

    /// The visible conversation, so the worker suppresses its notifications.
    pub fn showing(&self, visible: bool) {
        self.worker
            .showing(self.channel.as_deref().filter(|_| visible));
    }

    /// Handles the worker's pending events; returns what the UI must show.
    pub fn drain(&mut self) -> Vec<Update> {
        let events: Vec<Event> = self.worker.events.try_iter().collect();
        let mut out = Vec::new();
        for event in events {
            self.handle(event, &mut out);
        }
        out
    }

    fn handle(&mut self, event: Event, out: &mut Vec<Update>) {
        let current = self.generation;
        match event {
            Event::Restored { generation, result } if generation == current => match result {
                Ok(Some((token, account, spaces))) => self.open(token, account, spaces, out),
                Ok(None) => out.push(Update::SignedOut(None)),
                Err(e) => out.push(Update::SignedOut(Some(e))),
            },
            Event::CodeRequested { generation, result } if generation == current => {
                out.push(Update::CodeRequested(result));
            }
            Event::Verified {
                generation,
                result,
                attempts_remaining,
            } if generation == current => match result {
                Ok((token, account, spaces)) => {
                    out.push(Update::SignedIn);
                    self.worker.send(Command::PersistCredential {
                        generation: current,
                        token: token.clone(),
                    });
                    self.open(token, account, spaces, out);
                }
                Err(e) => out.push(Update::VerifyFailed(e, attempts_remaining)),
            },
            Event::Credential {
                generation,
                result: Err(e),
            } if generation == current => {
                out.push(Update::Failed(format!("Could not save the session: {e}")));
            }
            Event::NavigationPrepared {
                generation,
                navigation,
                result,
            } if generation == current && navigation == self.navigation => match result {
                Ok(prepared) => {
                    // Events still in flight for the previous conversation
                    // (its gateway, sends) carry the old generation and drop.
                    self.generation += 1;
                    let current = self.generation;
                    let (Some(detail), Some((history, session))) =
                        (prepared.detail, prepared.conversation)
                    else {
                        out.push(Update::Failed("No joined channel in this space.".into()));
                        return;
                    };
                    let mut timeline = core::Timeline::default();
                    if let Err(e) = timeline.reset(history.messages, &history.cursor) {
                        out.push(Update::Failed(e));
                        return;
                    }
                    match session {
                        Ok(s) => {
                            self.chat_token = Some(s.token);
                            self.me = Some(s.author.id);
                        }
                        Err(e) => out.push(Update::Failed(e)),
                    }
                    let channel = history.channel.id.clone();
                    self.channel = Some(channel.clone());
                    self.space = Some(detail.space.id.clone());
                    let messages: Vec<core::Message> = timeline.messages().cloned().collect();
                    let cursor = timeline.cursor();
                    self.timeline = timeline;
                    let spaces = self.spaces.as_ref().map_or_else(
                        || core::Spaces {
                            spaces: Vec::new(),
                            invitations: Vec::new(),
                            limits: None,
                        },
                        Clone::clone,
                    );
                    let (conversation, ids) =
                        conversation(&spaces, &detail, &channel, &messages, self.me.as_deref());
                    out.push(Update::Loaded(Box::new((conversation, ids))));
                    self.worker.send(Command::Connect {
                        generation: current,
                        token: self.token.clone(),
                        channel,
                        cursor,
                        presence: Some((detail.space.id.clone(), Vec::new())),
                        media: gateway::MediaWatch::default(),
                    });
                }
                Err(e) => out.push(Update::Failed(e.message)),
            },
            Event::Sent {
                generation, result, ..
            } if generation == current => match result {
                Ok(m) => {
                    if self.timeline.merge_sent(m.clone()).is_err() {
                        self.reload();
                    }
                    out.push(Update::Message(Box::new(m)));
                }
                Err(f) => out.push(Update::Failed(f.message)),
            },
            Event::Gateway(gateway::GatewayEvent::Message {
                generation,
                channel,
                message,
            }) if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                let id = message.id.clone();
                let old = self.seq();
                let applied = self.timeline.apply(*message);
                self.publish(old, applied, &id, out);
            }
            Event::Gateway(gateway::GatewayEvent::Reactions {
                generation,
                channel,
                update,
            }) if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                let id = update.message_id.clone();
                let old = self.seq();
                let applied = self.timeline.apply_reactions(update);
                self.publish(old, applied, &id, out);
            }
            Event::Gateway(gateway::GatewayEvent::Edit {
                generation,
                channel,
                update,
            }) if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                let id = update.message.id.clone();
                let old = self.seq();
                let applied = self.timeline.apply_edit(*update);
                self.publish(old, applied, &id, out);
            }
            Event::Gateway(gateway::GatewayEvent::Presence {
                generation,
                members,
                ..
            }) if generation == current => {
                out.push(Update::Presence(
                    members
                        .into_iter()
                        .filter(|p| p.status != "offline")
                        .map(|p| p.user_id)
                        .collect(),
                ));
            }
            Event::Gateway(gateway::GatewayEvent::Status {
                generation,
                online,
                detail,
                ..
            }) if generation == current => out.push(Update::Connection(
                (!online && !detail.is_empty()).then_some(detail),
            )),
            // Everything else (reactions, edits, threads, voice, unread
            // state, notifications...) is not shown by this client yet.
            _ => {}
        }
    }
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

fn color_for(id: &str) -> u32 {
    let h = id
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    model::AUTHOR_COLORS[h as usize % model::AUTHOR_COLORS.len()]
}

pub fn person(id: &str, name: &str) -> Person {
    Person {
        name: name.to_owned(),
        initials: initials(name),
        color: color_for(id),
        online: false,
    }
}

fn time_label(created_at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(created_at)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%-I:%M %p")
                .to_string()
        })
        .unwrap_or_default()
}

/// The displayed conversation for one channel's history.
pub fn conversation(
    spaces: &core::Spaces,
    detail: &core::SpaceDetail,
    channel: &str,
    messages: &[core::Message],
    me: Option<&str>,
) -> (Conversation, Ids) {
    let order = detail.member_order(Some(channel));
    let mut ids = Ids {
        members: order.iter().map(|m| m.id.clone()).collect(),
        spaces: spaces.spaces.iter().map(|s| s.id.clone()).collect(),
        ..Ids::default()
    };
    let mut members: Vec<Person> = order
        .iter()
        .map(|m| person(&m.id, &m.display_name))
        .collect();
    let listed = members.len();
    let mut out = Vec::with_capacity(messages.len());
    for m in messages {
        let Some((author, grouped)) = ids.place(m) else {
            continue;
        };
        if author == members.len() {
            members.push(person(&m.author.id, &m.author.name));
        }
        out.push(message(m, author, grouped, me));
    }
    let joined: Vec<&core::Channel> = detail.channels.iter().filter(|c| c.joined).collect();
    ids.channels = joined.iter().map(|c| c.id.clone()).collect();
    let channels = joined
        .iter()
        .map(|c| model::Channel {
            name: c.name.clone(),
            private: c.private,
        })
        .collect();
    let selected = joined.iter().position(|c| c.id == channel).unwrap_or(0);
    let space = spaces
        .spaces
        .iter()
        .position(|s| s.id == detail.space.id)
        .unwrap_or(0);
    let spaces = spaces
        .spaces
        .iter()
        .map(|s| model::Space {
            name: s.name.clone(),
        })
        .collect();
    let mut c = Conversation::live(spaces, space, channels, selected, members, out);
    c.listed_members = listed;
    (c, ids)
}

/// Message text as styled runs, as the egui client draws it: the shared
/// bold/italic grammar (`shared/messages/formatting.rs`) removes its markers
/// first; then, in the text that remains, the mentions the server resolved
/// (`caper_client::mentions`) become pills and links
/// (`shared/messages/links.rs`) are found between them.
pub fn spans(text: &str, mentions: &[core::Mention]) -> Vec<Span> {
    let mut visible = String::with_capacity(text.len());
    let mut styles = Vec::new();
    crate::formatting::visit_runs(text, |range, bold, italic| {
        let start = visible.len();
        visible.push_str(&text[range]);
        styles.push((start..visible.len(), bold, italic));
    });
    let pills: Vec<_> = caper_client::mentions::highlights(&visible, mentions)
        .into_iter()
        .map(|(range, _)| range)
        .collect();
    let mut links = Vec::new();
    let mut gap = 0;
    for (start, end) in pills
        .iter()
        .map(|p| (p.start, p.end))
        .chain([(visible.len(), visible.len())])
    {
        let mut offset = gap;
        for segment in crate::links::segments(&visible[gap..start]) {
            let next = offset + segment.text.len();
            if let Some(href) = segment.href {
                links.push((offset..next, href));
            }
            offset = next;
        }
        gap = end;
    }
    let mut out = Vec::new();
    for (range, bold, italic) in styles {
        let mut cuts = vec![range.start, range.end];
        for span in pills.iter().chain(links.iter().map(|(link, _)| link)) {
            for edge in [span.start, span.end] {
                if range.start < edge && edge < range.end {
                    cuts.push(edge);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for piece in cuts.windows(2) {
            let (start, end) = (piece[0], piece[1]);
            out.push(Span::Rich {
                text: visible[start..end].to_owned(),
                bold,
                italic,
                mention: pills.iter().any(|p| p.contains(&start)),
                href: links
                    .iter()
                    .find(|(link, _)| link.contains(&start))
                    .map(|(_, href)| href.clone()),
            });
        }
    }
    out
}

/// One displayed message (attachments and embeds are not drawn yet). `me`
/// is the signed-in author, whose reactions are highlighted.
pub fn message(
    m: &core::Message,
    author: usize,
    grouped: bool,
    me: Option<&str>,
) -> model::Message {
    let deleted = m.is_deleted();
    // As in the egui client: a deleted message keeps its place as a muted
    // notice without reactions; an edit marks the time, or follows the text
    // of a grouped row.
    let edited = m.revision > 1 && !deleted;
    let mut time = time_label(&m.created_at);
    if edited && !grouped {
        time.push_str(" (edited)");
    }
    model::Message {
        id: m.id.clone(),
        author,
        time,
        grouped,
        spans: if deleted {
            vec![Span::Rich {
                text: "This message was deleted.".into(),
                bold: false,
                italic: true,
                mention: false,
                href: None,
            }]
        } else {
            spans(&m.content.text, &m.content.mentions)
        },
        picture: None,
        reactions: if deleted {
            Vec::new()
        } else {
            m.reactions
                .iter()
                .filter(|r| !r.author_ids.is_empty())
                .map(|r| Reaction {
                    emoji: r.emoji.clone(),
                    count: r.author_ids.len() as u32,
                    mine: me.is_some_and(|me| r.author_ids.iter().any(|id| id == me)),
                })
                .collect()
        },
        edited_inline: edited && grouped,
        deleted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_message(extra: serde_json::Value) -> core::Message {
        let mut m = serde_json::json!({
            "id": "b00000000000001",
            "channelId": "general",
            "seq": "1",
            "author": { "id": "member000002", "name": "Alex", "isGuest": false },
            "content": { "version": 1, "type": "text", "text": "hello **there**" },
            "clientMessageId": "00000000-0000-4000-9000-000000000001",
            "createdAt": "2026-10-01T09:00:00Z",
        });
        for (k, v) in extra.as_object().unwrap() {
            m[k] = v.clone();
        }
        serde_json::from_value(m).unwrap()
    }

    #[test]
    fn rows_show_your_reactions_edits_and_deletions_as_the_egui_client_does() {
        let reacted = core_message(serde_json::json!({ "reactions": [
            { "emoji": "👍", "authorIds": ["member000002", "owner0000001"] },
            { "emoji": "🎉", "authorIds": ["member000002"] },
        ]}));
        let row = message(&reacted, 0, false, Some("owner0000001"));
        let chips: Vec<_> = row
            .reactions
            .iter()
            .map(|r| (r.emoji.as_str(), r.count, r.mine))
            .collect();
        assert_eq!(chips, [("👍", 2, true), ("🎉", 1, false)]);
        assert!(!row.time.contains("edited") && !row.edited_inline && !row.deleted);
        assert!(
            row != message(&reacted, 0, false, Some("member000003")),
            "mine changes the row"
        );

        let edited = core_message(serde_json::json!({
            "revision": 2, "editedAt": "2026-10-01T09:05:00Z", "editSeq": "2",
        }));
        assert!(message(&edited, 0, false, None).time.ends_with(" (edited)"));
        let grouped = message(&edited, 0, true, None);
        assert!(grouped.edited_inline && !grouped.time.contains("edited"));

        let deleted = core_message(serde_json::json!({
            "revision": 2, "editedAt": "2026-10-01T09:05:00Z", "editSeq": "2",
            "deletedAt": "2026-10-01T09:05:00Z",
            "content": { "version": 1, "type": "text", "text": "" },
            "reactions": [{ "emoji": "👍", "authorIds": ["member000002"] }],
        }));
        let row = message(&deleted, 0, false, None);
        assert!(row.deleted && row.reactions.is_empty() && !row.time.contains("edited"));
        assert!(
            matches!(&row.spans[..], [Span::Rich { text, italic: true, .. }] if text == "This message was deleted.")
        );
    }
    fn shown(spans: &[Span]) -> Vec<(String, bool, bool, Option<String>)> {
        spans
            .iter()
            .map(|s| match s {
                Span::Rich {
                    text,
                    bold,
                    italic,
                    href,
                    ..
                } => (text.clone(), *bold, *italic, href.clone()),
                _ => panic!("live messages are rich runs"),
            })
            .collect()
    }

    #[test]
    fn styles_and_links_split_into_runs_without_their_markers() {
        let runs = shown(&spans("**see https://caper.chat/x** and _this_", &[]));
        let text: String = runs.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(text, "see https://caper.chat/x and this");
        let link = runs.iter().find(|r| r.3.is_some()).expect("a link run");
        assert_eq!(link.0, "https://caper.chat/x");
        assert!(link.1, "the link keeps the bold around it");
        assert!(runs.iter().any(|r| r.0 == "this" && r.2 && !r.1));
        assert!(runs.iter().any(|r| r.0 == "see " && r.1 && r.3.is_none()));
    }

    #[test]
    fn resolved_mentions_become_pills_and_unresolved_ones_stay_text() {
        let maya = core::Mention {
            kind: "user".into(),
            id: Some("member000001".into()),
            username: Some("maya".into()),
        };
        let text = "hi @maya and @nobody, see **@maya's** https://x.example";
        let pills: Vec<String> = spans(text, std::slice::from_ref(&maya))
            .into_iter()
            .filter_map(|s| match s {
                Span::Rich {
                    text,
                    mention: true,
                    ..
                } => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(pills, ["@maya", "@maya"]);
    }
}
