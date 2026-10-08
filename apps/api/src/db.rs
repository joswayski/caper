use crate::RuntimeEnvironment;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
};
use std::{str::FromStr, time::Duration};

/// Migrates through the direct connection, then connects using the application role.
///
/// `DATABASE_URL` is optional so local and image checks still boot without a
/// database. Use PlanetScale's pooled port 6432 for application queries.
pub async fn connect_database(environment: &RuntimeEnvironment) -> Result<Option<PgPool>, String> {
    let Some(url) = environment
        .get("DATABASE_URL")
        .filter(|value| !value.trim().is_empty())
    else {
        tracing::info!("DATABASE_URL unset; starting without a database");
        return Ok(None);
    };
    // Validate runtime configuration before performing any schema changes.
    connect_options(&url)?;
    migrate_database(environment).await?;
    let pool = connect(&url).await?;
    Ok(Some(pool))
}

/// Startup and explicit migration command; never falls back to the application URL.
pub async fn migrate_database(environment: &RuntimeEnvironment) -> Result<(), String> {
    let url = environment
        .get("MIGRATION_DATABASE_URL")
        .filter(|value| !value.trim().is_empty())
        .ok_or("MIGRATION_DATABASE_URL is required to run migrations")?;
    let options =
        connect_options(&url).map_err(|_| "MIGRATION_DATABASE_URL must be a PostgreSQL URL")?;
    validate_migration_options(&options)?;
    let runtime_role = runtime_role_from_env(environment).await?;
    migrate_database_with(options, runtime_role.as_deref()).await
}

