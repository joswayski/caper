//! Run explicitly against a disposable Postgres database.
use caper_api::accounts::*;
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable Postgres DATABASE_URL"]
async fn workos_identity_sync_and_profiles(pool: PgPool) {
    let user = sync_workos_user(&pool, "user_ABC123", " Jose+test@Example.COM ")
        .await
        .unwrap();
    assert!(user.id > 0);
    assert_eq!(user.public_id.len(), 21);
    assert_eq!(user.email, "jose+test@example.com");
    assert!(!user.onboarded());

    let same = sync_workos_user(&pool, "user_ABC123", "changed@example.com")
        .await
        .unwrap();
    assert_eq!(same.id, user.id);
    assert_eq!(same.public_id, user.public_id);
    assert_eq!(same.email, "changed@example.com");

    let profile = set_profile(&pool, user.id, " Jose_1 ", " José 🌱 ")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(profile.username.as_deref(), Some("jose_1"));
    assert_eq!(profile.display_name.as_deref(), Some("José 🌱"));
    assert!(profile.onboarded());

    let other = sync_workos_user(&pool, "user_OTHER", "other@example.com")
        .await
        .unwrap();
    assert!(
        set_profile(&pool, other.id, "JOSE_1", "Other")
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable Postgres DATABASE_URL"]
async fn identity_constraints_and_concurrency(pool: PgPool) {
    let (a, b) = tokio::join!(
        sync_workos_user(&pool, "user_SAME", "same@example.com"),
        sync_workos_user(&pool, "user_SAME", "same@example.com"),
    );
    assert_eq!(a.unwrap().id, b.unwrap().id);
    assert!(
        sync_workos_user(&pool, "user_DIFFERENT", "same@example.com")
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
