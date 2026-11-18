use chrono::{DateTime, Utc};
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

/// Removes every session of `user_id`.
pub async fn delete_all_sessions(pool: &PgPool, user_id: Uuid) -> sqlx::Result<u64> {
    delete_other_sessions(pool, user_id, None).await
}

// --- One-time codes ----------------------------------------------------------

/// An unexpired, unused code that still accepts guesses.
pub struct ActiveCode {
    pub id: Uuid,
    pub code_hash: String,
}

/// Stores a new code and retires every earlier unused code of the same
/// purpose, so only the most recently sent code works.
pub async fn replace_code(
    pool: &PgPool,
    user_id: Uuid,
    purpose: &str,
    code_hash: &str,
    expires_at: DateTime<Utc>,
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query!(
        "UPDATE auth_codes SET consumed_at = now()
         WHERE user_id = $1 AND purpose = $2 AND consumed_at IS NULL",
        user_id,
        purpose,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO auth_codes (id, user_id, purpose, code_hash, expires_at) VALUES ($1, $2, $3, $4, $5)",
        Uuid::now_v7(),
        user_id,
        purpose,
        code_hash,
        expires_at,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

/// When the latest code of this purpose was issued (for resend throttling).
pub async fn last_code_sent_at(pool: &PgPool, user_id: Uuid, purpose: &str) -> sqlx::Result<Option<DateTime<Utc>>> {
    sqlx::query_scalar!(
        r#"SELECT max(created_at) AS "sent_at" FROM auth_codes WHERE user_id = $1 AND purpose = $2"#,
        user_id,
        purpose,
    )
    .fetch_one(pool)
    .await
}

pub async fn find_active_code(
    pool: &PgPool,
    user_id: Uuid,
    purpose: &str,
    max_attempts: i32,
) -> sqlx::Result<Option<ActiveCode>> {
    sqlx::query_as!(
        ActiveCode,
        "SELECT id, code_hash FROM auth_codes
         WHERE user_id = $1 AND purpose = $2 AND consumed_at IS NULL AND expires_at > now() AND attempts < $3
         ORDER BY created_at DESC LIMIT 1",
        user_id,
        purpose,
        max_attempts,
    )
    .fetch_optional(pool)
    .await
}

pub async fn record_code_attempt(pool: &PgPool, id: Uuid) -> sqlx::Result<()> {
    sqlx::query!("UPDATE auth_codes SET attempts = attempts + 1 WHERE id = $1", id).execute(pool).await?;
    Ok(())
}

/// Marks the code used. `false` if it was already used (a concurrent request won).
pub async fn consume_code(pool: &PgPool, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!("UPDATE auth_codes SET consumed_at = now() WHERE id = $1 AND consumed_at IS NULL", id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() == 1)
}

// --- Trusted devices ---------------------------------------------------------

pub async fn insert_trusted_device(
    pool: &PgPool,
    user_id: Uuid,
    token_hash: &str,
    expires_at: DateTime<Utc>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO trusted_devices (id, user_id, token_hash, expires_at) VALUES ($1, $2, $3, $4)",
        Uuid::now_v7(),
        user_id,
        token_hash,
        expires_at,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// `true` (and the device's last use is recorded) when the token belongs to
/// an unexpired trusted device of this user.
pub async fn touch_trusted_device(pool: &PgPool, user_id: Uuid, token_hash: &str) -> sqlx::Result<bool> {
    let result = sqlx::query!(
        "UPDATE trusted_devices SET last_used_at = now()
         WHERE user_id = $1 AND token_hash = $2 AND expires_at > now()",
        user_id,
        token_hash,
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn delete_trusted_devices(pool: &PgPool, user_id: Uuid) -> sqlx::Result<u64> {
    let result = sqlx::query!("DELETE FROM trusted_devices WHERE user_id = $1", user_id).execute(pool).await?;
    Ok(result.rows_affected())
}

/// Deletes expired codes and trusted devices; returns how many rows went.
pub async fn purge_expired(pool: &PgPool) -> sqlx::Result<u64> {
    let codes =
        sqlx::query!("DELETE FROM auth_codes WHERE expires_at < now() - interval '1 day'").execute(pool).await?;
    let devices = sqlx::query!("DELETE FROM trusted_devices WHERE expires_at < now()").execute(pool).await?;
    Ok(codes.rows_affected() + devices.rows_affected())
}
