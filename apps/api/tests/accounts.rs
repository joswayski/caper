//! Run explicitly against a disposable Postgres database.
use caper_api::accounts::set_profile;
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable Postgres DATABASE_URL"]
async fn provider_neutral_users_and_profiles(pool: PgPool) {
    let columns: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT column_name, is_nullable, column_default
         FROM information_schema.columns
         WHERE table_schema = 'public' AND table_name = 'users'
         ORDER BY ordinal_position",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let email = columns.iter().find(|column| column.0 == "email").unwrap();
    let verified = columns
        .iter()
        .find(|column| column.0 == "email_verified_at")
        .unwrap();
    let deleted = columns
        .iter()
        .find(|column| column.0 == "deleted_at")
        .unwrap();
    let avatar = columns
        .iter()
        .find(|column| column.0 == "avatar_id")
        .unwrap();
    assert_eq!(email.1, "YES");
    assert_eq!(verified.1, "YES");
    assert_eq!(verified.2, None);
    assert_eq!(deleted.1, "YES");
    assert_eq!(avatar.1, "NO");
    assert!(
        avatar
            .2
            .as_deref()
            .is_some_and(|value| value.contains("random"))
    );

    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (external_id) VALUES ('V1StGXR8_Z5jdHi6B-myT') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let profile = set_profile(&pool, user_id, " Caper_Test ", " Caper Friend ")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(profile.username.as_deref(), Some("caper_test"));
    assert_eq!(profile.display_name.as_deref(), Some("Caper Friend"));
    assert_eq!(profile.email, None);
    assert!((0..800).contains(&profile.avatar_id));

    let avatar_before = profile.avatar_id;
    let profile = set_profile(&pool, user_id, "caper_test", "Updated Name")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(profile.avatar_id, avatar_before);

    let avatars: Vec<i16> = sqlx::query_scalar(
        "INSERT INTO users(external_id) SELECT 'avatar-' || n FROM generate_series(1, 64) n RETURNING avatar_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(avatars.iter().all(|avatar| (0..800).contains(avatar)));
    let last_avatar: i16 = sqlx::query_scalar(
        "INSERT INTO users(external_id,avatar_id) VALUES('last-avatar',799) RETURNING avatar_id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(last_avatar, 799);
    assert!(
        sqlx::query("INSERT INTO users(external_id,avatar_id) VALUES('invalid-avatar',800)")
            .execute(&pool)
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable Postgres DATABASE_URL"]
async fn avatar_migration_backfills_existing_profiles(pool: PgPool) {
    sqlx::raw_sql(include_str!("../migrations/202609070001_accounts.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users(external_id,username,display_name) SELECT 'legacy-' || n, 'member_' || n, 'Existing member' FROM generate_series(1,64) n")
        .execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../migrations/202609300001_user_avatars.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let assignments: Vec<(i64, i16)> = sqlx::query_as("SELECT id,avatar_id FROM users ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(assignments.len(), 64);
    assert!(assignments.iter().all(|(_, id)| (0..800).contains(id)));
    // A non-volatile/once-evaluated default would assign everyone the same image.
    assert!(assignments.iter().any(|(_, id)| *id != assignments[0].1));
    for (user, avatar) in assignments {
        let profile = set_profile(&pool, user, &format!("member_{user}"), "Renamed")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(profile.avatar_id, avatar);
        assert_eq!(
            serde_json::to_value(profile.public()).unwrap()["avatarId"],
            avatar
        );
    }
    for invalid in [-1_i16, 800] {
        assert!(
            sqlx::query("UPDATE users SET avatar_id=$1 WHERE id=1")
                .bind(invalid)
                .execute(&pool)
                .await
                .is_err()
        );
    }
}
