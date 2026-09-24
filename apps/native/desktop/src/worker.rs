use crate::api::Api;
use crate::gateway::{self, GatewayEvent};
use crate::model::{
    Account, Channel, ChatSession, History, Member, Message, Space, SpaceDetail, Spaces,
};
use eframe::egui;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

pub enum Command {
    Restore {
        generation: u64,
    },
    RequestCode {
        generation: u64,
        email: String,
    },
    VerifyCode {
        generation: u64,
        challenge: String,
        code: String,
    },
    Profile {
        generation: u64,
        token: String,
        username: String,
        display_name: String,
    },
    PrepareNavigation {
        generation: u64,
        navigation: u64,
        token: Option<String>,
        space: Option<String>,
        channel: Option<String>,
        name: String,
        cached: Option<History>,
    },
    PrefetchNavigation {
        generation: u64,
        request: u64,
        space: Option<String>,
        channel: Option<String>,
        token: Option<String>,
    },
    LoadChannel {
        generation: u64,
        token: Option<String>,
        channel: String,
        name: String,
        general: bool,
    },
    LoadOlder {
        generation: u64,
        token: Option<String>,
        channel: String,
        before: String,
    },
    Connect {
        generation: u64,
        token: Option<String>,
        channel: String,
        cursor: String,
        presence: Option<(String, Vec<String>)>,
    },
    Send {
        generation: u64,
        token: Option<String>,
        chat_token: String,
        channel: String,
        client_id: String,
        text: String,
    },
    Typing {
        token: Option<String>,
        chat_token: String,
        channel: String,
        typing: bool,
    },
    Admin {
        generation: u64,
        token: String,
        operation: AdminOperation,
    },
    Logout {
        generation: u64,
        token: String,
    },
    PersistCredential {
        generation: u64,
        token: String,
    },
    ClearCredential {
        generation: u64,
    },
    Activity,
    StopGateway,
}

#[derive(Clone)]
pub enum AdminOperation {
    CreateSpace {
        name: String,
    },
    UpdateSpace {
        space: String,
        name: String,
    },
    DeleteSpace {
        space: String,
    },
    LeaveSpace {
        space: String,
        member: String,
    },
    CreateChannel {
        space: String,
        name: String,
        private: bool,
    },
    UpdateChannel {
        space: String,
        channel: String,
        name: String,
        private: bool,
    },
    DeleteChannel {
        space: String,
        channel: String,
    },
    LoadMembers {
        space: String,
        channel: Option<String>,
    },
    AddMember {
        space: String,
        channel: Option<String>,
        username: String,
    },
    RemoveMember {
        space: String,
        channel: Option<String>,
        member: String,
    },
}

pub enum AdminResult {
    SpaceCreated(Space),
    SpaceUpdated(Space),
    SpaceDeleted(String),
    SpaceLeft(String),
    ChannelCreated(Channel),
    ChannelUpdated(Channel),
    ChannelDeleted(String),
    Members {
        channel: Option<String>,
        members: Vec<Member>,
    },
    MemberAdded {
        channel: Option<String>,
        member: Member,
    },
    MemberRemoved {
        channel: Option<String>,
        member: String,
    },
}

pub enum Event {
    Restored {
        generation: u64,
        result: Result<Option<(String, Account, Spaces)>, String>,
    },
    CodeRequested {
        generation: u64,
        result: Result<String, String>,
    },
    Verified {
        generation: u64,
        result: Result<(String, Account, Spaces), String>,
    },
    Profiled {
        generation: u64,
        result: Result<(Account, Spaces), String>,
    },
    NavigationPrepared {
        generation: u64,
        navigation: u64,
        result: Result<PreparedNavigation, LoadError>,
    },
    NavigationPrefetched {
        generation: u64,
        request: u64,
        space: Option<String>,
        channel: Option<String>,
        result: Result<crate::navigation::Read, LoadError>,
    },
    ChannelLoaded {
        generation: u64,
        channel: String,
        general: bool,
        result: Result<(History, ChatSession), LoadError>,
    },
    OlderLoaded {
        generation: u64,
        channel: String,
        result: Result<History, LoadError>,
    },
    Sent {
        generation: u64,
        channel: String,
        client_id: String,
        result: Result<Message, SendFailure>,
    },
    Credential {
        generation: u64,
        result: Result<(), String>,
    },
    LoggedOut {
        generation: u64,
        result: Result<(), String>,
    },
    Admin {
        generation: u64,
        result: Result<AdminResult, String>,
    },
    Gateway(GatewayEvent),
}

pub struct SendFailure {
    pub status: Option<u16>,
    pub message: String,
}

