use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
};
use std::{str::FromStr, time::Duration};

/// Connects to PlanetScale Postgres and applies embedded migrations.
///
/// `DATABASE_URL` is optional so local and image checks still boot without a
/// database. When it is set, startup fails closed if the URL, TLS, or
/// migrations cannot be applied.
///
/// sqlx takes a Postgres advisory lock while migrating, so concurrent pods wait
/// and apply each file once. Use a direct primary URL (port `5432`), not
/// PgBouncer (`6432`): transaction pooling drops session locks between
/// statements.
pub async fn connect_database() -> Result<Option<PgPool>, String> {
    let Some(url) = std::env::var("DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        tracing::info!("DATABASE_URL unset; starting without a database");
        return Ok(None);
    };
    let pool = connect(&url).await?;
    migrate(&pool).await?;
    Ok(Some(pool))
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
