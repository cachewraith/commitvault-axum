use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

use super::models::User;

pub async fn find_by_email(pool: &PgPool, email: &str) -> sqlx::Result<Option<User>> {
    sqlx::query_as!(User, "SELECT id, email, password_hash, created_at FROM users WHERE email = $1", email)
        .fetch_optional(pool)
        .await
}

pub async fn find_by_id(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<User>> {
    sqlx::query_as!(User, "SELECT id, email, password_hash, created_at FROM users WHERE id = $1", id)
        .fetch_optional(pool)
        .await
}

pub async fn count(executor: impl PgExecutor<'_>) -> sqlx::Result<i64> {
    sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM users"#).fetch_one(executor).await
}

pub async fn insert(executor: impl PgExecutor<'_>, email: &str, password_hash: &str) -> sqlx::Result<Uuid> {
    sqlx::query_scalar!(
        "INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3) RETURNING id",
        Uuid::now_v7(),
        email,
        password_hash,
    )
    .fetch_one(executor)
    .await
}

pub async fn update_password(pool: &PgPool, id: Uuid, password_hash: &str) -> sqlx::Result<()> {
    sqlx::query!("UPDATE users SET password_hash = $2 WHERE id = $1", id, password_hash).execute(pool).await?;
    Ok(())
}

/// Removes every session of `user_id` except `keep_session_id`.
pub async fn delete_other_sessions(pool: &PgPool, user_id: Uuid, keep_session_id: Option<&str>) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        "DELETE FROM sessions WHERE data ->> 'user_id' = $1 AND ($2::text IS NULL OR id <> $2)",
        user_id.to_string(),
        keep_session_id,
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}
