use std::sync::LazyLock;

use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use super::{
    dto::{ReleaseDetailResponse, ReleaseResponse, ReleaseSort},
    models::{Release, ReleaseStatus},
};
use crate::{
    modules::tags::models::TagTarget,
    shared::{
        pagination::{Page, SortOrder},
        sql::{COMMIT_BRIEF_JSON, PROJECT_REF_JSON},
    },
};

static COLUMNS_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT r.id, r.project_id, {PROJECT_REF_JSON} AS project, r.version, r.title, r.description, \
                r.released_at, r.status, r.created_at, r.updated_at, {tags} AS tags, \
                (SELECT count(*) FROM release_commits rc WHERE rc.release_id = r.id) AS commit_count",
        tags = TagTarget::Release.json_sql("r.id"),
    )
});

const FROM_SQL: &str = " FROM releases r JOIN projects p ON p.id = r.project_id";

#[derive(Debug, Default)]
pub struct ReleaseFilter {
    pub project_id: Option<Uuid>,
    pub status: Option<ReleaseStatus>,
    /// Escaped `%term%` pattern.
    pub pattern: Option<String>,
    pub tag_id: Option<Uuid>,
    pub commit_id: Option<Uuid>,
    pub sort: ReleaseSort,
    pub order: SortOrder,
}

fn push_filters(query: &mut QueryBuilder<Postgres>, filter: &ReleaseFilter) {
    query.push(" WHERE true");
    if let Some(project_id) = filter.project_id {
        query.push(" AND r.project_id = ").push_bind(project_id);
    }
    if let Some(status) = filter.status {
        query.push(" AND r.status = ").push_bind(status.as_str());
    }
    if let Some(pattern) = &filter.pattern {
        query.push(" AND (r.version ILIKE ").push_bind(pattern);
        query.push(" OR r.title ILIKE ").push_bind(pattern);
        query.push(" OR r.description ILIKE ").push_bind(pattern);
        query.push(" OR r.release_notes_text ILIKE ").push_bind(pattern).push(")");
    }
    if let Some(tag_id) = filter.tag_id {
        query.push(" AND EXISTS (SELECT 1 FROM release_tags rt WHERE rt.release_id = r.id AND rt.tag_id = ");
        query.push_bind(tag_id).push(")");
    }
    if let Some(commit_id) = filter.commit_id {
        query.push(" AND EXISTS (SELECT 1 FROM release_commits rc WHERE rc.release_id = r.id AND rc.commit_id = ");
        query.push_bind(commit_id).push(")");
    }
}

pub async fn list(pool: &PgPool, filter: &ReleaseFilter, page: Page) -> sqlx::Result<(Vec<ReleaseResponse>, i64)> {
    let mut rows: QueryBuilder<Postgres> = QueryBuilder::new(format!("{}{FROM_SQL}", COLUMNS_SQL.as_str()));
    push_filters(&mut rows, filter);
    rows.push(" ORDER BY ");
    match (filter.sort, filter.order) {
        // Unreleased drafts have no date: show them first when newest-first.
        (ReleaseSort::ReleasedAt, SortOrder::Desc) => rows.push("r.released_at DESC NULLS FIRST, r.created_at DESC"),
        (ReleaseSort::ReleasedAt, SortOrder::Asc) => rows.push("r.released_at ASC NULLS LAST, r.created_at ASC"),
        (ReleaseSort::CreatedAt, order) => rows.push("r.created_at ").push(order.sql()),
        (ReleaseSort::Version, order) => rows.push("r.version ").push(order.sql()),
    };
    rows.push(", r.id LIMIT ").push_bind(page.limit()).push(" OFFSET ").push_bind(page.offset());

    let mut count: QueryBuilder<Postgres> = QueryBuilder::new(format!("SELECT count(*){FROM_SQL}"));
    push_filters(&mut count, filter);

    tokio::try_join!(
        rows.build_query_as::<ReleaseResponse>().fetch_all(pool),
        count.build_query_scalar::<i64>().fetch_one(pool),
    )
}

pub async fn find_detail(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<ReleaseDetailResponse>> {
    let mut query: QueryBuilder<Postgres> = QueryBuilder::new(COLUMNS_SQL.as_str());
    query.push(format!(
        ", r.release_notes, \
         (SELECT coalesce(jsonb_agg({COMMIT_BRIEF_JSON} ORDER BY c.commit_date DESC, c.id), '[]'::jsonb) \
          FROM release_commits rc JOIN commits c ON c.id = rc.commit_id WHERE rc.release_id = r.id) AS commits\
         {FROM_SQL} WHERE r.id = "
    ));
    query.push_bind(id);
    query.build_query_as::<ReleaseDetailResponse>().fetch_optional(pool).await
}

pub async fn find_for_update(conn: &mut PgConnection, id: Uuid) -> sqlx::Result<Option<Release>> {
    sqlx::query_as!(
        Release,
        r#"SELECT id, project_id, version, title, description, release_notes, release_notes_text, released_at,
                  status AS "status: ReleaseStatus"
           FROM releases WHERE id = $1 FOR UPDATE"#,
        id,
    )
    .fetch_optional(conn)
    .await
}

pub async fn insert(conn: &mut PgConnection, release: &Release) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO releases (id, project_id, version, title, description, release_notes, release_notes_text,
                               released_at, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        release.id,
        release.project_id,
        release.version,
        release.title,
        release.description,
        release.release_notes,
        release.release_notes_text,
        release.released_at,
        release.status.as_str(),
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn update(conn: &mut PgConnection, release: &Release) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE releases
         SET version = $2, title = $3, description = $4, release_notes = $5, release_notes_text = $6,
             released_at = $7, status = $8
         WHERE id = $1",
        release.id,
        release.version,
        release.title,
        release.description,
        release.release_notes,
        release.release_notes_text,
        release.released_at,
        release.status.as_str(),
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!("DELETE FROM releases WHERE id = $1", id).execute(pool).await?;
    Ok(result.rows_affected() > 0)
}

/// Replaces the release's commit set. Only commits of `project_id` are
/// attached; the return value is how many were, so callers can detect ids
/// that are unknown or belong to another project.
pub async fn replace_commits(
    conn: &mut PgConnection,
    release_id: Uuid,
    project_id: Uuid,
    commit_ids: &[Uuid],
) -> sqlx::Result<u64> {
    sqlx::query!("DELETE FROM release_commits WHERE release_id = $1", release_id).execute(&mut *conn).await?;
    if commit_ids.is_empty() {
        return Ok(0);
    }
    let inserted = sqlx::query!(
        "INSERT INTO release_commits (release_id, commit_id, project_id)
         SELECT $1, id, $2 FROM commits WHERE id = ANY($3) AND project_id = $2",
        release_id,
        project_id,
        commit_ids,
    )
    .execute(&mut *conn)
    .await?;
    Ok(inserted.rows_affected())
}
