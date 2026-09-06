use caper_api::{Cloudflare, Config, app, connect_database, shutdown_cleanup, spawn_cleanup};
use std::{future::IntoFuture, sync::Arc, time::Duration};

// Leave 20 seconds for provider cleanup and a margin inside Kubernetes' 60s grace.
const HTTP_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

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
    let database = connect_database().await.unwrap_or_else(|error| {
        eprintln!("database error: {error}");
        std::process::exit(2);
    });
    let state = caper_api::AppState::with_database(config, Arc::new(Cloudflare::new()), database);
    spawn_cleanup(state.clone());
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .expect("bind media API");
    tracing::info!(%bind, "media API listening");
    let shutdown_state = state.clone();
    serve_with_drain(listener, app(state), shutdown_signal())
        .await
        .expect("serve media API");
    shutdown_cleanup(&shutdown_state).await;
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
