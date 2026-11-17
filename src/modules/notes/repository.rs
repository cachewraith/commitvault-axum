use std::sync::LazyLock;

use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use super::{
    dto::{NoteCategory, NoteDetailResponse, NoteResponse, NoteSort},
    models::Note,
};
use crate::{
    modules::tags::models::TagTarget,
    shared::{
        pagination::{Page, SortOrder},
        sql::PROJECT_REF_JSON,
    },
};

/// List columns. The document body is deliberately left out: lists only need
/// the excerpt, and bodies can be large.
static COLUMNS_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT n.id, n.project_id, CASE WHEN p.id IS NULL THEN NULL ELSE {PROJECT_REF_JSON} END AS project, \
                n.title, n.excerpt, n.category, n.is_pinned, n.created_at, n.updated_at, {tags} AS tags",
        tags = TagTarget::Note.json_sql("n.id"),
    )
});

const FROM_SQL: &str = " FROM notes n LEFT JOIN projects p ON p.id = n.project_id";

#[derive(Debug, Default)]
pub struct NoteFilter {
    pub project_id: Option<Uuid>,
    pub global_only: bool,
    pub category: Option<String>,
    pub pinned: Option<bool>,
    /// Escaped `%term%` pattern.
    pub pattern: Option<String>,
    pub tag_id: Option<Uuid>,
    pub sort: NoteSort,
    pub order: SortOrder,
    /// Keep pinned notes at the top regardless of the sort column.
    pub pinned_first: bool,
}

fn push_filters(query: &mut QueryBuilder<Postgres>, filter: &NoteFilter) {
    query.push(" WHERE true");
    if let Some(project_id) = filter.project_id {
        query.push(" AND n.project_id = ").push_bind(project_id);
    }
    if filter.global_only {
        query.push(" AND n.project_id IS NULL");
    }
    if let Some(category) = &filter.category {
        query.push(" AND n.category = ").push_bind(category);
    }
    if let Some(pinned) = filter.pinned {
        query.push(" AND n.is_pinned = ").push_bind(pinned);
    }
    if let Some(pattern) = &filter.pattern {
        query.push(" AND (n.title ILIKE ").push_bind(pattern);
        query.push(" OR n.category ILIKE ").push_bind(pattern);
        query.push(" OR n.content_text ILIKE ").push_bind(pattern).push(")");
    }
    if let Some(tag_id) = filter.tag_id {
        query.push(" AND EXISTS (SELECT 1 FROM note_tags nt WHERE nt.note_id = n.id AND nt.tag_id = ");
        query.push_bind(tag_id).push(")");
    }
}

pub async fn list(pool: &PgPool, filter: &NoteFilter, page: Page) -> sqlx::Result<(Vec<NoteResponse>, i64)> {
    let mut rows: QueryBuilder<Postgres> = QueryBuilder::new(format!("{}{FROM_SQL}", COLUMNS_SQL.as_str()));
    push_filters(&mut rows, filter);
    let column = match filter.sort {
        NoteSort::UpdatedAt => "n.updated_at",
        NoteSort::CreatedAt => "n.created_at",
        NoteSort::Title => "lower(n.title)",
    };
    rows.push(" ORDER BY ");
    if filter.pinned_first {
        rows.push("n.is_pinned DESC, ");
    }
    rows.push(column).push(' ').push(filter.order.sql()).push(", n.id");
    rows.push(" LIMIT ").push_bind(page.limit()).push(" OFFSET ").push_bind(page.offset());

    let mut count: QueryBuilder<Postgres> = QueryBuilder::new("SELECT count(*) FROM notes n");
    push_filters(&mut count, filter);

    tokio::try_join!(
        rows.build_query_as::<NoteResponse>().fetch_all(pool),
        count.build_query_scalar::<i64>().fetch_one(pool),
    )
}

pub async fn find_detail(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<NoteDetailResponse>> {
    let mut query: QueryBuilder<Postgres> = QueryBuilder::new(COLUMNS_SQL.as_str());
    query.push(", n.content_json").push(FROM_SQL).push(" WHERE n.id = ").push_bind(id);
    query.build_query_as::<NoteDetailResponse>().fetch_optional(pool).await
}

pub async fn find_for_update(conn: &mut PgConnection, id: Uuid) -> sqlx::Result<Option<Note>> {
    sqlx::query_as!(
        Note,
        "SELECT id, project_id, title, content_json, content_text, excerpt, category, is_pinned
         FROM notes WHERE id = $1 FOR UPDATE",
        id,
    )
    .fetch_optional(conn)
    .await
}

pub async fn insert(conn: &mut PgConnection, note: &Note) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO notes (id, project_id, title, content_json, content_text, excerpt, category, is_pinned)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        note.id,
        note.project_id,
        note.title,
        note.content_json,
        note.content_text,
        note.excerpt,
        note.category,
        note.is_pinned,
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn update(conn: &mut PgConnection, note: &Note) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE notes
         SET project_id = $2, title = $3, content_json = $4, content_text = $5, excerpt = $6,
             category = $7, is_pinned = $8
         WHERE id = $1",
        note.id,
        note.project_id,
        note.title,
        note.content_json,
        note.content_text,
        note.excerpt,
        note.category,
        note.is_pinned,
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!("DELETE FROM notes WHERE id = $1", id).execute(pool).await?;
    Ok(result.rows_affected() > 0)
}

/// Categories in use, most used first.
pub async fn categories(pool: &PgPool, project_id: Option<Uuid>, limit: i64) -> sqlx::Result<Vec<NoteCategory>> {
    sqlx::query_as!(
        NoteCategory,
        r#"SELECT category AS "category!", count(*) AS "note_count!"
           FROM notes
           WHERE category IS NOT NULL AND ($1::uuid IS NULL OR project_id = $1)
           GROUP BY category ORDER BY count(*) DESC, category LIMIT $2"#,
        project_id,
        limit,
    )
    .fetch_all(pool)
    .await
}
