//! Test client for one chat subscription on the multiplexed gateway protocol,
//! the only protocol clients use.
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_tungstenite::tungstenite::{Error, Message, client::IntoClientRequest};

pub(crate) type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The subscription ID `chat_socket` uses; its refusal names this ID.
pub(crate) const CHAT: &str = "chat";

/// Opens a signed-in gateway socket subscribed to `channel` after `after`.
/// Fails only when the gateway refuses the socket itself (signed out,
/// draining); a refused subscription arrives as an `error` frame.
pub(crate) async fn chat_socket(
    address: std::net::SocketAddr,
    cookie: &str,
    channel: &str,
    after: &str,
) -> Result<Socket, Error> {
    let mut request = format!("ws://{address}/api/chat/events")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("cookie", format!("caper_session={cookie}").parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await?;
    for frame in [
        json!({"type":"heartbeat","activityAgeMs":0}),
        json!({"type":"subscribe","id":CHAT,"kind":"chat","channelId":channel,"after":after}),
    ] {
        socket.send(Message::Text(frame.to_string().into())).await?;
    }
    Ok(socket)
}

/// The next chat event (unwrapped from its `event` frame), or the next
/// `migrating` or `error` frame. Keeps the socket's heartbeat alive.
pub(crate) async fn chat_event(socket: &mut Socket, wait: Duration) -> Value {
    tokio::time::timeout(wait, async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => {
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    match frame["type"].as_str() {
                        Some("event") => return frame["event"].clone(),
                        Some("hello" | "subscribed") => {}
                        Some("heartbeat") => socket
                            .send(Message::Text(
                                json!({"type":"heartbeat","activityAgeMs":0})
                                    .to_string()
                                    .into(),
                            ))
                            .await
                            .unwrap(),
                        _ => return frame,
                    }
                }
                Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
                frame => panic!("unexpected frame {frame:?}"),
            }
        }
    })
    .await
    .expect("gateway event timeout")
}

/// Waits for the gateway to end the chat subscription. Without new events it
/// rechecks access every ten seconds, so allow a full interval plus slack.
pub(crate) async fn chat_revoked(socket: &mut Socket) -> Value {
    let frame = chat_event(socket, Duration::from_secs(15)).await;
    assert_eq!(
        frame["type"], "error",
        "subscription was not revoked: {frame}"
    );
    assert_eq!(frame["id"], CHAT);
    frame
}
