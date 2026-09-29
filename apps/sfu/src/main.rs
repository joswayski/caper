use caper_sfu::{AppState, Credentials, engine, router, turn::Turn};
use std::{
    env,
    net::{IpAddr, SocketAddr},
    time::Instant,
};
use tokio::net::{TcpListener, UdpSocket};
use tower_http::{limit::RequestBodyLimitLayer, trace::TraceLayer};

fn required(key: &str) -> Result<String, String> {
    optional(key).ok_or_else(|| format!("{key} is required"))
}

fn optional(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn list(key: &str) -> Vec<String> {
    optional(key)
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let filter = tracing_subscriber::EnvFilter::try_from_env("RUST_LOG")
        .unwrap_or_else(|_| "caper_sfu=info,tower_http=info".into());
    if env::var("LOG_FORMAT").is_ok_and(|v| v == "json") {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }

    let http: SocketAddr = optional("SFU_HTTP_BIND")
        .unwrap_or_else(|| "0.0.0.0:8080".into())
        .parse()
        .map_err(|_| "SFU_HTTP_BIND must be a socket address")?;
    let udp_port: u16 = optional("SFU_UDP_PORT")
        .unwrap_or_else(|| "50000".into())
        .parse()
        .map_err(|_| "SFU_UDP_PORT must be a port")?;
    let public_ip: IpAddr = required("SFU_PUBLIC_IP")?
        .parse()
        .map_err(|_| "SFU_PUBLIC_IP must be an IP address")?;
    let max_sessions: usize = optional("SFU_MAX_SESSIONS")
        .unwrap_or_else(|| "1000".into())
        .parse()
        .map_err(|_| "SFU_MAX_SESSIONS must be a number")?;
    let credentials = Credentials {
        app_id: required("SFU_APP_ID")?,
        app_secret: required("SFU_APP_SECRET")?,
        turn_key_id: required("SFU_TURN_KEY_ID")?,
        turn_api_token: required("SFU_TURN_API_TOKEN")?,
    };
    let turn = Turn::new(
        optional("SFU_TURN_SECRET"),
        list("SFU_STUN_URLS"),
        list("SFU_TURN_URLS"),
    );

    let unspecified: IpAddr = if public_ip.is_ipv4() {
        "0.0.0.0".parse().unwrap()
    } else {
        "::".parse().unwrap()
    };
    let socket = UdpSocket::bind(SocketAddr::new(unspecified, udp_port))
        .await
        .map_err(|e| format!("binding UDP {udp_port}: {e}"))?;
    let public = SocketAddr::new(public_ip, udp_port);
    let (commands, receive) = tokio::sync::mpsc::channel(1024);
    let sfu = engine::Sfu::new(public, max_sessions, Instant::now());
    tokio::spawn(engine::run(sfu, socket, receive));

    let app = router(AppState::new(commands, credentials, turn))
        .layer(RequestBodyLimitLayer::new(256 * 1024))
        .layer(TraceLayer::new_for_http());
    let listener = TcpListener::bind(http)
        .await
        .map_err(|e| format!("binding {http}: {e}"))?;
    tracing::info!(%http, %public, "caper-sfu listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|e| e.to_string())
}
