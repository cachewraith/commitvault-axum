//! One bounded query per record type. Text predicates use `ILIKE` with an
//! escaped pattern (served by the trigram indexes); the term itself is only
//! ever a bind parameter. Every statement below is assembled from constants.

use chrono::{DateTime, Utc};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use crate::shared::{refs::ProjectRef, sql::PROJECT_REF_JSON};

#[derive(Debug, sqlx::FromRow)]
pub struct HitRow {
    pub id: Uuid,
    pub title: String,
    pub subtitle: Option<String>,
    #[sqlx(json(nullable))]
    pub project: Option<ProjectRef>,
    pub occurred_at: Option<DateTime<Utc>>,
}

pub struct SearchParams<'a> {
    /// Escaped `%term%` pattern.
    pub pattern: &'a str,
    /// Escaped `term%` pattern when the term could be a commit hash.
    pub hash_prefix: Option<&'a str>,
    pub project_id: Option<Uuid>,
    pub limit: i64,
}

pub async fn projects(pool: &PgPool, params: &SearchParams<'_>) -> sqlx::Result<Vec<HitRow>> {
    sqlx::query_as::<_, HitRow>(
        "SELECT p.id, p.name AS title, left(p.description, 160) AS subtitle, NULL::jsonb AS project, \
                p.updated_at AS occurred_at \
         FROM projects p \
         WHERE (p.name ILIKE $1 OR p.slug ILIKE $1 OR p.description ILIKE $1) AND ($2::uuid IS NULL OR p.id = $2) \
         ORDER BY (p.status = 'active') DESC, p.updated_at DESC LIMIT $3",
    )
    .bind(params.pattern)
    .bind(params.project_id)
    .bind(params.limit)
    .fetch_all(pool)
    .await
}

pub async fn commits(pool: &PgPool, params: &SearchParams<'_>) -> sqlx::Result<Vec<HitRow>> {
    // `$4` is NULL unless the term looks like a hash; `LIKE NULL` is then
    // NULL (no match), and hash-prefix matches sort first when present.
    let sql = format!(
        "SELECT c.id, c.commit_message AS title, c.short_hash || ' · ' || b.name AS subtitle, \
                {PROJECT_REF_JSON} AS project, c.commit_date AS occurred_at \
         FROM commits c JOIN projects p ON p.id = c.project_id JOIN branches b ON b.id = c.branch_id \
         WHERE (c.commit_message ILIKE $1 OR b.name ILIKE $1 OR c.author_name ILIKE $1 \
                OR c.description ILIKE $1 OR c.commit_hash LIKE $4::text) \
           AND ($2::uuid IS NULL OR c.project_id = $2) \
         ORDER BY coalesce(c.commit_hash LIKE $4::text, false) DESC, c.commit_date DESC LIMIT $3"
    );
    sqlx::query_as::<_, HitRow>(AssertSqlSafe(sql))
        .bind(params.pattern)
        .bind(params.project_id)
        .bind(params.limit)
        .bind(params.hash_prefix)
        .fetch_all(pool)
        .await
}

pub async fn deployments(pool: &PgPool, params: &SearchParams<'_>) -> sqlx::Result<Vec<HitRow>> {
    let sql = format!(
        "SELECT d.id, d.deployment_title AS title, e.name || ' · ' || replace(d.status, '_', ' ') AS subtitle, \
                {PROJECT_REF_JSON} AS project, d.deployed_at AS occurred_at \
         FROM deployments d JOIN projects p ON p.id = d.project_id JOIN environments e ON e.id = d.environment_id \
         WHERE (d.deployment_title ILIKE $1 OR d.deployment_notes ILIKE $1) \
           AND ($2::uuid IS NULL OR d.project_id = $2) \
         ORDER BY d.deployed_at DESC LIMIT $3"
    );
    project_scoped(pool, sql, params).await
}

pub async fn releases(pool: &PgPool, params: &SearchParams<'_>) -> sqlx::Result<Vec<HitRow>> {
    let sql = format!(
        "SELECT r.id, r.version || ' — ' || r.title AS title, r.status AS subtitle, {PROJECT_REF_JSON} AS project, \
                coalesce(r.released_at, r.created_at) AS occurred_at \
         FROM releases r JOIN projects p ON p.id = r.project_id \
         WHERE (r.version ILIKE $1 OR r.title ILIKE $1 OR r.description ILIKE $1 OR r.release_notes_text ILIKE $1) \
           AND ($2::uuid IS NULL OR r.project_id = $2) \
         ORDER BY coalesce(r.released_at, r.created_at) DESC LIMIT $3"
    );
    project_scoped(pool, sql, params).await
}

pub async fn notes(pool: &PgPool, params: &SearchParams<'_>) -> sqlx::Result<Vec<HitRow>> {
    let sql = format!(
        "SELECT n.id, n.title, nullif(n.excerpt, '') AS subtitle, \
                CASE WHEN p.id IS NULL THEN NULL ELSE {PROJECT_REF_JSON} END AS project, n.updated_at AS occurred_at \
         FROM notes n LEFT JOIN projects p ON p.id = n.project_id \
         WHERE (n.title ILIKE $1 OR n.category ILIKE $1 OR n.content_text ILIKE $1) \
           AND ($2::uuid IS NULL OR n.project_id = $2) \
         ORDER BY (n.title ILIKE $1) DESC, n.updated_at DESC LIMIT $3"
    );
    project_scoped(pool, sql, params).await
}

/// Tags are global, so the project filter does not apply.
pub async fn tags(pool: &PgPool, params: &SearchParams<'_>) -> sqlx::Result<Vec<HitRow>> {
    sqlx::query_as::<_, HitRow>(
        "SELECT t.id, t.name AS title, NULL::text AS subtitle, NULL::jsonb AS project, \
                NULL::timestamptz AS occurred_at \
         FROM tags t WHERE t.name ILIKE $1 ORDER BY lower(t.name) LIMIT $2",
    )
    .bind(params.pattern)
    .bind(params.limit)
    .fetch_all(pool)
    .await
}

/// Runs a statement taking `($1 pattern, $2 project_id, $3 limit)`.
async fn project_scoped(pool: &PgPool, sql: String, params: &SearchParams<'_>) -> sqlx::Result<Vec<HitRow>> {
    sqlx::query_as::<_, HitRow>(AssertSqlSafe(sql))
        .bind(params.pattern)
        .bind(params.project_id)
        .bind(params.limit)
        .fetch_all(pool)
        .await
}
