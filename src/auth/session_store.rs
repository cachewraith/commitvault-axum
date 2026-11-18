//! PostgreSQL-backed `tower_sessions::SessionStore` using the `sessions`
//! table from our own migrations (so the schema is versioned with the rest of
//! the database instead of being created at runtime by a third-party crate).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, postgres::types::PgInterval};
use tower_sessions::{
    SessionStore,
    session::{Id, Record},
    session_store::{Error, Result},
};

/// How far the stored expiry may lag behind the sliding expiry before an
/// otherwise unchanged session row is rewritten.
const EXPIRY_WRITE_SLACK: PgInterval = PgInterval { months: 0, days: 0, microseconds: 10 * 60 * 1_000_000 };

#[derive(Debug, Clone)]
pub struct PgSessionStore {
    pool: PgPool,
}

impl PgSessionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Deletes expired rows; returns how many were removed.
    pub async fn delete_expired(&self) -> sqlx::Result<u64> {
        let result = sqlx::query!("DELETE FROM sessions WHERE expires_at < now()").execute(&self.pool).await?;
        Ok(result.rows_affected())
    }

    /// Periodically purges expired sessions (and expired one-time codes and
    /// trusted browsers) until the task is aborted.
    pub async fn run_cleanup(self, every: std::time::Duration) {
        let mut interval = tokio::time::interval(every);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            match self.delete_expired().await {
                Ok(0) => {}
                Ok(count) => tracing::debug!(count, "purged expired sessions"),
                Err(err) => tracing::warn!(error = %err, "failed to purge expired sessions"),
            }
            if let Err(err) = super::repository::purge_expired(&self.pool).await {
                tracing::warn!(error = %err, "failed to purge expired sign-in codes");
            }
        }
    }
}

fn backend(err: sqlx::Error) -> Error {
    Error::Backend(err.to_string())
}

fn expires_at(record: &Record) -> Result<DateTime<Utc>> {
    DateTime::from_timestamp(record.expiry_date.unix_timestamp(), record.expiry_date.nanosecond())
        .ok_or_else(|| Error::Encode("session expiry is out of range".into()))
}

fn encode(record: &Record) -> Result<serde_json::Value> {
    serde_json::to_value(&record.data).map_err(|err| Error::Encode(err.to_string()))
}

#[async_trait]
impl SessionStore for PgSessionStore {
    async fn create(&self, record: &mut Record) -> Result<()> {
        let data = encode(record)?;
        let expires_at = expires_at(record)?;
        // Session ids are 128 random bits, so a collision is practically
        // impossible — but if one happens we must not overwrite (and thereby
        // hand over) somebody else's session: pick a new id and retry.
        loop {
            let inserted = sqlx::query!(
                "INSERT INTO sessions (id, data, expires_at) VALUES ($1, $2, $3) ON CONFLICT (id) DO NOTHING",
                record.id.to_string(),
                data,
                expires_at,
            )
            .execute(&self.pool)
            .await
            .map_err(backend)?;
            if inserted.rows_affected() == 1 {
                return Ok(());
            }
            record.id = Id::default();
        }
    }

    /// Called on every response (sliding expiry). Rewriting an unchanged row
    /// each time turned every read request into a write — a WAL flush per
    /// page view on a small server — so the row is only written when its data
    /// changed, its expiry moved earlier, or the stored expiry has fallen more
    /// than `EXPIRY_WRITE_SLACK` behind. A session may therefore end up to
    /// that much earlier than its cookie says; with a lifetime of days that is
    /// immaterial.
    async fn save(&self, record: &Record) -> Result<()> {
        sqlx::query!(
            "WITH changed AS (
                 UPDATE sessions SET data = $2, expires_at = $3
                 WHERE id = $1
                   AND (data IS DISTINCT FROM $2 OR expires_at > $3 OR expires_at < $3 - $4::interval)
             )
             INSERT INTO sessions (id, data, expires_at)
             SELECT $1::text, $2::jsonb, $3::timestamptz
             WHERE NOT EXISTS (SELECT 1 FROM sessions WHERE id = $1)
             ON CONFLICT (id) DO UPDATE SET data = EXCLUDED.data, expires_at = EXCLUDED.expires_at",
            record.id.to_string(),
            encode(record)?,
            expires_at(record)?,
            EXPIRY_WRITE_SLACK,
        )
        .execute(&self.pool)
        .await
        .map_err(backend)?;
        Ok(())
    }

    async fn load(&self, session_id: &Id) -> Result<Option<Record>> {
        let row = sqlx::query!(
            "SELECT data, expires_at FROM sessions WHERE id = $1 AND expires_at > now()",
            session_id.to_string(),
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;

        row.map(|row| {
            let data = serde_json::from_value(row.data).map_err(|err| Error::Decode(err.to_string()))?;
            let expiry_date = time::OffsetDateTime::from_unix_timestamp_nanos(
                i128::from(row.expires_at.timestamp()) * 1_000_000_000
                    + i128::from(row.expires_at.timestamp_subsec_nanos()),
            )
            .map_err(|err| Error::Decode(err.to_string()))?;
            Ok(Record { id: *session_id, data, expiry_date })
        })
        .transpose()
    }

    async fn delete(&self, session_id: &Id) -> Result<()> {
        sqlx::query!("DELETE FROM sessions WHERE id = $1", session_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(backend)?;
        Ok(())
    }
}
