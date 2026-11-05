//! Real conversations through the shared client core's background worker
//! (`caper_client::worker`): restores the stored session, opens the
//! account's first space and its first joined channel, follows it live
//! through the gateway, and sends messages. Signs in by email code when
//! no session is stored. Only used when `CAPER_API_URL` is set; the rest
//! of the egui client's commands come later.
use crate::model::{self, Conversation, Person, Reaction, Span};
use caper_client::worker::{Command, Event, Worker};
use caper_client::{Host, api::Api, gateway, grouping, model as core};
use std::collections::HashSet;
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
}

impl Ids {
    /// Where a new message goes: its author's index (one past the end for a
    /// new author, whom the caller adds) and whether it groups under the
    /// previous one. `None` when it is already shown.
    pub fn place(&mut self, m: &core::Message) -> Option<(usize, bool)> {
        if !self.seen.insert(m.id.clone()) {
            return None;
        }
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
    channel: Option<String>,
    chat_token: Option<String>,
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
        channel: None,
        chat_token: None,
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
                        Ok(s) => self.chat_token = Some(s.token),
                        Err(e) => out.push(Update::Failed(e)),
                    }
                    let channel = history.channel.id.clone();
                    self.channel = Some(channel.clone());
                    let messages: Vec<core::Message> = timeline.messages().cloned().collect();
                    let spaces = self.spaces.as_ref().map_or_else(
                        || core::Spaces {
                            spaces: Vec::new(),
                            invitations: Vec::new(),
                            limits: None,
                        },
                        Clone::clone,
                    );
                    let (conversation, ids) = conversation(&spaces, &detail, &channel, &messages);
                    out.push(Update::Loaded(Box::new((conversation, ids))));
                    self.worker.send(Command::Connect {
                        generation: current,
                        token: self.token.clone(),
                        channel,
                        cursor: timeline.cursor(),
                        presence: Some((detail.space.id.clone(), Vec::new())),
                        media: gateway::MediaWatch::default(),
                    });
                }
                Err(e) => out.push(Update::Failed(e.message)),
            },
            Event::Sent {
                generation, result, ..
            } if generation == current => match result {
                Ok(m) => out.push(Update::Message(Box::new(m))),
                Err(f) => out.push(Update::Failed(f.message)),
            },
            Event::Gateway(gateway::GatewayEvent::Message {
                generation,
                channel,
                message,
            }) if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                out.push(Update::Message(message))
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
    for m in messages.iter().filter(|m| m.deleted_at.is_none()) {
        let Some((author, grouped)) = ids.place(m) else {
            continue;
        };
        if author == members.len() {
            members.push(person(&m.author.id, &m.author.name));
        }
        out.push(message(m, author, grouped));
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

/// One displayed message. Its text is shown as written for now (Markdown
/// rendering, attachments and embeds are not drawn yet).
pub fn message(m: &core::Message, author: usize, grouped: bool) -> model::Message {
    model::Message {
        id: m.id.clone(),
        author,
        time: time_label(&m.created_at),
        grouped,
        spans: vec![Span::Text(m.content.text.clone())],
        picture: None,
        reactions: m
            .reactions
            .iter()
            .map(|r| Reaction {
                emoji: r.emoji.clone(),
                count: r.author_ids.len() as u32,
            })
            .collect(),
    }
}
