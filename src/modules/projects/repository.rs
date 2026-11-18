use std::sync::LazyLock;

use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use super::{
    dto::{ProjectResponse, ProjectSort},
    models::{Project, ProjectStatus},
};
use crate::{
    modules::tags::models::TagTarget,
    shared::pagination::{Page, SortOrder},
};

/// Read model: project columns, tags and record counts in one statement.
/// Each counter is answered from an index on `project_id`.
static VIEW_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT p.id, p.name, p.slug, p.description, p.repository_url, p.default_branch, p.status, \
                p.created_at, p.updated_at, p.archived_at, {tags} AS tags, \
                (SELECT count(*) FROM commits x WHERE x.project_id = p.id) AS commit_count, \
                (SELECT count(*) FROM environments x WHERE x.project_id = p.id) AS environment_count, \
                (SELECT count(*) FROM deployments x WHERE x.project_id = p.id) AS deployment_count, \
                (SELECT count(*) FROM releases x WHERE x.project_id = p.id) AS release_count, \
                (SELECT count(*) FROM notes x WHERE x.project_id = p.id) AS note_count, \
                (SELECT max(x.deployed_at) FROM deployments x WHERE x.project_id = p.id) AS last_deployed_at \
         FROM projects p",
        tags = TagTarget::Project.json_sql("p.id"),
    )
});

#[derive(Debug, Default)]
pub struct ProjectFilter {
    /// Already escaped `ILIKE` pattern.
    pub pattern: Option<String>,
    pub status: Option<ProjectStatus>,
    pub tag_id: Option<Uuid>,
    pub sort: ProjectSort,
    pub order: SortOrder,
}

fn push_filters(query: &mut QueryBuilder<Postgres>, filter: &ProjectFilter) {
    query.push(" WHERE true");
    if let Some(pattern) = &filter.pattern {
        query.push(" AND (p.name ILIKE ").push_bind(pattern);
        query.push(" OR p.slug ILIKE ").push_bind(pattern);
        query.push(" OR p.description ILIKE ").push_bind(pattern).push(")");
    }
    if let Some(status) = filter.status {
        query.push(" AND p.status = ").push_bind(status.as_str());
    }
    if let Some(tag_id) = filter.tag_id {
        query.push(" AND EXISTS (SELECT 1 FROM project_tags pt WHERE pt.project_id = p.id AND pt.tag_id = ");
        query.push_bind(tag_id).push(")");
    }
}

pub async fn list(pool: &PgPool, filter: &ProjectFilter, page: Page) -> sqlx::Result<(Vec<ProjectResponse>, i64)> {
    let mut rows: QueryBuilder<Postgres> = QueryBuilder::new(VIEW_SQL.as_str());
    push_filters(&mut rows, filter);
    // `last_deployed_at` and `commit_count` are output columns of VIEW_SQL.
    let column = match filter.sort {
        ProjectSort::Name => "lower(p.name)",
        ProjectSort::CreatedAt => "p.created_at",
        ProjectSort::UpdatedAt => "p.updated_at",
        ProjectSort::LastDeployedAt => "last_deployed_at",
        ProjectSort::Commits => "commit_count",
    };
    rows.push(" ORDER BY ").push(column).push(' ').push(filter.order.sql()).push(" NULLS LAST, p.id");
    rows.push(" LIMIT ").push_bind(page.limit()).push(" OFFSET ").push_bind(page.offset());

    let mut count: QueryBuilder<Postgres> = QueryBuilder::new("SELECT count(*) FROM projects p");
    push_filters(&mut count, filter);

    tokio::try_join!(
        rows.build_query_as::<ProjectResponse>().fetch_all(pool),
        count.build_query_scalar::<i64>().fetch_one(pool),
    )
}

pub async fn find_view(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<ProjectResponse>> {
    let mut query: QueryBuilder<Postgres> = QueryBuilder::new(VIEW_SQL.as_str());
    query.push(" WHERE p.id = ").push_bind(id);
    query.build_query_as::<ProjectResponse>().fetch_optional(pool).await
}

pub async fn exists(pool: &PgPool, id: Uuid) -> sqlx::Result<bool> {
    sqlx::query_scalar!(r#"SELECT EXISTS (SELECT 1 FROM projects WHERE id = $1) AS "exists!""#, id)
        .fetch_one(pool)
        .await
}

/// Loads the row and locks it for the rest of the transaction.
pub async fn find_for_update(conn: &mut PgConnection, id: Uuid) -> sqlx::Result<Option<Project>> {
    sqlx::query_as!(
        Project,
        r#"SELECT id, name, slug, description, repository_url, default_branch,
                  status AS "status: ProjectStatus", archived_at
           FROM projects WHERE id = $1 FOR UPDATE"#,
        id,
    )
    .fetch_optional(conn)
    .await
}

/// Existing slugs equal to `base` or of the form `base-<n>`.
pub async fn slugs_like(pool: &PgPool, base: &str) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar!("SELECT slug FROM projects WHERE slug = $1 OR slug LIKE $1 || '-%'", base).fetch_all(pool).await
}

pub struct NewProject<'a> {
    pub name: &'a str,
    pub slug: &'a str,
    pub description: Option<&'a str>,
    pub repository_url: Option<&'a str>,
    pub default_branch: &'a str,
}

pub async fn insert(conn: &mut PgConnection, new: &NewProject<'_>) -> sqlx::Result<Uuid> {
    sqlx::query_scalar!(
        "INSERT INTO projects (id, name, slug, description, repository_url, default_branch)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        Uuid::now_v7(),
        new.name,
        new.slug,
        new.description,
        new.repository_url,
        new.default_branch,
    )
    .fetch_one(conn)
    .await
}

pub async fn update(conn: &mut PgConnection, project: &Project) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE projects
         SET name = $2, slug = $3, description = $4, repository_url = $5, default_branch = $6,
             status = $7, archived_at = $8
         WHERE id = $1",
        project.id,
        project.name,
        project.slug,
        project.description,
        project.repository_url,
        project.default_branch,
        project.status.as_str(),
        project.archived_at,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Deletes the project and, through `ON DELETE CASCADE`, everything it owns.
pub async fn delete(pool: &PgPool, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!("DELETE FROM projects WHERE id = $1", id).execute(pool).await?;
    Ok(result.rows_affected() > 0)
}
