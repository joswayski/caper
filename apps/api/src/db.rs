use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
};
use std::{str::FromStr, time::Duration};

/// Connects using the application role. Never runs migrations at startup.
///
/// `DATABASE_URL` is optional so local and image checks still boot without a
/// database. Use PlanetScale's pooled port 6432 for application queries.
pub async fn connect_database() -> Result<Option<PgPool>, String> {
    let Some(url) = std::env::var("DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        tracing::info!("DATABASE_URL unset; starting without a database");
        return Ok(None);
    };
    let pool = connect(&url).await?;
    Ok(Some(pool))
}

/// Explicit migration command only; never falls back to the application URL.
pub async fn migrate_database() -> Result<(), String> {
    let url = std::env::var("MIGRATION_DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or("MIGRATION_DATABASE_URL is required for --migrate")?;
    let options =
        connect_options(&url).map_err(|_| "MIGRATION_DATABASE_URL must be a PostgreSQL URL")?;
    // SQLx migrations need a session-level advisory lock, not transaction pooling.
    if options.get_port() == 6432 {
        return Err(
            "MIGRATION_DATABASE_URL must use a direct connection, not pooled port 6432".into(),
        );
    }
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await
        .map_err(|_| "failed to connect to MIGRATION_DATABASE_URL")?;
    let result = migrate(&pool).await;
    pool.close().await;
    result
}

pub(crate) fn connect_options(url: &str) -> Result<PgConnectOptions, String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err("DATABASE_URL must be a PostgreSQL URL".into());
    }
    let scheme = trimmed.split(':').next().unwrap_or_default();
    if scheme != "postgres" && scheme != "postgresql" {
        return Err("DATABASE_URL must be a PostgreSQL URL".into());
    }
    let mut options = PgConnectOptions::from_str(trimmed)
        .map_err(|_| "DATABASE_URL must be a PostgreSQL URL".to_string())?;
    if !matches!(options.get_ssl_mode(), PgSslMode::VerifyFull) {
        options = options.ssl_mode(PgSslMode::VerifyFull);
    }
    Ok(options)
}

async fn connect(url: &str) -> Result<PgPool, String> {
    let options = connect_options(url)?;
    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await
        .map_err(|_| "failed to connect to DATABASE_URL".to_string())
}

async fn migrate(pool: &PgPool) -> Result<(), String> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|_| "database migration failed".to_string())?;
    tracing::info!("database migrations applied");
    Ok(())
}