#[derive(Debug)]
pub struct LoadError {
    pub message: String,
    pub access_denied: bool,
    pub space_access_denied: bool,
}

impl From<crate::api::ApiError> for LoadError {
    fn from(error: crate::api::ApiError) -> Self {
        Self {
            access_denied: error
                .status
                .is_some_and(|status| matches!(status.as_u16(), 401 | 403 | 404)),
            space_access_denied: false,
            message: error.to_string(),
        }
    }
}

pub struct PreparedNavigation {
    pub detail: Option<SpaceDetail>,
    pub conversation: Option<(History, ChatSession)>,
}

fn prepare_navigation(
    api: &Api,
    token: Option<&str>,
    space: Option<&str>,
    channel: Option<&str>,
    name: &str,
    cached: Option<History>,
) -> Result<PreparedNavigation, LoadError> {
    let detail = space
        .map(|space| api.space(token.unwrap_or_default(), space))
        .transpose()
        .map_err(|error| {
            let mut error = LoadError::from(error);
            error.space_access_denied = error.access_denied;
            error
        })?;
    if let Some(detail) = &detail
        && Some(detail.space.id.as_str()) != space
    {
        return Err(crate::api::ApiError {
            status: None,
            message: "Caper returned another space.".into(),
        }
        .into());
    }
    let selected = if let Some(detail) = &detail {
        if let Some(id) = channel {
            Some(
                detail
                    .channels
                    .iter()
                    .find(|entry| entry.id == id)
                    .ok_or_else(|| crate::api::ApiError {
                        status: Some(reqwest::StatusCode::NOT_FOUND),
                        message: "This channel is no longer accessible.".into(),
                    })?,
            )
        } else {
            detail.channels.first()
        }
    } else {
        None
    };
    let history = if let Some(selected) = selected {
        match cached.filter(|history| {
            history.space.id == detail.as_ref().unwrap().space.id
                && history.channel.id == selected.id
        }) {
            Some(history) => Some(history),
            None => Some(api.history(token, &selected.id, None)?),
        }
    } else if space.is_none() {
        Some(match cached {
            Some(history) => history,
            None => api.general_history(token)?,
        })
    } else {
        None
    };
    if let (Some(detail), Some(history)) = (&detail, &history)
        && (detail.space.id != history.space.id
            || selected.is_none_or(|entry| entry.id != history.channel.id))
    {
        return Err(crate::api::ApiError {
            status: None,
            message: "Caper returned another conversation.".into(),
        }
        .into());
    }
    if let Some(history) = &history {
        if history
            .messages
            .iter()
            .any(|message| message.channel_id != history.channel.id)
        {
            return Err(crate::api::ApiError {
                status: None,
                message: "Caper returned messages from another channel.".into(),
            }
            .into());
        }
        crate::model::Timeline::default()
            .reset(history.messages.clone(), &history.cursor)
            .map_err(|message| crate::api::ApiError {
                status: None,
                message,
            })?;
    }
    let conversation = history
        .map(|history| {
            api.chat_session(token, name)
                .map(|session| (history, session))
        })
        .transpose()?;
    Ok(PreparedNavigation {
        detail,
        conversation,
    })
}

fn prefetch_navigation(
    api: &Api,
    token: Option<&str>,
    space: Option<&str>,
    channel: Option<&str>,
) -> Result<crate::navigation::Read, LoadError> {
    let prepared = prepare_navigation_read(api, token, space, channel)?;
    Ok(crate::navigation::Read {
        detail: prepared.0,
        history: prepared.1,
    })
}

fn prepare_navigation_read(
    api: &Api,
    token: Option<&str>,
    space: Option<&str>,
    channel: Option<&str>,
) -> Result<(Option<SpaceDetail>, Option<History>), LoadError> {
    let detail = space
        .map(|space| api.space(token.unwrap_or_default(), space))
        .transpose()
        .map_err(|error| {
            let mut error = LoadError::from(error);
            error.space_access_denied = error.access_denied;
            error
        })?;
    let selected = detail.as_ref().and_then(|detail| match channel {
        Some(id) => detail.channels.iter().find(|entry| entry.id == id),
        None => detail.channels.first(),
    });
    if channel.is_some() && detail.is_some() && selected.is_none() {
        return Err(LoadError {
            message: "This channel is no longer accessible.".into(),
            access_denied: true,
            space_access_denied: false,
        });
    }
    let history = if let Some(selected) = selected {
        Some(api.history(token, &selected.id, None)?)
    } else if space.is_none() {
        Some(api.general_history(token)?)
    } else {
        None
    };
    Ok((detail, history))
}

