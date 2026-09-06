use caper_api::{Cloudflare, Config, app, shutdown_cleanup, spawn_cleanup};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let config = Config::from_env().unwrap_or_else(|error| {
        eprintln!("configuration error: {error}");
        std::process::exit(2);
    });
    let bind = config.bind;
    let state = caper_api::AppState::new(config, Arc::new(Cloudflare::new()));
    spawn_cleanup(state.clone());
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .expect("bind media API");
    tracing::info!(%bind, "media API listening");
    let shutdown_state = state.clone();
    axum::serve(listener, app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("serve media API");
    shutdown_cleanup(&shutdown_state).await;
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
