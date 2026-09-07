//! Run explicitly against disposable Postgres: see docs/media.md.
use caper_api::accounts::*;
use sqlx::PgPool;

async fn login(pool: &PgPool, email: &str, token: u8) -> User {
    let challenge = new_public_id();
    assert!(
        issue_challenge(pool, email, &challenge, &[7; 32])
            .await
            .unwrap()
    );
    complete_login(pool, &challenge, &[7; 32], &[token; 32])
        .await
        .unwrap()
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable Postgres DATABASE_URL"]
async fn profiles_and_identity(pool: PgPool) {
    let user = login(&pool, " Jose+test@Example.COM ", 1).await;
    assert_eq!(user.email, "jose+test@example.com");
    assert_eq!(user.public_id.len(), 21);
    assert!(user.id > 0 && user.username.is_none());
    let profile = set_profile(&pool, user.id, " Jose_1 ", " José 🌱 ")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(profile.username.as_deref(), Some("jose_1"));
    assert_eq!(profile.display_name.as_deref(), Some("José 🌱"));
    assert_eq!(profile.public_id, user.public_id);
    let other = login(&pool, "other@example.com", 2).await;
    assert_ne!(other.public_id, user.public_id);
    assert!(
        set_profile(&pool, other.id, "JOSE_1", "Other")
            .await
            .is_err()
    );
    for username in ["ab", "with-hyphen", "UPPER SPACE", &"a".repeat(33)] {
        assert!(
            set_profile(&pool, other.id, username, "Other")
                .await
                .is_err()
        );
    }
    for display in ["", "  ", &"🌱".repeat(65)] {
        assert!(
            set_profile(&pool, other.id, "other", display)
                .await
                .is_err()
        );
    }
    // Display names are non-unique and measured in characters, not UTF-8 bytes.
    assert!(
        set_profile(&pool, other.id, &"a".repeat(32), "José 🌱")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        set_profile(&pool, other.id, "other", &"🌱".repeat(64))
            .await
            .unwrap()
            .is_some()
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable Postgres DATABASE_URL"]
async fn concurrent_single_use_and_resend(pool: PgPool) {
    let challenge = new_public_id();
    let (a, b) = tokio::join!(
        issue_challenge(&pool, "a@example.com", &challenge, &[7; 32]),
        issue_challenge(&pool, "A@example.com", &challenge, &[7; 32]),
    );
    assert_ne!(a.unwrap(), b.unwrap());
    let (a, b) = tokio::join!(
        complete_login(&pool, &challenge, &[7; 32], &[1; 32]),
        complete_login(&pool, &challenge, &[7; 32], &[2; 32]),
    );
    assert_ne!(a.unwrap().is_some(), b.unwrap().is_some());
    assert!(
        !issue_challenge(&pool, "a@example.com", &new_public_id(), &[8; 32])
            .await
            .unwrap()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable Postgres DATABASE_URL"]
async fn attempt_limit_and_expiry(pool: PgPool) {
    let challenge = new_public_id();
    issue_challenge(&pool, "a@example.com", &challenge, &[7; 32])
        .await
        .unwrap();
    // Concurrent wrong guesses cannot lose increments or exceed the limit.
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let pool = pool.clone();
        let challenge = challenge.clone();
        tasks.push(tokio::spawn(async move {
            complete_login(&pool, &challenge, &[9; 32], &[1; 32])
                .await
                .unwrap()
                .is_none()
        }));
    }
    for task in tasks {
        assert!(task.await.unwrap());
    }
    let attempts: i16 = sqlx::query_scalar("SELECT attempts FROM login_challenges")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(attempts, 5);
    assert!(
        complete_login(&pool, &challenge, &[7; 32], &[1; 32])
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE login_challenges SET created_at = now() - interval '11 minutes', expires_at = now() - interval '1 minute', attempts = 0")
        .execute(&pool).await.unwrap();
    assert!(
        complete_login(&pool, &challenge, &[7; 32], &[1; 32])
            .await
            .unwrap()
            .is_none()
    );
    let replacement = new_public_id();
    assert!(
        issue_challenge(&pool, "a@example.com", &replacement, &[8; 32])
            .await
            .unwrap()
    );
    assert!(
        complete_login(&pool, &challenge, &[7; 32], &[1; 32])
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        complete_login(&pool, &replacement, &[8; 32], &[1; 32])
            .await
            .unwrap()
            .is_some()
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable Postgres DATABASE_URL"]
async fn sessions_relogin_revocation_and_cleanup(pool: PgPool) {
    let user = login(&pool, "a@example.com", 1).await;
    sqlx::query("UPDATE login_challenges SET created_at = now() - interval '61 seconds'")
        .execute(&pool)
        .await
        .unwrap();
    let same = login(&pool, "A@example.com", 2).await;
    assert_eq!(same.id, user.id);
    assert_eq!(
        session_user(&pool, &[1; 32]).await.unwrap().unwrap().id,
        user.id
    );
    revoke_session(&pool, &[1; 32]).await.unwrap();
    assert!(session_user(&pool, &[1; 32]).await.unwrap().is_none());
    assert!(session_user(&pool, &[2; 32]).await.unwrap().is_some());
    revoke_all_sessions(&pool, user.id).await.unwrap();
    assert!(session_user(&pool, &[2; 32]).await.unwrap().is_none());
    login(&pool, "other@example.com", 3).await;
    sqlx::query("UPDATE sessions SET created_at = now() - interval '31 days', expires_at = now() - interval '1 day'")
        .execute(&pool).await.unwrap();
    assert!(session_user(&pool, &[3; 32]).await.unwrap().is_none());
    sqlx::query("UPDATE login_challenges SET created_at = now() - interval '11 minutes', expires_at = now() - interval '1 minute'")
        .execute(&pool).await.unwrap();
    delete_expired(&pool).await.unwrap();
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM sessions), (SELECT count(*) FROM login_challenges)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(counts, (0, 0));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable Postgres DATABASE_URL"]
async fn failure_rolls_back_consumption_and_account_creation(pool: PgPool) {
    login(&pool, "a@example.com", 1).await;
    let challenge = new_public_id();
    issue_challenge(&pool, "b@example.com", &challenge, &[7; 32])
        .await
        .unwrap();
    // Deliberate duplicate session digest forces an insert failure.
    assert!(
        complete_login(&pool, &challenge, &[7; 32], &[1; 32])
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert!(
        complete_login(&pool, &challenge, &[7; 32], &[2; 32])
            .await
            .unwrap()
            .is_some()
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable Postgres DATABASE_URL"]
async fn database_constraints_and_cascade(pool: PgPool) {
    let user = login(&pool, "a@example.com", 1).await;
    assert!(
        sqlx::query(
            "INSERT INTO users (public_id, email) SELECT public_id, 'b@example.com' FROM users"
        )
        .execute(&pool)
        .await
        .is_err()
    );
    assert!(
        sqlx::query("UPDATE users SET email = 'A@example.com'")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE users SET username = 'valid'")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE sessions SET token_hash = decode('00', 'hex')")
            .execute(&pool)
            .await
            .is_err()
    );
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(session_user(&pool, &[1; 32]).await.unwrap().is_none());
    // Applying embedded migrations again is a no-op, as on replica startup.
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
}
