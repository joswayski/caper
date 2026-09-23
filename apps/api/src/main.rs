use caper_api::{
    Cloudflare, Config, RuntimeEnvironment, app, connect_database, shutdown_cleanup, spawn_cleanup,
};
use std::{future::IntoFuture, sync::Arc, time::Duration};

// Leave 20 seconds for provider cleanup and a margin inside Kubernetes' 60s grace.
const HTTP_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

mod telemetry;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    // AWS SDK and HTTP/SQL clients enable both Rustls crypto backends. Redis
    // uses the process default, so select it before constructing any TLS client.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Rustls crypto provider must be installed once at startup");
    let environment = match RuntimeEnvironment::load().await {
        Ok(environment) => environment,
        Err(error) => {
            eprintln!("API configuration failed: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let telemetry = telemetry::init(&environment);
    if environment.secret_loaded() {
        tracing::info!("application settings loaded from Secrets Manager");
    }
    tracing::info!(event_name = "service_starting", "media API starting");
    let result = run(&environment).await;
    if let Err(error) = &result {
        tracing::error!(
            event_name = "service_failed",
            error,
            "media API stopped with an error"
        );
    } else {
        tracing::info!(event_name = "service_stopped", "media API stopped");
    }
    // HTTP drain (30s) + provider cleanup (20s) + log flush (3s) fit the 60s pod grace.
    telemetry.shutdown();
    if result.is_ok() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

async fn run(environment: &RuntimeEnvironment) -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    match (args.next().as_deref(), args.next()) {
        (Some("--migrate"), None) => return caper_api::migrate_database(environment).await,
        (Some("--gateway"), None) => return run_gateway(environment).await,
        (None, None) => {}
        _ => return Err("usage: caper-api [--migrate|--gateway]".into()),
    }
    let config = Config::from_env(environment)?;
    let bind = config.bind;
    let database = connect_database(environment).await?;
    let mut state =
        caper_api::AppState::with_database(config, Arc::new(Cloudflare::new()), database);
    state.enable_accounts_from_env(environment).await?;
    state.enable_shared_media(environment).await?;
    state.enable_chat(environment).await?;
    spawn_cleanup(state.clone());
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|_| "could not bind media API".to_owned())?;
    tracing::info!(%bind, "media API listening");
    let shutdown_state = state.clone();
    serve_with_drain(listener, app(state), async {
        shutdown_signal().await;
        shutdown_state.begin_shutdown();
    })
    .await
    .map_err(|_| "media API serving or HTTP draining failed".to_owned())?;
    shutdown_cleanup(&shutdown_state).await;
    Ok(())
}

async fn run_gateway(environment: &RuntimeEnvironment) -> Result<(), String> {
    let bind = environment
        .get("CHAT_GATEWAY_BIND")
        .unwrap_or_else(|| "0.0.0.0:3002".into());
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .map_err(|_| "could not bind chat gateway")?;
    if !environment
        .get("CHAT_ENABLED")
        .is_some_and(|v| v == "true" || v == "1")
    {
        let router = axum::Router::new()
            .route(
                "/health",
                axum::routing::get(|| async { axum::http::StatusCode::NO_CONTENT }),
            )
            .fallback(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE });
        return serve_with_drain(listener, router, shutdown_signal())
            .await
            .map_err(|_| "disabled gateway drain failed".into());
    }
    let pool = caper_api::connect_runtime_database(environment).await?;
    let state = caper_api::gateway::Gateway::from_env(Some(&pool), environment).await?;
    state.start();
    tracing::info!(
        event_name = "chat_gateway_listening",
        "chat gateway listening"
    );
    serve_with_drain(listener, caper_api::gateway::router(state.clone()), async {
        shutdown_signal().await;
        state.begin_shutdown();
        // Upgraded WebSockets outlive Axum's HTTP serve future. Keep the runtime
        // alive through both handoff and accepted commands (including commands
        // detached from a lost socket). Readiness and new commands fail now.
        tokio::time::sleep(
            caper_api::gateway::COMMAND_TIMEOUT.max(caper_api::gateway::HANDOFF_WINDOW)
                + Duration::from_secs(1),
        )
        .await;
    })
    .await
    .map_err(|_| "chat gateway drain failed".to_owned())
}

async fn serve_with_drain(
    listener: tokio::net::TcpListener,
    router: axum::Router,
    shutdown: impl std::future::Future<Output = ()>,
) -> std::io::Result<()> {
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .into_future();
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => return result,
        () = shutdown => {},
    }
    let _ = stop.send(());
    tracing::info!("draining media API requests");
    match tokio::time::timeout(HTTP_DRAIN_TIMEOUT, server).await {
        Ok(result) => result,
        Err(_) => {
            // Axum's connection tasks can outlive the serve future. Do not run
            // registry cleanup concurrently with requests that failed to drain.
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "HTTP drain deadline reached; terminating without provider cleanup",
            ))
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use tokio::sync::{Notify, oneshot};

    #[tokio::test]
    async fn shutdown_drains_an_in_flight_request() {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let router = axum::Router::new().route(
            "/slow",
            get({
                let entered = entered.clone();
                let release = release.clone();
                move || async move {
                    entered.notify_one();
                    release.notified().await;
                    "finished"
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel();
        let server = tokio::spawn(serve_with_drain(listener, router, async {
            stopped.await.unwrap();
        }));
        let request = tokio::spawn(async move {
            reqwest::get(format!("http://{address}/slow"))
                .await
                .unwrap()
                .text()
                .await
                .unwrap()
        });
        entered.notified().await;
        stop.send(()).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !server.is_finished(),
            "shutdown must wait for the active request"
        );
        release.notify_one();
        assert_eq!(request.await.unwrap(), "finished");
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            reqwest::get(format!("http://{address}/slow"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn shutdown_deadline_skips_cleanup_for_a_stuck_request() {
        let entered = Arc::new(Notify::new());
        let router = axum::Router::new().route(
            "/stuck",
            get({
                let entered = entered.clone();
                move || async move {
                    entered.notify_one();
                    std::future::pending::<()>().await;
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel();
        let server = tokio::spawn(serve_with_drain(listener, router, async {
            stopped.await.unwrap();
        }));
        let request = tokio::spawn(reqwest::get(format!("http://{address}/stuck")));
        entered.notified().await;
        stop.send(()).unwrap();
        let error = tokio::time::timeout(HTTP_DRAIN_TIMEOUT + Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        request.abort();
    }
}
