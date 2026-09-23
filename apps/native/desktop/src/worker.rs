use crate::api::Api;
use crate::gateway::{self, GatewayEvent};
use crate::model::{Account, ChatSession, History, Message, SpaceDetail, Spaces};
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
    LoadSpace {
        generation: u64,
        token: String,
        space: String,
    },
    LoadChannel {
        generation: u64,
        token: String,
        channel: String,
        name: String,
    },
    Connect {
        generation: u64,
        token: String,
        channel: String,
        cursor: String,
    },
    Send {
        generation: u64,
        token: String,
        chat_token: String,
        channel: String,
        client_id: String,
        text: String,
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
    SpaceLoaded {
        generation: u64,
        space: String,
        result: Result<SpaceDetail, String>,
    },
    ChannelLoaded {
        generation: u64,
        channel: String,
        result: Result<(History, ChatSession), LoadError>,
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
    Gateway(GatewayEvent),
}

pub struct SendFailure {
    pub status: Option<u16>,
    pub message: String,
}

pub struct LoadError {
    pub message: String,
    pub access_denied: bool,
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
        Command::LoadSpace {
            generation,
            token,
            space,
        } => Event::SpaceLoaded {
            generation,
            space: space.clone(),
            result: api.space(&token, &space).map_err(|error| error.to_string()),
        },
        Command::LoadChannel {
            generation,
            token,
            channel,
            name,
        } => {
            let result = api
                .history(&token, &channel)
                .and_then(|history| {
                    api.chat_session(&token, &name)
                        .map(|session| (history, session))
                })
                .map_err(|error| LoadError {
                    access_denied: error
                        .status
                        .is_some_and(|status| matches!(status.as_u16(), 401 | 403 | 404)),
                    message: error.to_string(),
                });
            Event::ChannelLoaded {
                generation,
                channel,
                result,
            }
        }
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
                .send(&token, &chat_token, &channel, &client_id, &text)
                .map_err(|error| SendFailure {
                    status: error.status.map(|status| status.as_u16()),
                    message: error.to_string(),
                }),
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
