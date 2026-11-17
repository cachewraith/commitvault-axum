use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use super::{
    dto::{TagResponse, TagSort},
    models::{Tag, TagTarget},
};
use crate::shared::pagination::{Page, SortOrder};

/// Tag columns plus usage counters. Each counter is an index-only scan on the
/// join table's `tag_id` index.
const VIEW_SQL: &str = "\
    SELECT q.*, (q.project_count + q.commit_count + q.deployment_count + q.release_count + q.note_count) AS total \
    FROM (SELECT t.id, t.name, t.slug, t.color, t.created_at, t.updated_at, \
        (SELECT count(*) FROM project_tags x WHERE x.tag_id = t.id) AS project_count, \
        (SELECT count(*) FROM commit_tags x WHERE x.tag_id = t.id) AS commit_count, \
        (SELECT count(*) FROM deployment_tags x WHERE x.tag_id = t.id) AS deployment_count, \
        (SELECT count(*) FROM release_tags x WHERE x.tag_id = t.id) AS release_count, \
        (SELECT count(*) FROM note_tags x WHERE x.tag_id = t.id) AS note_count \
        FROM tags t";

pub struct TagFilter {
    /// Already escaped `ILIKE` pattern.
    pub name_pattern: Option<String>,
    pub sort: TagSort,
    pub order: SortOrder,
}

pub async fn list(pool: &PgPool, filter: &TagFilter, page: Page) -> sqlx::Result<(Vec<TagResponse>, i64)> {
    let mut rows: QueryBuilder<Postgres> = QueryBuilder::new(VIEW_SQL);
    let mut count: QueryBuilder<Postgres> = QueryBuilder::new("SELECT count(*) FROM tags t");
    if let Some(pattern) = &filter.name_pattern {
        rows.push(" WHERE t.name ILIKE ").push_bind(pattern);
        count.push(" WHERE t.name ILIKE ").push_bind(pattern);
    }
    let column = match filter.sort {
        TagSort::Name => "lower(q.name)",
        TagSort::CreatedAt => "q.created_at",
        TagSort::Usage => "total",
    };
    rows.push(") q ORDER BY ").push(column).push(' ').push(filter.order.sql()).push(", q.id");
    rows.push(" LIMIT ").push_bind(page.limit()).push(" OFFSET ").push_bind(page.offset());

    tokio::try_join!(
        rows.build_query_as::<TagResponse>().fetch_all(pool),
        count.build_query_scalar::<i64>().fetch_one(pool),
    )
}

pub async fn find_view(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<TagResponse>> {
    let mut query: QueryBuilder<Postgres> = QueryBuilder::new(VIEW_SQL);
    query.push(" WHERE t.id = ").push_bind(id).push(") q");
    query.build_query_as::<TagResponse>().fetch_optional(pool).await
}

pub async fn find(conn: &mut PgConnection, id: Uuid) -> sqlx::Result<Option<Tag>> {
    sqlx::query_as!(Tag, "SELECT id, name, slug, color, created_at, updated_at FROM tags WHERE id = $1 FOR UPDATE", id)
        .fetch_optional(conn)
        .await
}

pub async fn insert(pool: &PgPool, name: &str, slug: &str, color: &str) -> sqlx::Result<Uuid> {
    sqlx::query_scalar!(
        "INSERT INTO tags (id, name, slug, color) VALUES ($1, $2, $3, $4) RETURNING id",
        Uuid::now_v7(),
        name,
        slug,
        color,
    )
    .fetch_one(pool)
    .await
}

pub async fn update(conn: &mut PgConnection, tag: &Tag) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE tags SET name = $2, slug = $3, color = $4 WHERE id = $1",
        tag.id,
        tag.name,
        tag.slug,
        tag.color
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Returns `false` when no such tag existed.
pub async fn delete(pool: &PgPool, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!("DELETE FROM tags WHERE id = $1", id).execute(pool).await?;
    Ok(result.rows_affected() > 0)
}

/// Replaces the tag set of one entity. Returns how many of `tag_ids` exist and
/// were attached, so the caller can detect unknown ids.
pub async fn replace_links(
    conn: &mut PgConnection,
    target: TagTarget,
    entity_id: Uuid,
    tag_ids: &[Uuid],
) -> sqlx::Result<u64> {
    let (table, column) = target.table();
    // `table`/`column` are constants from `TagTarget::table`, never user input.
    sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table} WHERE {column} = $1")))
        .bind(entity_id)
        .execute(&mut *conn)
        .await?;
    if tag_ids.is_empty() {
        return Ok(0);
    }
    let inserted = sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO {table} ({column}, tag_id) SELECT $1, id FROM tags WHERE id = ANY($2)"
    )))
    .bind(entity_id)
    .bind(tag_ids)
    .execute(&mut *conn)
    .await?;
    Ok(inserted.rows_affected())
}
