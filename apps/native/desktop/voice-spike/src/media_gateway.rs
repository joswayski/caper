#![allow(dead_code)]

use crate::media::Snapshot;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender},
};
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::{Message, client::IntoClientRequest, stream::MaybeTlsStream};

const POLL: Duration = Duration::from_millis(200);
const HEARTBEAT: Duration = Duration::from_secs(10);
const WATCHDOG: Duration = Duration::from_secs(30);
const CONNECT_DEADLINE: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum Event {
    Online { generation: u64 },
    Offline { generation: u64, detail: String },
    Snapshot { generation: u64, snapshot: Snapshot },
    AccessDenied { generation: u64, detail: String },
}

pub struct Control {
    stop: Arc<AtomicBool>,
    activity: Arc<Mutex<Instant>>,
    finished: Receiver<()>,
}

impl Control {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn stop_and_wait(&self) -> bool {
        self.stop();
        self.finished.recv_timeout(Duration::from_secs(1)).is_ok()
    }

    pub fn activity(&self) {
        if let Ok(mut activity) = self.activity.lock() {
            *activity = Instant::now();
        }
    }
}

pub fn spawn(
    base: &url::Url,
    account_token: Option<String>,
    media_token: String,
    channel_id: Option<String>,
    generation: u64,
    events: Sender<Event>,
) -> Control {
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let activity = Arc::new(Mutex::new(Instant::now()));
    let thread_activity = activity.clone();
    let (completion, finished) = mpsc::channel();
    let mut endpoint = base.join("api/chat/events").expect("constant gateway path");
    endpoint
        .set_scheme(if base.scheme() == "https" {
            "wss"
        } else {
            "ws"
        })
        .expect("compatible WebSocket scheme");
    thread::spawn(move || {
        run(
            endpoint,
            account_token,
            media_token,
            channel_id,
            generation,
            events,
            stopped,
            thread_activity,
        );
        let _ = completion.send(());
    });
    Control {
        stop,
        activity,
        finished,
    }
}

#[derive(Debug)]
enum Failure {
    Retry(String),
    Denied(String),
}

