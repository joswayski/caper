use std::process::ExitCode;

use caper_media_worker::{local, worker::Worker};
use lambda_runtime::{LambdaEvent, service_fn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if args.first().map(String::as_str) == Some("process") {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_ansi(false)
            .init();
        return match local::run(&args[1..]).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    if !args.is_empty() {
        eprintln!("{}", local::USAGE);
        return ExitCode::from(2);
    }

    // Lambda: one JSON line per event in CloudWatch.
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_current_span(false)
        .without_time()
        .init();
    // Cold start: read the secret once, then serve invocations.
    let worker = match Worker::from_env().await {
        Ok(worker) => worker,
        Err(error) => {
            tracing::error!(error = %error, "media worker configuration failed");
            return ExitCode::FAILURE;
        }
    };
    let worker = &worker;
    let handler =
        service_fn(move |event: LambdaEvent<_>| async move { worker.handle(event).await });
    match lambda_runtime::run(handler).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, "lambda runtime stopped");
            ExitCode::FAILURE
        }
    }
}
