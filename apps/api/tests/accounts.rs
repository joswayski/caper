//! Run explicitly against a disposable Postgres database.
use caper_api::accounts::*;
use chrono::{TimeZone, Utc};
use sqlx::PgPool;

fn provider_time(hour: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 7, hour, 0, 0).unwrap()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable Postgres DATABASE_URL"]
async fn workos_identity_sync_and_profiles(pool: PgPool) {
    let user = sync_workos_user(
        &pool,
        "user_ABC123",
        " Jose+test@Example.COM ",
        provider_time(10),
    )
    .await
    .unwrap();
    assert!(user.id > 0);
    assert_eq!(user.external_id.len(), 21);
    assert_ne!(user.external_id, user.id.to_string());
    assert_eq!(user.email.as_deref(), Some("jose+test@example.com"));
    assert!(!user.onboarded());

    let same = sync_workos_user(
        &pool,
        "user_ABC123",
        "changed@example.com",
        provider_time(11),
    )
    .await
    .unwrap();
    assert_eq!(same.id, user.id);
    assert_eq!(same.external_id, user.external_id);
    assert_eq!(same.email.as_deref(), Some("changed@example.com"));

    let profile = set_profile(&pool, user.id, " Jose_1 ", " José 🌱 ")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(profile.username.as_deref(), Some("jose_1"));
    assert_eq!(profile.display_name.as_deref(), Some("José 🌱"));
    assert!(profile.onboarded());
    assert_eq!(
        serde_json::to_value(profile.public()).unwrap(),
        serde_json::json!({"id":user.external_id,"username":"jose_1", "displayName":"José 🌱"})
    );

    let other = sync_workos_user(&pool, "user_OTHER", "other@example.com", provider_time(10))
        .await
        .unwrap();
    assert_ne!(other.external_id, user.external_id);
    assert!(
        set_profile(&pool, other.id, "JOSE_1", "Other")
            .await
            .is_err()
    );
    let renamed = set_profile(&pool, user.id, "jose_new", "José")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(renamed.id, user.id);
    assert_eq!(renamed.external_id, user.external_id);
    assert_eq!(renamed.username.as_deref(), Some("jose_new"));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable Postgres DATABASE_URL"]
async fn identity_constraints_and_concurrency(pool: PgPool) {
    let (a, b) = tokio::join!(
        sync_workos_user(&pool, "user_SAME", "same@example.com", provider_time(10)),
        sync_workos_user(&pool, "user_SAME", "same@example.com", provider_time(10)),
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a.id, b.id);
    assert_eq!(a.external_id, b.external_id);
    assert!(
        sync_workos_user(
            &pool,
            "user_DIFFERENT",
            "same@example.com",
            provider_time(10),
        )
        .await
        .is_err()
    );
    let removed: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT to_regclass('sessions')::text, to_regclass('login_challenges')::text",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(removed, (None, None));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable Postgres DATABASE_URL"]
async fn webhook_ordering_deduplication_and_terminal_deletion(pool: PgPool) {
    let user = sync_workos_user(
        &pool,
        "user_LIFECYCLE",
        "first@example.com",
        provider_time(10),
    )
    .await
    .unwrap();
    let newer = Utc.with_ymd_and_hms(2026, 9, 7, 12, 0, 0).unwrap();
    let older = Utc.with_ymd_and_hms(2026, 9, 7, 11, 0, 0).unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(
        record_workos_event(&mut tx, "event_new", "user.updated")
            .await
            .unwrap()
    );
    apply_workos_update(&mut tx, "user_LIFECYCLE", Some("new@example.com"), newer)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    assert!(
        !record_workos_event(&mut tx, "event_new", "user.updated")
            .await
            .unwrap()
    );
    assert!(
        record_workos_event(&mut tx, "event_old", "user.updated")
            .await
            .unwrap()
    );
    apply_workos_update(&mut tx, "user_LIFECYCLE", Some("stale@example.com"), older)
        .await
        .unwrap();
    apply_workos_deletion(&mut tx, "user_LIFECYCLE", newer)
        .await
        .unwrap();
    apply_workos_update(
        &mut tx,
        "user_LIFECYCLE",
        Some("resurrect@example.com"),
        newer,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let row: (
        Option<String>,
        Option<chrono::DateTime<Utc>>,
        Option<chrono::DateTime<Utc>>,
    ) = sqlx::query_as("SELECT email, email_verified_at, deleted_at FROM users WHERE id = $1")
        .bind(user.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.0, None);
    assert_eq!(row.1, None);
    assert_eq!(row.2, Some(newer));
    assert!(matches!(
        sync_workos_user(&pool, "user_LIFECYCLE", "cached@example.com", older,).await,
        Err(sqlx::Error::RowNotFound)
    ));

    let mut tx = pool.begin().await.unwrap();
    apply_workos_deletion(&mut tx, "user_DELETED_FIRST", older)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(matches!(
        sync_workos_user(&pool, "user_DELETED_FIRST", "later@example.com", newer,).await,
        Err(sqlx::Error::RowNotFound)
    ));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable Postgres DATABASE_URL"]
async fn provider_snapshots_cannot_undo_newer_unverified_email(pool: PgPool) {
    let stale = provider_time(10);
    let webhook_at = provider_time(11);
    let fresh = provider_time(12);
    let user = sync_workos_user(&pool, "user_ORDERED", "verified@example.com", stale)
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    apply_workos_update(&mut tx, "user_ORDERED", None, webhook_at)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert!(matches!(
        sync_workos_user(&pool, "user_ORDERED", "verified@example.com", stale).await,
        Err(sqlx::Error::RowNotFound)
    ));
    let denied: (
        Option<String>,
        Option<chrono::DateTime<Utc>>,
        Option<chrono::DateTime<Utc>>,
    ) = sqlx::query_as(
        "SELECT email, email_verified_at, workos_updated_at FROM users WHERE id = $1",
    )
    .bind(user.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(denied, (None, None, Some(webhook_at)));

    let restored = sync_workos_user(&pool, "user_ORDERED", "fresh@example.com", fresh)
        .await
        .unwrap();
    assert_eq!(restored.email.as_deref(), Some("fresh@example.com"));
    let advanced: (Option<chrono::DateTime<Utc>>,) =
        sqlx::query_as("SELECT workos_updated_at FROM users WHERE id = $1")
            .bind(user.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(advanced.0, Some(fresh));
}