#[allow(clippy::too_many_arguments)]
fn run(
    endpoint: url::Url,
    account_token: Option<String>,
    media_token: String,
    channel_id: Option<String>,
    generation: u64,
    events: Sender<Event>,
    stop: Arc<AtomicBool>,
    activity: Arc<Mutex<Instant>>,
) {
    let mut attempt = 0_u32;
    while !stop.load(Ordering::Relaxed) {
        let result = connect_once(
            &endpoint,
            account_token.as_deref(),
            &media_token,
            channel_id.as_deref(),
            generation,
            &events,
            &stop,
            &activity,
        );
        if stop.load(Ordering::Relaxed) {
            return;
        }
        match result {
            Ok(()) => {}
            Err(Failure::Denied(detail)) => {
                let _ = events.send(Event::AccessDenied { generation, detail });
                return;
            }
            Err(Failure::Retry(detail)) => {
                let _ = events.send(Event::Offline { generation, detail });
            }
        }
        attempt = attempt.saturating_add(1);
        let delay = Duration::from_millis((250_u64 << attempt.min(4)).min(5_000));
        let deadline = Instant::now() + delay;
        while Instant::now() < deadline {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn connect_once(
    endpoint: &url::Url,
    account_token: Option<&str>,
    media_token: &str,
    channel_id: Option<&str>,
    generation: u64,
    events: &Sender<Event>,
    stop: &Arc<AtomicBool>,
    activity: &Mutex<Instant>,
) -> Result<(), Failure> {
    let mut request = endpoint
        .as_str()
        .into_client_request()
        .map_err(|_| Failure::Retry("invalid voice gateway endpoint".into()))?;
    if let Some(token) = account_token {
        let mut value = tungstenite::http::HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| Failure::Denied("invalid account credential".into()))?;
        value.set_sensitive(true);
        request
            .headers_mut()
            .insert(tungstenite::http::header::AUTHORIZATION, value);
    }
    let (mut socket, _) =
        connect_bounded(endpoint, request, stop).map_err(|error| match *error {
            tungstenite::Error::Http(response)
                if matches!(response.status().as_u16(), 401 | 403 | 404) =>
            {
                Failure::Denied("voice access or account session expired".into())
            }
            _ => Failure::Retry("voice updates are offline".into()),
        })?;
    set_timeout(socket.get_mut(), POLL)
        .map_err(|_| Failure::Retry("could not configure voice gateway".into()))?;
    let subscription = uuid::Uuid::new_v4().to_string();
    let mut subscribed = false;
    let mut last_server = Instant::now();
    let mut last_heartbeat = Instant::now();
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = socket.close(None);
            return Ok(());
        }
        if subscribed && last_heartbeat.elapsed() >= HEARTBEAT {
            let activity_age = activity.lock().map_or(86_400_000, |at| {
                at.elapsed().as_millis().min(86_400_000) as u64
            });
            socket
                .send(Message::Text(
                    json!({"type":"heartbeat","activityAgeMs":activity_age})
                        .to_string()
                        .into(),
                ))
                .map_err(|_| Failure::Retry("voice gateway heartbeat failed".into()))?;
            last_heartbeat = Instant::now();
        }
        if last_server.elapsed() >= WATCHDOG {
            return Err(Failure::Retry("voice gateway timed out".into()));
        }
        let frame = match socket.read() {
            Ok(frame) => {
                last_server = Instant::now();
                frame
            }
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(_) => return Err(Failure::Retry("voice gateway disconnected".into())),
        };
        let Message::Text(text) = frame else {
            continue;
        };
        let value: Value = serde_json::from_str(&text)
            .map_err(|_| Failure::Retry("voice gateway returned an invalid frame".into()))?;
        match value["type"].as_str() {
            Some("hello") if !subscribed => {
                socket
                    .send(Message::Text(
                        json!({
                            "type":"subscribe", "id":subscription, "kind":"media",
                            "token":media_token,
                            "channelId":channel_id
                        })
                        .to_string()
                        .into(),
                    ))
                    .map_err(|_| Failure::Retry("voice subscription failed".into()))?;
                subscribed = true;
            }
            Some("heartbeat") => {}
            Some("subscribed") if value["id"] == subscription => {
                let _ = events.send(Event::Online { generation });
            }
            Some("event") if value["id"] == subscription => {
                let snapshot: Snapshot = serde_json::from_value(value["event"].clone())
                    .map_err(|_| Failure::Retry("voice gateway returned invalid state".into()))?;
                let _ = events.send(Event::Snapshot {
                    generation,
                    snapshot,
                });
            }
            Some("error") if value["id"] == subscription => {
                let status = value["status"].as_u64().unwrap_or(500);
                let detail = value["error"]
                    .as_str()
                    .unwrap_or("voice subscription rejected")
                    .to_owned();
                if matches!(status, 401 | 403 | 404) {
                    return Err(Failure::Denied(detail));
                }
                return Err(Failure::Retry(detail));
            }
            Some("migrating") => return Ok(()),
            _ => {}
        }
    }
}

// Do not use tungstenite::connect: it follows redirects with the original
// Authorization header, and its DNS/TCP/upgrade phases have no I/O deadline.
fn connect_bounded(
    endpoint: &url::Url,
    request: tungstenite::http::Request<()>,
    stop: &Arc<AtomicBool>,
) -> Result<
    (
        tungstenite::WebSocket<MaybeTlsStream<std::net::TcpStream>>,
        tungstenite::handshake::client::Response,
    ),
    Box<tungstenite::Error>,