pub struct Worker {
    commands: Sender<Command>,
    pub events: Receiver<Event>,
}

impl Worker {
    pub fn new(api: Api, context: egui::Context) -> Self {
        let (commands, incoming) = mpsc::channel();
        let (events, outgoing) = mpsc::channel();
        thread::spawn(move || manage(api, context, incoming, events));
        Self {
            commands,
            events: outgoing,
        }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

fn manage(api: Api, context: egui::Context, incoming: Receiver<Command>, events: Sender<Event>) {
    let mut gateway: Option<gateway::GatewayControl> = None;
    let mut credential_generation = 0;
    while let Ok(command) = incoming.recv() {
        match command {
            Command::Connect {
                generation,
                token,
                channel,
                cursor,
                presence,
            } => {
                if let Some(control) = gateway.take() {
                    control.stop();
                }
                let (gateway_tx, gateway_rx) = mpsc::channel();
                gateway = Some(gateway::spawn(
                    api.base(),
                    token,
                    generation,
                    channel,
                    cursor,
                    presence,
                    gateway_tx,
                ));
                let events = events.clone();
                let context = context.clone();
                thread::spawn(move || {
                    while let Ok(event) = gateway_rx.recv() {
                        send(&events, &context, Event::Gateway(event));
                    }
                });
            }
            Command::StopGateway => {
                if let Some(control) = gateway.take() {
                    control.stop();
                }
            }
            Command::Activity => {
                if let Some(control) = &gateway {
                    control.activity();
                }
            }
            Command::PersistCredential { generation, token } => {
                if advance_generation(&mut credential_generation, generation) {
                    let result = crate::credentials::save(api.base(), &token);
                    send(&events, &context, Event::Credential { generation, result });
                }
            }
            Command::ClearCredential { generation } => {
                if advance_generation(&mut credential_generation, generation) {
                    let result = crate::credentials::delete(api.base());
                    send(&events, &context, Event::Credential { generation, result });
                }
            }
            command => {
                let api = api.clone();
                let events = events.clone();
                let context = context.clone();
                thread::spawn(move || execute(&api, command, &events, &context));
            }
        }
    }
    if let Some(control) = gateway {
        control.stop();
    }
}

fn advance_generation(current: &mut u64, candidate: u64) -> bool {
    if candidate < *current {
        return false;
    }
    *current = candidate;
    true
}

fn execute(api: &Api, command: Command, events: &Sender<Event>, context: &egui::Context) {
    let event = match command {
        Command::Restore { generation } => {
            let result = crate::credentials::load(api.base()).and_then(|token| {
                token.map_or(Ok(None), |token| {
                    let account = api.me(&token).map_err(|error| error.to_string())?;
                    let spaces = api.spaces(&token).map_err(|error| error.to_string())?;
                    Ok(Some((token, account, spaces)))
                })
            });
            Event::Restored { generation, result }
        }
        Command::RequestCode { generation, email } => Event::CodeRequested {
            generation,
            result: api.request_code(&email).map_err(|error| error.to_string()),
        },
        Command::VerifyCode {
            generation,
            challenge,
            code,
        } => {
            let result = api
                .verify_code(&challenge, &code)
                .and_then(|(account, token)| {
                    let spaces = api.spaces(&token)?;
                    Ok((token, account, spaces))
                })
                .map_err(|error| error.to_string());
            Event::Verified { generation, result }
        }
        Command::Profile {
            generation,
            token,
            username,
            display_name,
        } => Event::Profiled {
            generation,
            result: api
                .update_profile(&token, &username, &display_name)
                .and_then(|account| api.spaces(&token).map(|spaces| (account, spaces)))
                .map_err(|error| error.to_string()),
        },
        Command::PrepareNavigation {
            generation,
            navigation,
            token,
            space,
            channel,
            name,
            cached,
        } => Event::NavigationPrepared {
            generation,
            navigation,
            result: prepare_navigation(
                api,
                token.as_deref(),
                space.as_deref(),
                channel.as_deref(),
                &name,
                cached,
            ),
        },
        Command::PrefetchNavigation {
            generation,
            request,
            token,
            space,
            channel,
        } => Event::NavigationPrefetched {
            generation,
            request,
            result: prefetch_navigation(
                api,
                token.as_deref(),
                space.as_deref(),
                channel.as_deref(),
            ),
            space,
            channel,
        },
        Command::LoadChannel {
            generation,
            token,
            channel,
            name,
            general,
        } => {
            let result = (if general {
                api.general_history(token.as_deref())
            } else {
                api.history(token.as_deref(), &channel, None)
            })
            .and_then(|history| {
                api.chat_session(token.as_deref(), &name)
                    .map(|session| (history, session))
            })
            .map_err(LoadError::from);
            Event::ChannelLoaded {
                generation,
                channel,
                general,
                result,
            }
        }
        Command::LoadOlder {
            generation,
            token,
            channel,
            before,
        } => Event::OlderLoaded {
            generation,
            channel: channel.clone(),
            result: api
                .history(token.as_deref(), &channel, Some(&before))
                .map_err(LoadError::from),
        },
        Command::Send {
            generation,
            token,
            chat_token,
            channel,
            client_id,
            text,
        } => Event::Sent {
            generation,
            channel: channel.clone(),
            client_id: client_id.clone(),
            result: api
                .send(token.as_deref(), &chat_token, &channel, &client_id, &text)
                .map_err(|error| SendFailure {
                    status: error.status.map(|status| status.as_u16()),
                    message: error.to_string(),
                }),
        },
        Command::Typing {
            token,
            chat_token,
            channel,
            typing,
        } => {
            let _ = api.typing(token.as_deref(), &chat_token, &channel, typing);
            return;
        }
        Command::Admin {
            generation,
            token,
            operation,
        } => Event::Admin {
            generation,
            result: execute_admin(api, &token, operation).map_err(|error| error.to_string()),
        },
        Command::Logout { generation, token } => Event::LoggedOut {
            generation,
            result: api.logout(&token).map_err(|error| error.to_string()),
        },
        Command::Connect { .. }
        | Command::StopGateway
        | Command::Activity
        | Command::PersistCredential { .. }
        | Command::ClearCredential { .. } => return,
    };
    send(events, context, event);
}

fn execute_admin(
    api: &Api,
    token: &str,
    operation: AdminOperation,
) -> Result<AdminResult, crate::api::ApiError> {
    Ok(match operation {
        AdminOperation::CreateSpace { name } => {
            AdminResult::SpaceCreated(api.create_space(token, &name)?)
        }
        AdminOperation::UpdateSpace { space, name } => {
            AdminResult::SpaceUpdated(api.update_space(token, &space, &name)?)
        }
        AdminOperation::DeleteSpace { space } => {
            api.delete_space(token, &space)?;
            AdminResult::SpaceDeleted(space)
        }
        AdminOperation::LeaveSpace { space, member } => {
            api.remove_member(token, &space, None, &member)?;
            AdminResult::SpaceLeft(space)
        }
        AdminOperation::CreateChannel {
            space,
            name,
            private,
        } => AdminResult::ChannelCreated(api.create_channel(token, &space, &name, private)?),
        AdminOperation::UpdateChannel {
            space,
            channel,
            name,
            private,
        } => AdminResult::ChannelUpdated(
            api.update_channel(token, &space, &channel, &name, private)?,
        ),
        AdminOperation::DeleteChannel { space, channel } => {
            api.delete_channel(token, &space, &channel)?;
            AdminResult::ChannelDeleted(channel)
        }
        AdminOperation::LoadMembers { space, channel } => AdminResult::Members {
            channel: channel.clone(),
            members: api.members(token, &space, channel.as_deref())?.members,
        },
        AdminOperation::AddMember {
            space,
            channel,
            username,
        } => AdminResult::MemberAdded {
            channel: channel.clone(),
            member: api.add_member(token, &space, channel.as_deref(), &username)?,
        },
        AdminOperation::RemoveMember {
            space,
            channel,
            member,
        } => {
            api.remove_member(token, &space, channel.as_deref(), &member)?;
            AdminResult::MemberRemoved { channel, member }
        }
    })
}

fn send(events: &Sender<Event>, context: &egui::Context, event: Event) {
    let _ = events.send(event);
    context.request_repaint();
}

pub fn current(
    generation: u64,
    expected: u64,
    event_channel: Option<&str>,
    selected: Option<&str>,
) -> bool {
    generation == expected && event_channel.is_none_or(|channel| Some(channel) == selected)
}

#[cfg(test)]
mod tests {
    use super::{advance_generation, current};

    #[test]
    fn navigation_rechecks_access_and_prepares_requested_channel_not_first() {
        use std::io::{BufRead, BufReader, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api =
            crate::api::Api::new(&format!("http://{}", server.local_addr().unwrap())).unwrap();
        let worker = std::thread::spawn(move || {
            let detail = r#"{"space":{"id":"s","name":"Space","ownerId":"owner"},"channels":[{"id":"first","spaceId":"s","name":"first","private":false},{"id":"second","spaceId":"s","name":"second","private":true}],"members":[]}"#;
            for (path, body) in [
                ("GET /api/spaces/s", detail),
                (
                    "GET /api/chat/channels/second/messages",
                    r#"{"space":{"id":"s","name":"Space"},"channel":{"id":"second","name":"second"},"messages":[],"cursor":"0","hasMore":false}"#,
                ),
                (
                    "POST /api/chat/session",
                    r#"{"token":"chat","author":{"id":"u","name":"User","isGuest":false}}"#,
                ),
                ("GET /api/spaces/s", detail),
                ("GET /api/spaces/s", detail),
            ] {
                let (stream, _) = server.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(request, format!("{path} HTTP/1.1\r\n"));
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                }
                write!(reader.get_mut(), "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let prepared = super::prepare_navigation(
            &api,
            Some("account"),
            Some("s"),
            Some("second"),
            "User",
            None,
        )
        .unwrap();
        assert_eq!(prepared.detail.unwrap().space.id, "s");
        let mut warm = prepared.conversation.unwrap().0;
        assert_eq!(warm.channel.id, "second");
        warm.channel.id = "revoked".into();
        let failure = super::prepare_navigation(
            &api,
            Some("account"),
            Some("s"),
            Some("revoked"),
            "User",
            Some(warm),
        )
        .err()
        .unwrap();
        assert!(failure.access_denied);
        assert!(!failure.space_access_denied);
        let speculative =
            super::prefetch_navigation(&api, Some("account"), Some("s"), Some("revoked"));
        assert!(
            speculative.err().unwrap().access_denied,
            "an explicit revoked target must not fall back to the first channel"
        );
        worker.join().unwrap();
    }

    #[test]
    fn speculative_navigation_only_performs_reads_and_creates_no_session() {
        use std::io::{BufRead, BufReader, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api =
            crate::api::Api::new(&format!("http://{}", server.local_addr().unwrap())).unwrap();
        let worker = std::thread::spawn(move || {
            for (path, body) in [
                (
                    "GET /api/spaces/s",
                    r#"{"space":{"id":"s","name":"Space","ownerId":"owner"},"channels":[{"id":"c","spaceId":"s","name":"channel","private":true}],"members":[]}"#,
                ),
                (
                    "GET /api/chat/channels/c/messages",
                    r#"{"space":{"id":"s","name":"Space"},"channel":{"id":"c","name":"channel"},"messages":[],"cursor":"7","hasMore":false}"#,
                ),
            ] {
                let (stream, _) = server.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(request, format!("{path} HTTP/1.1\r\n"));
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                }
                write!(reader.get_mut(), "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let read = super::prefetch_navigation(&api, Some("account"), Some("s"), Some("c")).unwrap();
        assert_eq!(read.history.unwrap().cursor, "7");
        worker.join().unwrap();
    }

    #[test]
    fn leave_space_removes_only_self_membership_and_propagates_denial() {
        use std::io::{BufRead, BufReader, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api =
            crate::api::Api::new(&format!("http://{}/", server.local_addr().unwrap())).unwrap();
        let worker = std::thread::spawn(move || {
            for denied in [false, true] {
                let (stream, _) = server.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(
                    request,
                    "DELETE /api/spaces/space-a/members/self-b HTTP/1.1\r\n"
                );
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    headers.push_str(&line);
                }
                assert!(
                    headers
                        .to_ascii_lowercase()
                        .contains("authorization: bearer fixture-token\r\n")
                );
                let response = if denied {
                    "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else {
                    "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n"
                };
                reader.get_mut().write_all(response.as_bytes()).unwrap();
            }
        });
        let leave = || {
            super::execute_admin(
                &api,
                "fixture-token",
                super::AdminOperation::LeaveSpace {
                    space: "space-a".into(),
                    member: "self-b".into(),
                },
            )
        };
        assert!(matches!(leave(), Ok(super::AdminResult::SpaceLeft(id)) if id == "space-a"));
        assert!(leave().is_err());
        worker.join().unwrap();
    }

    #[test]
    fn logout_and_channel_switch_isolate_late_results() {
        assert!(
            !current(4, 5, Some("old"), Some("old")),
            "pre-logout event must be discarded"
        );
        assert!(
            !current(5, 5, Some("old"), Some("new")),
            "old-channel event must be discarded"
        );
        assert!(current(5, 5, Some("new"), Some("new")));
    }

    #[test]
    fn stale_auth_cannot_overwrite_or_delete_newer_vault_generation() {
        let mut generation = 8;
        assert!(!advance_generation(&mut generation, 7));
        assert_eq!(generation, 8);
        assert!(advance_generation(&mut generation, 9));
        assert_eq!(generation, 9);
        assert!(!advance_generation(&mut generation, 8));
    }
}
