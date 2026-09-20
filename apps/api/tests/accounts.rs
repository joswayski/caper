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
    assert_eq!(email.1, "YES");
    assert_eq!(verified.1, "YES");
    assert_eq!(verified.2, None);
    assert_eq!(deleted.1, "YES");

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
}