> {
    use std::net::{TcpStream, ToSocketAddrs};
    use std::sync::mpsc;
    use tungstenite::error::UrlError;

    let host = endpoint
        .host_str()
        .ok_or(tungstenite::Error::Url(UrlError::NoHostName))?;
    let port = endpoint
        .port_or_known_default()
        .ok_or(tungstenite::Error::Url(UrlError::NoHostName))?;
    let host = host.to_owned();
    let (send, receive) = mpsc::channel();
    // Resolver calls cannot be forcibly interrupted by the stdlib. Keep the
    // credential on this thread; the detached resolver receives host/port only.
    thread::spawn(move || {
        let _ = send.send(
            (host.as_str(), port)
                .to_socket_addrs()
                .map(|items| items.collect::<Vec<_>>()),
        );
    });
    let deadline = Instant::now() + CONNECT_DEADLINE;
    let addresses = loop {
        if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
            return Err(Box::new(tungstenite::Error::Url(
                UrlError::UnableToConnect(endpoint.to_string()),
            )));
        }
        match receive.recv_timeout(POLL) {
            Ok(result) => break result.map_err(tungstenite::Error::Io)?,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(Box::new(tungstenite::Error::Url(
                    UrlError::UnableToConnect(endpoint.to_string()),
                )));
            }
        }
    };
    let mut last_error = None;
    for address in addresses {
        if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
            break;
        }
        match TcpStream::connect_timeout(
            &address,
            POLL.min(deadline.saturating_duration_since(Instant::now())),
        ) {
            Ok(stream) => {
                stream
                    .set_read_timeout(Some(POLL))
                    .map_err(tungstenite::Error::Io)?;
                stream
                    .set_write_timeout(Some(POLL))
                    .map_err(tungstenite::Error::Io)?;
                // A per-read timeout alone cannot bound a peer that trickles
                // TLS or upgrade bytes indefinitely. Shutdown of a clone
                // interrupts the original socket even inside the synchronous
                // TLS handshake, enforcing the original end-to-end deadline.
                let interrupt = stream.try_clone().map_err(tungstenite::Error::Io)?;
                let stopped = Arc::clone(stop);
                let completed = Arc::new(AtomicBool::new(false));
                let finished = Arc::clone(&completed);
                let watchdog = thread::spawn(move || {
                    while !finished.load(Ordering::Acquire) {
                        if stopped.load(Ordering::Relaxed) || Instant::now() >= deadline {
                            let _ = interrupt.shutdown(std::net::Shutdown::Both);
                            return;
                        }
                        thread::sleep(Duration::from_millis(20));
                    }
                });
                // One handshake attempt; redirects are never followed.
                let result = tungstenite::client_tls_with_config(request, stream, None, None)
                    .map_err(|error| match error {
                        tungstenite::HandshakeError::Failure(error) => error,
                        tungstenite::HandshakeError::Interrupted(_) => tungstenite::Error::Io(
                            std::io::Error::new(std::io::ErrorKind::TimedOut, "upgrade timed out"),
                        ),
                    });
                completed.store(true, Ordering::Release);
                let _ = watchdog.join();
                if stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
                    return Err(Box::new(tungstenite::Error::Io(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "voice connection cancelled or timed out",
                    ))));
                }
                return result.map_err(Box::new);
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(Box::new(tungstenite::Error::Io(last_error.unwrap_or_else(
        || {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "voice connection cancelled or timed out",
            )
        },
    ))))
}