async fn runtime_role_from_env(environment: &RuntimeEnvironment) -> Result<Option<String>, String> {
    let Some(url) = environment
        .get("DATABASE_URL")
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
    let local = matches!(options.get_host(), "localhost" | "127.0.0.1" | "::1")
        || (allow_insecure && options.get_host() == "postgres");
    if !local && !matches!(options.get_ssl_mode(), PgSslMode::VerifyFull) {
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

/// Gateway replicas use only the runtime role and never run migrations.
pub async fn connect_runtime_database(environment: &RuntimeEnvironment) -> Result<PgPool, String> {
    let url = environment
        .get("DATABASE_URL")
        .ok_or("DATABASE_URL is required")?;
    connect(&url).await
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
        format!(
            "GRANT SELECT, INSERT, UPDATE ON public.spaces, public.channels, public.chat_sessions, public.messages, public.channel_events TO {role}"
        ),
        format!("GRANT SELECT, INSERT ON public.message_versions TO {role}"),
        format!(
            "GRANT SELECT, INSERT, UPDATE, DELETE ON public.message_reactions, public.message_reaction_activity, public.message_pin_activity TO {role}"
        ),
        format!("GRANT SELECT, INSERT, UPDATE ON public.assets TO {role}"),
        format!(
            "GRANT SELECT, INSERT, UPDATE, DELETE ON public.space_members, public.channel_members, public.channel_joins TO {role}"
        ),
        format!(
            "GRANT SELECT, INSERT, UPDATE ON public.space_invitations, public.space_invite_limits, public.channel_invitations TO {role}"
        ),
        format!(
            "GRANT SELECT, INSERT, UPDATE, DELETE ON public.direct_conversations, public.direct_reads, public.push_devices, public.push_notifications, public.push_deliveries TO {role}"
        ),
        format!("GRANT SELECT, INSERT, UPDATE ON public.user_blocks TO {role}"),
        format!(
            "GRANT USAGE ON SEQUENCE public.push_devices_id_seq, public.push_notifications_id_seq TO {role}"
        ),
        format!(
            "GRANT USAGE ON SEQUENCE public.spaces_id_seq, public.channels_id_seq, public.chat_sessions_id_seq, public.messages_id_seq, public.message_reaction_activity_id_seq, public.message_reactions_id_seq, public.space_members_id_seq, public.channel_members_id_seq, public.channel_joins_id_seq, public.message_pin_activity_id_seq, public.assets_id_seq TO {role}"
        ),
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
        assert!(matches!(
            connect_options_with(
                "postgres://user:password@hosted.example/caperchat?sslmode=disable",
                true
            )
            .unwrap()
            .get_ssl_mode(),
            PgSslMode::VerifyFull
        ));
    }

    #[test]
    fn embedded_migration_versions_are_unique_and_increasing() {
        let versions: Vec<_> = sqlx::migrate!("./migrations")
            .iter()
            .map(|migration| migration.version)
            .collect();
        assert!(
            versions.windows(2).all(|pair| pair[0] < pair[1]),
            "migration versions must be unique and increasing: {versions:?}"
        );
    }

    #[test]
    fn runtime_role_is_quoted_as_a_postgres_identifier() {
        assert_eq!(quote_identifier("caper-runtime"), r#""caper-runtime""#);
        assert_eq!(quote_identifier("quoted\"role"), r#""quoted""role""#);
    }

    #[sqlx::test(migrations = false)]
    #[ignore = "requires disposable Postgres DATABASE_URL"]
    async fn dm_migrations_upgrade_channel_joining_without_rewriting_history(pool: PgPool) {
        let mut before = sqlx::migrate!("./migrations");
        before.migrations = std::borrow::Cow::Owned(
            before
                .iter()
                .filter(|migration| migration.version < 202610030001)
                .cloned()
                .collect(),
        );
        before.run(&pool).await.unwrap();
        let ledger: Vec<(i64, Vec<u8>)> =
            sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations ORDER BY version")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(ledger.last().unwrap().0, 202610010001);
        pool.execute(
            "INSERT INTO users(external_id) VALUES('upgrade-owner'),('upgrade-peer');
             INSERT INTO spaces(external_id,name,owner_id)
                SELECT 'upgrade-space','Retained space',id FROM users WHERE external_id='upgrade-owner';
             INSERT INTO channels(external_id,space_id,name,last_seq)
                SELECT 'upgrade-room',id,'general',1 FROM spaces WHERE external_id='upgrade-space';
             INSERT INTO channel_joins(channel_id,user_id)
                SELECT c.id,u.id FROM channels c,users u
                WHERE c.external_id='upgrade-room' AND u.external_id='upgrade-owner';
             INSERT INTO space_invitations(space_id,user_id,status)
                SELECT s.id,u.id,'pending' FROM spaces s,users u
                WHERE s.external_id='upgrade-space' AND u.external_id='upgrade-peer';
             INSERT INTO chat_sessions(external_id,token_hash,name)
                VALUES('upgrade-chat',decode('01','hex'),'Retained author');
             INSERT INTO messages(external_id,channel_id,session_id,client_message_id,request_hash,channel_seq,payload)
                SELECT 'upgrade-msg',c.id,s.id,'00000000-0000-4000-8000-000000000001',decode('02','hex'),1,
                    '{\"content\":{\"text\":\"Retained message\"}}'::jsonb
                FROM channels c,chat_sessions s
                WHERE c.external_id='upgrade-room' AND s.external_id='upgrade-chat';",
        )
        .await
        .unwrap();
        let after = sqlx::migrate!("./migrations");
        after.run(&pool).await.unwrap();
        after.run(&pool).await.unwrap();
        assert_eq!(
            sqlx::query_as::<_, (i64, Vec<u8>)>(
                "SELECT version,checksum FROM _sqlx_migrations WHERE version < 202610030001 ORDER BY version",
            )
            .fetch_all(&pool)
            .await
            .unwrap(),
            ledger,
            "applied versions and checksums must not change",
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT payload->'content'->>'text' FROM messages WHERE external_id='upgrade-msg'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            "Retained message",
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM channel_joins")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1,
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM space_invitations WHERE status='pending'",
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1,
        );
        pool.execute(
            "INSERT INTO channels(external_id,name,private) VALUES('upgrade-dm','Direct',true)",
        )
        .await
        .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM push_devices")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0,
        );
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
            ("public.space_members", "UPDATE"),
            ("public.channel_joins", "UPDATE"),
            ("public.message_reactions", "UPDATE"),
            ("public.message_versions", "SELECT"),
            ("public.message_versions", "INSERT"),
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
        for privilege in ["UPDATE", "DELETE"] {
            assert!(
                !sqlx::query_scalar::<_, bool>("SELECT has_table_privilege($1, $2, $3)")
                    .bind(&runtime_role)
                    .bind("public.message_versions")
                    .bind(privilege)
                    .fetch_one(&verify)
                    .await
                    .unwrap(),
                "retained message versions must be append-only for {runtime_role}"
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
