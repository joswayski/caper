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
// Longest a handshake read or write blocks before stop and the deadline are
// checked again.
const HANDSHAKE_SLICE: Duration = Duration::from_millis(50);
pub(crate) const CATCHUP_DEADLINE: Duration = Duration::from_secs(15);

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
pub(crate) enum Failure {
    Retry(String),
    Denied(String),
}

// Shared by the desktop chat and voice streams. Replacement handshakes run on
// another thread so DNS/TLS/upgrade cannot stop delivery on the draining socket.
pub(crate) struct Connection {
    pub socket: tungstenite::WebSocket<MaybeTlsStream<HandshakeStream>>,
    pub hello: bool,
    pub opened: Instant,
    last_server: Instant,
    last_heartbeat: Instant,
}

impl Connection {
    pub fn open(
        endpoint: &url::Url,
        token: Option<&str>,
        stop: &Arc<AtomicBool>,
    ) -> Result<Self, Failure> {
        let opened = Instant::now();
        let mut request = endpoint
            .as_str()
            .into_client_request()
            .map_err(|_| Failure::Retry("invalid gateway endpoint".into()))?;
        if let Some(token) = token {
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
                    Failure::Denied("channel access or account session expired".into())
                }
                _ => Failure::Retry("live updates are offline".into()),
            })?;
        set_timeout(socket.get_mut(), POLL)
            .map_err(|_| Failure::Retry("could not configure gateway".into()))?;
        Ok(Self {
            socket,
            hello: false,
            opened,
            last_server: Instant::now(),
            last_heartbeat: Instant::now(),
        })
    }

    pub fn send(&mut self, frame: Value) -> Result<(), Failure> {
        self.socket
            .send(Message::Text(frame.to_string().into()))
            .map_err(|_| Failure::Retry("gateway send failed".into()))
    }

    pub fn read(&mut self, activity: &Mutex<Instant>) -> Result<Option<Value>, Failure> {
        if self.hello && self.last_heartbeat.elapsed() >= HEARTBEAT {
            let age = activity.lock().map_or(86_400_000, |at| {
                at.elapsed().as_millis().min(86_400_000) as u64
            });
            self.send(json!({"type":"heartbeat", "activityAgeMs":age}))?;
            self.last_heartbeat = Instant::now();
        }
        if self.last_server.elapsed() >= WATCHDOG {
            return Err(Failure::Retry("gateway timed out".into()));
        }
        match self.socket.read() {
            Ok(Message::Text(text)) => {
                self.last_server = Instant::now();
                serde_json::from_str(&text)
                    .map(Some)
                    .map_err(|_| Failure::Retry("gateway returned an invalid frame".into()))
            }
            Ok(Message::Close(_)) => Err(Failure::Retry("gateway disconnected".into())),
            Ok(_) => {
                self.last_server = Instant::now();
                Ok(None)
            }
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(_) => Err(Failure::Retry("gateway disconnected".into())),
        }
    }

    pub fn close(&mut self) {
        let _ = self.socket.close(None);
    }
}

pub(crate) struct ConnectionAttempt {
    receive: Receiver<Result<Connection, Failure>>,
    cancel: Arc<AtomicBool>,
}

impl ConnectionAttempt {
    pub fn start(endpoint: &url::Url, token: Option<&str>) -> Self {
        let endpoint = endpoint.clone();
        let token = token.map(str::to_owned);
        let cancel = Arc::new(AtomicBool::new(false));
        let stopped = cancel.clone();
        let (send, receive) = mpsc::channel();
        thread::spawn(move || {
            let _ = send.send(Connection::open(&endpoint, token.as_deref(), &stopped));
        });
        Self { receive, cancel }
    }

    pub fn take(&self) -> Option<Result<Connection, Failure>> {
        match self.receive.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err(Failure::Retry("gateway connection failed".into())))
            }
        }
    }
}

