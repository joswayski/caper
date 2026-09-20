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
    validate_migration_options(&options)?;
    let runtime_role = runtime_role_from_env().await?;
    migrate_database_with(options, runtime_role.as_deref()).await
}

async fn runtime_role_from_env() -> Result<Option<String>, String> {
    let Some(url) = std::env::var("DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(None);
    };
    let pool = connect(&url).await?;
    let role = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&pool)
        .await
        .map_err(|_| "failed to identify the database runtime role")?;
    pool.close().await;
    Ok(Some(role))
}

async fn migrate_database_with(
    mut options: PgConnectOptions,
    runtime_role: Option<&str>,
) -> Result<(), String> {
    validate_migration_options(&options)?;
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
    let result = async {
        migrate(&pool).await?;
        if let Some(runtime_role) = runtime_role {
            grant_runtime_access(&pool, runtime_role).await?;
        }
        Ok(())
    }
    .await;
    pool.close().await;
    result
}

fn validate_migration_options(options: &PgConnectOptions) -> Result<(), String> {
    // SQLx migrations need a session-level advisory lock, not transaction pooling.
    if options.get_port() == 6432 {
        return Err(
            "MIGRATION_DATABASE_URL must use a direct connection, not pooled port 6432".into(),
        );
    }
    Ok(())
}

pub(crate) fn connect_options(url: &str) -> Result<PgConnectOptions, String> {
    let allow_insecure = std::env::var("DATABASE_ALLOW_INSECURE")
        .ok()
        .is_some_and(|value| value == "true" || value == "1");
    connect_options_with(url, allow_insecure)
}

fn connect_options_with(url: &str, allow_insecure: bool) -> Result<PgConnectOptions, String> {
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
    let loopback = matches!(options.get_host(), "localhost" | "127.0.0.1" | "::1");
    if !loopback && !allow_insecure && !matches!(options.get_ssl_mode(), PgSslMode::VerifyFull) {
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

async fn grant_runtime_access(pool: &PgPool, runtime_role: &str) -> Result<(), String> {
    let role = quote_identifier(runtime_role);
    for statement in [
        format!("GRANT USAGE ON SCHEMA public TO {role}"),
        format!("GRANT SELECT, INSERT, UPDATE ON public.users TO {role}"),
        format!(
            "GRANT SELECT, INSERT, UPDATE, DELETE ON public.auth_email_challenges, public.account_sessions TO {role}"
        ),
        format!("GRANT USAGE ON SEQUENCE public.users_id_seq TO {role}"),
    ] {
        sqlx::query(&statement)
            .execute(pool)
            .await
            .map_err(|_| "failed to grant database access to the runtime role")?;
    }
    tracing::info!("database runtime grants applied");
    Ok(())
}

fn quote_identifier(value: &str) -> String {
    format!(r#""{}""#, value.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Executor;
    use uuid::Uuid;

    #[test]
    fn non_loopback_database_requires_verified_tls_unless_explicitly_allowed() {
        let url = "postgres://user:password@postgres:5432/caperchat?sslmode=disable";

        assert!(matches!(
            connect_options_with(url, false).unwrap().get_ssl_mode(),
            PgSslMode::VerifyFull
        ));
        assert!(matches!(
            connect_options_with(url, true).unwrap().get_ssl_mode(),
            PgSslMode::Disable
        ));
    }

    #[test]
    fn runtime_role_is_quoted_as_a_postgres_identifier() {
        assert_eq!(quote_identifier("caper-runtime"), r#""caper-runtime""#);
        assert_eq!(quote_identifier("quoted\"role"), r#""quoted""role""#);
    }

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
        let runtime_role = format!("caper_runtime_{}", Uuid::new_v4().simple());
        admin
            .execute(format!(r#"CREATE DATABASE "{database}""#).as_str())
            .await
            .unwrap();
        admin
            .execute(format!(r#"CREATE ROLE "{runtime_role}""#).as_str())
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
        migrate_database_with(migration_options.clone(), Some(&runtime_role))
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
        for (object, privilege) in [
            ("public.users", "SELECT"),
            ("public.users", "INSERT"),
            ("public.users", "UPDATE"),
            ("public.auth_email_challenges", "DELETE"),
            ("public.account_sessions", "DELETE"),
        ] {
            assert!(
                sqlx::query_scalar::<_, bool>("SELECT has_table_privilege($1, $2, $3)")
                    .bind(&runtime_role)
                    .bind(object)
                    .bind(privilege)
                    .fetch_one(&verify)
                    .await
                    .unwrap(),
                "{runtime_role} lacks {privilege} on {object}"
            );
        }
        assert!(
            sqlx::query_scalar::<_, bool>("SELECT has_sequence_privilege($1, $2, $3)")
                .bind(&runtime_role)
                .bind("public.users_id_seq")
                .bind("USAGE")
                .fetch_one(&verify)
                .await
                .unwrap()
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

        migrate_database_with(migration_options, Some(&runtime_role))
            .await
            .unwrap();
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
        admin
            .execute(format!(r#"DROP ROLE "{runtime_role}""#).as_str())
            .await
            .unwrap();
    }
}
