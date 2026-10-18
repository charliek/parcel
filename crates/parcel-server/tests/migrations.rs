//! Migrations that move data, against the state they meet in production.

mod common;

use sqlx::{AssertSqlSafe, PgPool};

const BEFORE_0005: [&str; 4] = [
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_comments_and_data.sql"),
    include_str!("../migrations/0003_identities_and_sharing.sql"),
    include_str!("../migrations/0004_comment_via_mcp.sql"),
];
const DROP_USERS_SUB: &str = include_str!("../migrations/0005_drop_users_sub.sql");

async fn apply(pool: &PgPool, sql: &str) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(AssertSqlSafe(sql.to_string()))
        .execute(pool)
        .await
        .map(|_| ())
}

#[tokio::test]
async fn dropping_users_sub_refuses_to_strand_a_user() {
    let pool = common::schema_pool().await;
    for sql in BEFORE_0005 {
        apply(&pool, sql).await.expect("earlier migration");
    }
    // A legacy row with no identity (0003 backfills one for every user, so
    // only a hand-edited database looks like this).
    let stranded: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO users (sub, email, name) VALUES ('legacy-sub', 'old@example.com', 'Old')
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let refused = apply(&pool, DROP_USERS_SUB).await.unwrap_err();
    assert!(
        refused.to_string().contains("not dropping users.sub"),
        "{refused}"
    );

    sqlx::query(
        "INSERT INTO user_identities (provider, subject, user_id, email)
         VALUES ('slauth', 'legacy-sub', $1, 'old@example.com')",
    )
    .bind(stranded)
    .execute(&pool)
    .await
    .unwrap();
    apply(&pool, DROP_USERS_SUB)
        .await
        .expect("drops once every user can sign in");
    let column: Option<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns
         WHERE table_schema = current_schema() AND table_name = 'users' AND column_name = 'sub'",
    )
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(column, None);
}