impl Drop for ConnectionAttempt {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

struct MediaStream {
    connection: Connection,
    subscribed: bool,
    revision: Option<u64>,
}

impl MediaStream {
    fn new(connection: Connection) -> Self {
        Self {
            connection,
            subscribed: false,
            revision: None,
        }
    }
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
    let mut active = MediaStream::new(Connection::open(endpoint, account_token, stop)?);
    let mut candidate: Option<MediaStream> = None;
    let mut opening: Option<ConnectionAttempt> = None;
    let mut migrating = false;
    let mut retry_at = Instant::now();
    let mut applied_revision = None;
    let subscription = uuid::Uuid::new_v4().to_string();
    loop {
        if stop.load(Ordering::Relaxed) {
            active.connection.close();
            if let Some(candidate) = &mut candidate {
                candidate.connection.close();
            }
            return Ok(());
        }
        if migrating && candidate.is_none() && opening.is_none() && Instant::now() >= retry_at {
            opening = Some(ConnectionAttempt::start(endpoint, account_token));
        }
        if let Some(result) = opening.as_ref().and_then(ConnectionAttempt::take) {
            opening = None;
            match result {
                Ok(connection) => candidate = Some(MediaStream::new(connection)),
                Err(_) => retry_at = Instant::now() + Duration::from_millis(250),
            }
        }
        for replacement in [false, true] {
            let stream = if replacement {
                let Some(stream) = &mut candidate else {
                    continue;
                };
                stream
            } else {
                &mut active
            };
            let result = (|| {
                if replacement && stream.connection.opened.elapsed() >= CATCHUP_DEADLINE {
                    return Err(Failure::Retry("replacement did not catch up".into()));
                }
                let Some(value) = stream.connection.read(activity)? else {
                    return Ok(false);
                };
                match value["type"].as_str() {
                    Some("hello") if !stream.connection.hello => {
                        stream.connection.hello = true;
                        stream.connection.send(json!({"type":"subscribe", "id":subscription, "kind":"media", "token":media_token, "channelId":channel_id}))?;
                    }
                    Some("subscribed") if value["id"] == subscription => {
                        stream.subscribed = true;
                        if !replacement {
                            let _ = events.send(Event::Online { generation });
                        }
                    }
                    Some("event") if value["id"] == subscription => {
                        let snapshot: Snapshot = serde_json::from_value(value["event"].clone())
                            .map_err(|_| {
                                Failure::Retry("voice gateway returned invalid state".into())
                            })?;
                        let revision = snapshot
                            .revision
                            .ok_or_else(|| Failure::Retry("missing voice revision".into()))?;
                        stream.revision = Some(
                            stream
                                .revision
                                .map_or(revision, |previous| previous.max(revision)),
                        );
                        if applied_revision.is_none_or(|previous| revision > previous) {
                            applied_revision = Some(revision);
                            let _ = events.send(Event::Snapshot {
                                generation,
                                snapshot,
                            });
                        }
                    }
                    Some("error") if value["id"] == subscription => {
                        let detail = value["error"]
                            .as_str()
                            .unwrap_or("voice subscription rejected")
                            .to_owned();
                        return Err(
                            if matches!(value["status"].as_u64(), Some(401 | 403 | 404)) {
                                Failure::Denied(detail)
                            } else {
                                Failure::Retry(detail)
                            },
                        );
                    }
                    Some("migrating") => {
                        if replacement {
                            return Err(Failure::Retry("replacement is draining".into()));
                        }
                        return Ok(true);
                    }
                    _ => {}
                }
                Ok(false)
            })();
            match result {
                Ok(true) => migrating = true,
                Ok(false) => {}
                Err(_) if replacement => {
                    if let Some(mut failed) = candidate.take() {
                        failed.connection.close();
                    }
                    retry_at = Instant::now() + Duration::from_millis(250);
                }
                Err(error) => return Err(error),
            }
        }
        if candidate.as_ref().is_some_and(|stream| {
            stream.subscribed
                && stream.revision.is_some_and(|revision| {
                    applied_revision.is_none_or(|applied| revision >= applied)
                })
        }) {
            let mut old = std::mem::replace(&mut active, candidate.take().unwrap());
            old.connection.close();
            migrating = false;
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
        tungstenite::WebSocket<MaybeTlsStream<HandshakeStream>>,
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
                // Blocking reads and writes are cut into short slices, and stop
                // and the total deadline are checked between them. A shutdown
                // from another thread is not enough: Windows does not reliably
                // wake a recv that is already blocked, so a trickling peer could
                // hold the cancelled handshake until its read timeout. A slow
                // peer may still pause between bytes for up to the time left.
                let stream = HandshakeStream {
                    stream,
                    guard: Some((Arc::clone(stop), deadline)),
                };
                // One handshake attempt; redirects are never followed.
                let result = tungstenite::client_tls_with_config(request, stream, None, None)
                    .map_err(|error| match error {
                        tungstenite::HandshakeError::Failure(error) => error,
                        tungstenite::HandshakeError::Interrupted(_) => tungstenite::Error::Io(
                            std::io::Error::new(std::io::ErrorKind::TimedOut, "upgrade timed out"),
                        ),
                    });
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

// The TCP stream under a gateway WebSocket. While `guard` is set (during the
// handshake), timeouts are retried in HANDSHAKE_SLICE steps until stop or the
// deadline; afterwards I/O passes straight through with the caller's timeouts.
#[derive(Debug)]
pub(crate) struct HandshakeStream {
    stream: std::net::TcpStream,
    guard: Option<(Arc<AtomicBool>, Instant)>,
}

impl HandshakeStream {
    fn sliced<T>(
        &mut self,
        set_timeout: fn(&std::net::TcpStream, Option<Duration>) -> std::io::Result<()>,
        mut io: impl FnMut(&mut std::net::TcpStream) -> std::io::Result<T>,
    ) -> std::io::Result<T> {
        let Some((stop, deadline)) = &self.guard else {
            return io(&mut self.stream);
        };
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if stop.load(Ordering::Relaxed) || left.is_zero() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "voice connection cancelled or timed out",
                ));
            }
            set_timeout(&self.stream, Some(left.min(HANDSHAKE_SLICE)))?;
            match io(&mut self.stream) {
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                result => return result,
            }
        }
    }
}