#[cfg(unix)]
fn set_timeout(
    stream: &mut MaybeTlsStream<std::net::TcpStream>,
    timeout: Duration,
) -> std::io::Result<()> {
    match stream {
        MaybeTlsStream::Plain(stream) => {
            stream.set_read_timeout(Some(timeout))?;
            stream.set_write_timeout(Some(timeout))
        }
        MaybeTlsStream::Rustls(stream) => {
            stream.get_mut().set_read_timeout(Some(timeout))?;
            stream.get_mut().set_write_timeout(Some(timeout))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use tungstenite::handshake::server::{ErrorResponse, Request, Response};

    fn trickle_upgrade(listener: TcpListener, ready: mpsc::Sender<()>) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = [0_u8; 2048];
            let received = stream.read(&mut bytes).unwrap();
            assert!(String::from_utf8_lossy(&bytes[..received]).contains("GET /api/chat/events"));
            // A valid but unterminated header field keeps the HTTP parser in
            // progress. Each byte arrives inside the socket's 200ms timeout.
            stream
                .write_all(b"HTTP/1.1 101 Switching Protocols\r\nX-Slow: ")
                .unwrap();
            ready.send(()).unwrap();
            let started = Instant::now();
            let continuation = b"aaaaaaaaaaaa\r\nX-Slow: ";
            let mut index = 0;
            while started.elapsed() < Duration::from_secs(7) {
                thread::sleep(Duration::from_millis(100));
                if stream.write_all(&continuation[index..index + 1]).is_err() {
                    break;
                }
                index = (index + 1) % continuation.len();
            }
        })
    }

    #[test]
    fn stop_interrupts_continuously_trickling_upgrade() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (ready, seen) = mpsc::channel();
        let server = trickle_upgrade(listener, ready);
        let (events, incoming) = mpsc::channel();
        let base = url::Url::parse(&format!("http://{address}/")).unwrap();
        let control = spawn(
            &base,
            Some("account-secret".into()),
            "media".into(),
            None,
            13,
            events,
        );
        seen.recv_timeout(Duration::from_secs(2)).unwrap();
        thread::sleep(Duration::from_millis(280));
        let started = Instant::now();
        assert!(
            control.stop_and_wait(),
            "cancel must interrupt the active HTTP parser"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(
            incoming.try_recv().is_err(),
            "cancel must not report a fresh offline event"
        );
        server.join().unwrap();
    }

    #[test]
    fn total_deadline_interrupts_continuously_trickling_upgrade() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (ready, seen) = mpsc::channel();
        let server = trickle_upgrade(listener, ready);
        let endpoint = url::Url::parse(&format!("ws://{address}/api/chat/events")).unwrap();
        let request = endpoint.as_str().into_client_request().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let started = Instant::now();
        let result = connect_bounded(&endpoint, request, &stop);
        assert!(result.is_err());
        assert!(started.elapsed() >= CONNECT_DEADLINE);
        assert!(started.elapsed() < CONNECT_DEADLINE + Duration::from_secs(1));
        seen.recv_timeout(Duration::from_secs(1)).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn cross_origin_redirect_never_receives_account_credential() {
        let attacker = TcpListener::bind("127.0.0.1:0").unwrap();
        attacker.set_nonblocking(true).unwrap();
        let address = attacker.local_addr().unwrap();
        let legitimate = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = legitimate.local_addr().unwrap();
        let server = thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = legitimate.accept().unwrap();
            let mut bytes = [0_u8; 2048];
            let size = stream.read(&mut bytes).unwrap();
            assert!(String::from_utf8_lossy(&bytes[..size]).contains("Bearer account-secret"));
            stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: ws://{address}/steal\r\nContent-Length: 0\r\n\r\n").as_bytes()).unwrap();
        });
        let endpoint = url::Url::parse(&format!("ws://{origin}/api/chat/events")).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let mut request = endpoint.as_str().into_client_request().unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer account-secret".parse().unwrap());
        assert!(
            matches!(connect_bounded(&endpoint, request, &stop), Err(error) if matches!(error.as_ref(), tungstenite::Error::Http(response) if response.status().is_redirection()))
        );
        server.join().unwrap();
        assert!(attacker.accept().is_err());
    }

    #[test]
    fn stop_interrupts_silent_upgrade_without_waiting_for_read() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut bytes = [0_u8; 2048];
            let _ = std::io::Read::read(&mut stream, &mut bytes);
            thread::sleep(Duration::from_millis(400));
        });
        let (events, _) = mpsc::channel();
        let base = url::Url::parse(&format!("http://{address}/")).unwrap();
        let control = spawn(
            &base,
            Some("account-secret".into()),
            "capability".into(),
            None,
            12,
            events,
        );
        thread::sleep(Duration::from_millis(40));
        let started = Instant::now();
        assert!(control.stop_and_wait());
        server.join().unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn media_capability_stays_in_subscription_and_stop_closes_promptly() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket =
                tungstenite::accept_hdr(stream, |request: &Request, response: Response| {
                    assert_eq!(request.uri().path(), "/api/chat/events");
                    assert!(request.uri().query().is_none());
                    assert_eq!(request.headers()["authorization"], "Bearer account-secret");
                    assert!(request.headers().get("x-caper-media-token").is_none());
                    Ok(response)
                })
                .unwrap();
            socket
                .send(Message::Text(
                    json!({"type":"hello","idleTimeoutSeconds":600,"serverTime":0})
                        .to_string()
                        .into(),
                ))
                .unwrap();
            let Message::Text(subscribe) = socket.read().unwrap() else {
                panic!("expected media subscription")
            };
            let subscribe: Value = serde_json::from_str(&subscribe).unwrap();
            assert_eq!(subscribe["kind"], "media");
            assert_eq!(subscribe["channelId"], "chan00000001");
            assert_eq!(subscribe["token"], "media-secret");
            socket
                .send(Message::Text(
                    json!({"type":"heartbeat"}).to_string().into(),
                ))
                .unwrap();
            socket
                .get_mut()
                .set_read_timeout(Some(Duration::from_millis(120)))
                .unwrap();
            assert!(
                matches!(socket.read(), Err(tungstenite::Error::Io(error)) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)),
                "server heartbeat must not trigger an immediate client reply"
            );
            socket
                .get_mut()
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            assert!(matches!(socket.read(), Ok(Message::Close(_))));
        });
        let (events, _) = mpsc::channel();
        let base = url::Url::parse(&format!("http://{address}/")).unwrap();
        let control = spawn(
            &base,
            Some("account-secret".into()),
            "media-secret".into(),
            Some("chan00000001".into()),
            7,
            events,
        );
        thread::sleep(Duration::from_millis(250));
        control.stop();
        server.join().unwrap();
    }

    #[test]
    fn denied_upgrade_is_terminal_for_current_generation() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _: Result<_, _> = tungstenite::accept_hdr(stream, |_: &Request, _: Response| {
                let mut denied = ErrorResponse::new(Some("revoked".into()));
                *denied.status_mut() = tungstenite::http::StatusCode::FORBIDDEN;
                Err(denied)
            });
        });
        let (events, incoming) = mpsc::channel();
        let base = url::Url::parse(&format!("http://{address}/")).unwrap();
        let _control = spawn(
            &base,
            Some("revoked".into()),
            "expired-media".into(),
            Some("chan00000001".into()),
            11,
            events,
        );
        let denied = (0..3).find_map(|_| {
            let event = incoming.recv_timeout(Duration::from_secs(1)).ok()?;
            match event {
                Event::AccessDenied { generation, .. } => Some(generation),
                _ => None,
            }
        });
        assert_eq!(denied, Some(11));
        server.join().unwrap();
    }
}

#[cfg(windows)]
fn set_timeout(
    stream: &mut MaybeTlsStream<std::net::TcpStream>,
    timeout: Duration,
) -> std::io::Result<()> {
    match stream {
        MaybeTlsStream::Plain(stream) => {
            stream.set_read_timeout(Some(timeout))?;
            stream.set_write_timeout(Some(timeout))
        }
        MaybeTlsStream::Rustls(stream) => {
            stream.get_mut().set_read_timeout(Some(timeout))?;
            stream.get_mut().set_write_timeout(Some(timeout))
        }
        _ => Ok(()),
    }
}
