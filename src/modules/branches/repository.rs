use std::sync::LazyLock;

use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use super::{
    dto::{BranchEnvironment, BranchResponse, BranchSort},
    models::Branch,
};
use crate::shared::{
    pagination::{Page, SortOrder},
    sql::{COMMIT_BRIEF_JSON, PROJECT_REF_JSON},
};

/// Read model: the branch, its project, and its commit statistics (one
/// index range scan on `commits_branch_date_idx` per branch).
static VIEW_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT b.id, b.project_id, {PROJECT_REF_JSON} AS project, b.name, b.description, \
                (p.default_branch = b.name) AS is_default, s.commit_count, s.last_commit_at, \
                b.created_at, b.updated_at \
         FROM branches b JOIN projects p ON p.id = b.project_id \
         CROSS JOIN LATERAL (SELECT count(*) AS commit_count, max(c.commit_date) AS last_commit_at \
                             FROM commits c WHERE c.branch_id = b.id) s"
    )
});

/// Latest deployment record per environment among the branch's commits.
static ENVIRONMENTS_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT DISTINCT ON (e.position, e.name, e.id) e.id, e.name, e.slug, e.color, \
                count(*) OVER (PARTITION BY e.id) AS deployment_count, \
                jsonb_build_object('id', d.id, 'deployment_title', d.deployment_title, 'status', d.status, \
                                   'deployed_at', d.deployed_at, 'commit', {COMMIT_BRIEF_JSON}) AS latest_deployment \
         FROM deployments d \
         JOIN commits c ON c.id = d.commit_id \
         JOIN environments e ON e.id = d.environment_id \
         WHERE c.branch_id = $1 \
         ORDER BY e.position, e.name, e.id, d.deployed_at DESC, d.created_at DESC"
    )
});

#[derive(Debug)]
pub struct BranchFilter {
    pub project_id: Uuid,
    /// Already escaped `ILIKE` pattern.
    pub pattern: Option<String>,
    pub sort: BranchSort,
    pub order: SortOrder,
}

fn push_filters(query: &mut QueryBuilder<Postgres>, filter: &BranchFilter) {
    query.push(" WHERE b.project_id = ").push_bind(filter.project_id);
    if let Some(pattern) = &filter.pattern {
        query.push(" AND b.name ILIKE ").push_bind(pattern);
    }
}

pub async fn list(pool: &PgPool, filter: &BranchFilter, page: Page) -> sqlx::Result<(Vec<BranchResponse>, i64)> {
    let mut rows: QueryBuilder<Postgres> = QueryBuilder::new(VIEW_SQL.as_str());
    push_filters(&mut rows, filter);
    let column = match filter.sort {
        BranchSort::LastCommit => "s.last_commit_at",
        BranchSort::Name => "b.name",
        BranchSort::CreatedAt => "b.created_at",
    };
    rows.push(" ORDER BY ").push(column).push(' ').push(filter.order.sql()).push(" NULLS LAST, b.name, b.id");
    rows.push(" LIMIT ").push_bind(page.limit()).push(" OFFSET ").push_bind(page.offset());

    let mut count: QueryBuilder<Postgres> = QueryBuilder::new("SELECT count(*) FROM branches b");
    push_filters(&mut count, filter);

    tokio::try_join!(
        rows.build_query_as::<BranchResponse>().fetch_all(pool),
        count.build_query_scalar::<i64>().fetch_one(pool),
    )
}

pub async fn find_view(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<BranchResponse>> {
    let mut query: QueryBuilder<Postgres> = QueryBuilder::new(VIEW_SQL.as_str());
    query.push(" WHERE b.id = ").push_bind(id);
    query.build_query_as::<BranchResponse>().fetch_optional(pool).await
}

pub async fn environments(pool: &PgPool, id: Uuid) -> sqlx::Result<Vec<BranchEnvironment>> {
    // Built only from the constant fragments above; the id is a bind parameter.
    sqlx::query_as::<_, BranchEnvironment>(sqlx::AssertSqlSafe(ENVIRONMENTS_SQL.as_str()))
        .bind(id)
        .fetch_all(pool)
        .await
}

pub async fn find_for_update(conn: &mut PgConnection, id: Uuid) -> sqlx::Result<Option<Branch>> {
    sqlx::query_as!(Branch, "SELECT id, project_id, name, description FROM branches WHERE id = $1 FOR UPDATE", id)
        .fetch_optional(conn)
        .await
}

/// The project a branch belongs to, if the branch exists.
pub async fn project_id_of(conn: &mut PgConnection, id: Uuid) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar!("SELECT project_id FROM branches WHERE id = $1", id).fetch_optional(conn).await
}

pub async fn insert(conn: &mut PgConnection, branch: &Branch) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO branches (id, project_id, name, description) VALUES ($1, $2, $3, $4)",
        branch.id,
        branch.project_id,
        branch.name,
        branch.description,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Creates the branch unless the project already has one with this name.
pub async fn insert_if_missing(conn: &mut PgConnection, project_id: Uuid, name: &str) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO branches (id, project_id, name) VALUES ($1, $2, $3)
         ON CONFLICT (project_id, name) DO NOTHING",
        Uuid::now_v7(),
        project_id,
        name,
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn update(conn: &mut PgConnection, branch: &Branch) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE branches SET name = $2, description = $3 WHERE id = $1",
        branch.id,
        branch.name,
        branch.description,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Keeps `projects.default_branch` pointing at a renamed branch.
pub async fn rename_default_branch(
    conn: &mut PgConnection,
    project_id: Uuid,
    old_name: &str,
    new_name: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE projects SET default_branch = $3 WHERE id = $1 AND default_branch = $2",
        project_id,
        old_name,
        new_name,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Removes the branch's commits and their deployment records (cascade delete).
pub async fn delete_commits(conn: &mut PgConnection, id: Uuid) -> sqlx::Result<u64> {
    sqlx::query!("DELETE FROM deployments WHERE commit_id IN (SELECT id FROM commits WHERE branch_id = $1)", id)
        .execute(&mut *conn)
        .await?;
    let result = sqlx::query!("DELETE FROM commits WHERE branch_id = $1", id).execute(conn).await?;
    Ok(result.rows_affected())
}

pub async fn delete(conn: &mut PgConnection, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!("DELETE FROM branches WHERE id = $1", id).execute(conn).await?;
    Ok(result.rows_affected() > 0)
}