impl std::io::Read for HandshakeStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.sliced(std::net::TcpStream::set_read_timeout, |stream| {
            std::io::Read::read(stream, buf)
        })
    }
}

impl std::io::Write for HandshakeStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.sliced(std::net::TcpStream::set_write_timeout, |stream| {
            std::io::Write::write(stream, buf)
        })
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::Write::flush(&mut self.stream)
    }
}

// Ends the handshake guard and applies the steady-state I/O timeout.
#[cfg(any(unix, windows))]
fn set_timeout(
    stream: &mut MaybeTlsStream<HandshakeStream>,
    timeout: Duration,
) -> std::io::Result<()> {
    let stream = match stream {
        MaybeTlsStream::Plain(stream) => stream,
        MaybeTlsStream::Rustls(stream) => stream.get_mut(),
        _ => return Ok(()),
    };
    stream.guard = None;
    stream.stream.set_read_timeout(Some(timeout))?;
    stream.stream.set_write_timeout(Some(timeout))
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use tungstenite::handshake::server::{ErrorResponse, Request, Response};

    #[test]
    fn media_handoff_preserves_capability_and_current_roster_through_failed_candidates() {
        type Socket = tungstenite::WebSocket<std::net::TcpStream>;
        fn send(socket: &mut Socket, frame: Value) {
            socket
                .send(Message::Text(frame.to_string().into()))
                .unwrap();
        }
        fn accept(listener: &TcpListener) -> (Socket, String) {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut socket =
                tungstenite::accept_hdr(stream, |request: &Request, response: Response| {
                    assert_eq!(request.headers()["authorization"], "Bearer account-secret");
                    assert!(request.uri().query().is_none());
                    Ok(response)
                })
                .unwrap();
            send(&mut socket, json!({"type":"hello"}));
            let frame: Value =
                serde_json::from_str(&socket.read().unwrap().into_text().unwrap()).unwrap();
            assert_eq!(frame["token"], "stable-capability");
            let id = frame["id"].as_str().unwrap().to_owned();
            send(&mut socket, json!({"type":"subscribed", "id":id}));
            (socket, id)
        }
        fn snapshot(socket: &mut Socket, id: &str, revision: u64) {
            send(
                socket,
                json!({"type":"event", "id":id, "event":{"type":"snapshot", "revision":revision, "participants":[]}}),
            );
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (advance, advanced) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut old, id) = accept(&listener);
            snapshot(&mut old, &id, 5);
            advanced.recv_timeout(Duration::from_secs(5)).unwrap();
            send(&mut old, json!({"type":"migrating"}));
            let (mut replacement, next_id) = accept(&listener);
            assert_eq!(id, next_id);
            snapshot(&mut replacement, &id, 4);
            snapshot(&mut old, &id, 6);
            advanced.recv_timeout(Duration::from_secs(5)).unwrap();
            snapshot(&mut replacement, &id, 6);
            assert!(
                matches!(old.read(), Ok(Message::Close(_))),
                "stale revision cannot promote a replacement"
            );
            send(&mut replacement, json!({"type":"migrating"}));
            let (stream, _) = listener.accept().unwrap();
            let _: Result<_, _> = tungstenite::accept_hdr(stream, |_: &Request, _: Response| {
                // Hold the replacement upgrade until delivery on the old
                // socket is observed. A blocking handoff cannot pass this.
                snapshot(&mut replacement, &id, 7);
                advanced.recv_timeout(Duration::from_secs(5)).unwrap();
                let mut response = ErrorResponse::new(None);
                *response.status_mut() = tungstenite::http::StatusCode::SERVICE_UNAVAILABLE;
                Err(response)
            });
            let (mut last, last_id) = accept(&listener);
            assert_eq!(id, last_id);
            snapshot(&mut last, &id, 8);
            assert!(matches!(replacement.read(), Ok(Message::Close(_))));
            assert!(
                matches!(last.read(), Ok(Message::Close(_))),
                "stop closes promoted socket"
            );
        });
        let (events, incoming) = mpsc::channel();
        let control = spawn(
            &url::Url::parse(&format!("http://{address}")).unwrap(),
            Some("account-secret".into()),
            "stable-capability".into(),
            Some("voice".into()),
            7,
            events,
        );
        let mut revisions = vec![];
        let mut online = 0;
        loop {
            match incoming.recv_timeout(Duration::from_secs(5)).unwrap() {
                Event::Snapshot { snapshot, .. } => {
                    let revision = snapshot.revision.unwrap();
                    revisions.push(revision);
                    if revision == 8 {
                        break;
                    }
                    advance.send(()).unwrap();
                }
                Event::Online { .. } => online += 1,
                other => panic!("planned handoff must not go offline: {other:?}"),
            }
        }
        assert_eq!(
            revisions,
            [5, 6, 7, 8],
            "overlap and stale snapshots are delivered once"
        );
        assert_eq!(online, 1, "candidate acknowledgment is not a reconnect");
        assert!(control.stop_and_wait());
        server.join().unwrap();
    }

    fn trickle_upgrade(listener: TcpListener, ready: mpsc::Sender<()>) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            // Small writes must not wait for Nagle/delayed ACKs: that
            // can trip the client's idle timeout instead of its total deadline.
            stream.set_nodelay(true).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = [0_u8; 2048];
            let received = stream.read(&mut bytes).unwrap();
            assert!(String::from_utf8_lossy(&bytes[..received]).contains("GET /api/chat/events"));
            // A valid but unterminated header field keeps the HTTP parser in
            // progress. Use chunks, not individual bytes: tungstenite rejects
            // too many undersized reads before the total deadline is reached.
            // One header also avoids its header-count limit; even seven seconds
            // of these chunks stays below its total handshake byte limit.
            stream
                .write_all(b"HTTP/1.1 101 Switching Protocols\r\nX-Slow: ")
                .unwrap();
            ready.send(()).unwrap();
            let started = Instant::now();
            let continuation = [b'a'; 256];
            while started.elapsed() < Duration::from_secs(7) {
                // Leave scheduler headroom inside the 200ms per-read timeout.
                thread::sleep(Duration::from_millis(50));
                if stream.write_all(&continuation).is_err() {
                    break;
                }
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
        let elapsed = started.elapsed();
        let error = result.expect_err("unterminated upgrade must time out");
        assert!(
            elapsed >= CONNECT_DEADLINE,
            "upgrade failed before the total deadline after {elapsed:?}: {error}"
        );
        // Slack for a loaded Windows runner; still well before the 7s trickle ends.
        assert!(elapsed < CONNECT_DEADLINE + Duration::from_millis(1500));
        assert!(matches!(
            *error,
            tungstenite::Error::Io(ref error) if error.kind() == std::io::ErrorKind::TimedOut
        ));
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
