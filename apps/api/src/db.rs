use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
};
use std::{str::FromStr, time::Duration};

/// Migrates through the direct connection, then connects using the application role.
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
    // Validate runtime configuration before performing any schema changes.
    connect_options(&url)?;
    migrate_database().await?;
    let pool = connect(&url).await?;
    Ok(Some(pool))
}

/// Startup and explicit migration command; never falls back to the application URL.
pub async fn migrate_database() -> Result<(), String> {
    let url = std::env::var("MIGRATION_DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or("MIGRATION_DATABASE_URL is required to run migrations")?;
    let options =
        connect_options(&url).map_err(|_| "MIGRATION_DATABASE_URL must be a PostgreSQL URL")?;
    migrate_database_with(options).await
}

async fn migrate_database_with(mut options: PgConnectOptions) -> Result<(), String> {
    // SQLx migrations need a session-level advisory lock, not transaction pooling.
    if options.get_port() == 6432 {
        return Err(
            "MIGRATION_DATABASE_URL must use a direct connection, not pooled port 6432".into(),
        );
    }
    // The migration role or database may have a schema first in its search path.
    // Pin only this short-lived direct connection; PlanetScale's runtime pool must
    // not receive startup parameters through PgBouncer.
    options = options.options([("search_path", "public")]);
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

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Executor;
    use uuid::Uuid;

    #[tokio::test]
    #[ignore = "requires disposable Postgres DATABASE_URL"]
    async fn migrations_pin_public_despite_username_schema_and_database_search_path() {
        let base_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must point at disposable Postgres");
        let admin_options = PgConnectOptions::from_str(&base_url).unwrap();
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(admin_options.clone())
            .await
            .unwrap();
        let database = format!("caper_migration_{}", Uuid::new_v4().simple());
        admin
            .execute(format!(r#"CREATE DATABASE "{database}""#).as_str())
            .await
            .unwrap();

        let migration_options = admin_options.clone().database(&database);
        let setup = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(migration_options.clone())
            .await
            .unwrap();
        let username_schema: String = sqlx::query_scalar("SELECT quote_ident(current_user)")
            .fetch_one(&setup)
            .await
            .unwrap();
        setup
            .execute(format!("CREATE SCHEMA {username_schema}").as_str())
            .await
            .unwrap();
        setup.execute("CREATE SCHEMA custom").await.unwrap();
        setup.close().await;

        // The default "$user", public path would create tables in the user schema.
        migrate_database_with(migration_options.clone())
            .await
            .unwrap();
        let verify = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(migration_options.clone())
            .await
            .unwrap();
        let locations: Vec<(String, String)> = sqlx::query_as(
            "SELECT schemaname, tablename FROM pg_tables
             WHERE tablename IN ('users', '_sqlx_migrations')
             ORDER BY tablename, schemaname",
        )
        .fetch_all(&verify)
        .await
        .unwrap();
        assert_eq!(
            locations,
            vec![
                ("public".into(), "_sqlx_migrations".into()),
                ("public".into(), "users".into()),
            ]
        );
        let user_id: i64 = sqlx::query_scalar(
            "INSERT INTO public.users (external_id) VALUES ('migration-rerun-data') RETURNING id",
        )
        .fetch_one(&verify)
        .await
        .unwrap();
        // Exclude public entirely for the second connection and shadow users.
        // Migration must still reuse the public ledger; runtime must target public.users.
        sqlx::query("CREATE TABLE custom.users (id bigint)")
            .execute(&verify)
            .await
            .unwrap();
        verify
            .execute(format!(r#"ALTER DATABASE "{database}" SET search_path TO custom"#).as_str())
            .await
            .unwrap();
        let ledger_entries: i64 =
            sqlx::query_scalar("SELECT count(*) FROM public._sqlx_migrations")
                .fetch_one(&verify)
                .await
                .unwrap();
        verify.close().await;

        migrate_database_with(migration_options).await.unwrap();
        let verify = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(admin_options.database(&database))
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT current_schema()")
                .fetch_one(&verify)
                .await
                .unwrap(),
            "custom"
        );
        let profile = crate::accounts::set_profile(&verify, user_id, "schema_test", "Schema Test")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(profile.username.as_deref(), Some("schema_test"));
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM pg_tables WHERE tablename = '_sqlx_migrations' AND schemaname <> 'public'",
            )
            .fetch_one(&verify)
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM public._sqlx_migrations")
                .fetch_one(&verify)
                .await
                .unwrap(),
            ledger_entries
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM public.users WHERE external_id = 'migration-rerun-data'",
            )
            .fetch_one(&verify)
            .await
            .unwrap(),
            1
        );
        verify.close().await;

        admin
            .execute(format!(r#"DROP DATABASE "{database}""#).as_str())
            .await
            .unwrap();
    }
}
