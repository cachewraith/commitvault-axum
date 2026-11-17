use std::sync::LazyLock;

use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use super::{dto::EnvironmentResponse, models::Environment};
use crate::shared::{
    pagination::Page,
    sql::{COMMIT_BRIEF_JSON, PROJECT_REF_JSON},
};

/// Read model: the environment, its project, and its latest recorded
/// deployments. Both sub-selects walk `deployments_environment_date_idx`.
static VIEW_SQL: LazyLock<String> = LazyLock::new(|| {
    let latest = |extra: &str| {
        format!(
            "(SELECT jsonb_build_object('id', d.id, 'deployment_title', d.deployment_title, 'status', d.status, \
                     'deployed_at', d.deployed_at, 'commit', {COMMIT_BRIEF_JSON}) \
              FROM deployments d JOIN commits c ON c.id = d.commit_id \
              WHERE d.environment_id = e.id{extra} \
              ORDER BY d.deployed_at DESC, d.created_at DESC LIMIT 1)"
        )
    };
    format!(
        "SELECT e.id, e.project_id, {PROJECT_REF_JSON} AS project, e.name, e.slug, e.description, e.color, \
                e.server_label, e.position, e.created_at, e.updated_at, \
                (SELECT count(*) FROM deployments d WHERE d.environment_id = e.id) AS deployment_count, \
                {any} AS latest_deployment, {successful} AS latest_successful_deployment \
         FROM environments e JOIN projects p ON p.id = e.project_id",
        any = latest(""),
        successful = latest(" AND d.status = 'successful'"),
    )
});

#[derive(Debug, Default)]
pub struct EnvironmentFilter {
    pub project_id: Option<Uuid>,
    /// Already escaped `ILIKE` pattern.
    pub pattern: Option<String>,
    /// Only environments of non-archived projects.
    pub active_projects_only: bool,
    /// Most recently deployed first instead of project/position order.
    pub recent_first: bool,
}

fn push_filters(query: &mut QueryBuilder<Postgres>, filter: &EnvironmentFilter) {
    query.push(" WHERE true");
    if let Some(project_id) = filter.project_id {
        query.push(" AND e.project_id = ").push_bind(project_id);
    }
    if let Some(pattern) = &filter.pattern {
        query.push(" AND (e.name ILIKE ").push_bind(pattern);
        query.push(" OR e.server_label ILIKE ").push_bind(pattern).push(")");
    }
    if filter.active_projects_only {
        query.push(" AND p.status = 'active'");
    }
}

pub async fn list(
    pool: &PgPool,
    filter: &EnvironmentFilter,
    page: Page,
) -> sqlx::Result<(Vec<EnvironmentResponse>, i64)> {
    let mut rows: QueryBuilder<Postgres> = QueryBuilder::new(VIEW_SQL.as_str());
    push_filters(&mut rows, filter);
    rows.push(if filter.recent_first {
        " ORDER BY (SELECT max(d.deployed_at) FROM deployments d WHERE d.environment_id = e.id) DESC NULLS LAST, e.id"
    } else {
        " ORDER BY lower(p.name), e.position, e.created_at, e.id"
    });
    rows.push(" LIMIT ").push_bind(page.limit()).push(" OFFSET ").push_bind(page.offset());

    let mut count: QueryBuilder<Postgres> =
        QueryBuilder::new("SELECT count(*) FROM environments e JOIN projects p ON p.id = e.project_id");
    push_filters(&mut count, filter);

    tokio::try_join!(
        rows.build_query_as::<EnvironmentResponse>().fetch_all(pool),
        count.build_query_scalar::<i64>().fetch_one(pool),
    )
}

/// All environments of one project in display order (bounded by
/// `MAX_ENVIRONMENTS_PER_PROJECT`).
pub async fn list_for_project(pool: &PgPool, project_id: Uuid) -> sqlx::Result<Vec<EnvironmentResponse>> {
    let mut query: QueryBuilder<Postgres> = QueryBuilder::new(VIEW_SQL.as_str());
    query.push(" WHERE e.project_id = ").push_bind(project_id).push(" ORDER BY e.position, e.created_at, e.id");
    query.build_query_as::<EnvironmentResponse>().fetch_all(pool).await
}

pub async fn find_view(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<EnvironmentResponse>> {
    let mut query: QueryBuilder<Postgres> = QueryBuilder::new(VIEW_SQL.as_str());
    query.push(" WHERE e.id = ").push_bind(id);
    query.build_query_as::<EnvironmentResponse>().fetch_optional(pool).await
}

pub async fn find_for_update(conn: &mut PgConnection, id: Uuid) -> sqlx::Result<Option<Environment>> {
    sqlx::query_as!(
        Environment,
        "SELECT id, project_id, name, slug, description, color, server_label FROM environments WHERE id = $1 FOR UPDATE",
        id,
    )
    .fetch_optional(conn)
    .await
}

/// Ids of the project's environments, locked for the transaction.
pub async fn ids_for_update(conn: &mut PgConnection, project_id: Uuid) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar!("SELECT id FROM environments WHERE project_id = $1 ORDER BY position FOR UPDATE", project_id)
        .fetch_all(conn)
        .await
}

/// `(count, highest position)` for a project.
pub async fn count_and_max_position(conn: &mut PgConnection, project_id: Uuid) -> sqlx::Result<(i64, i32)> {
    let row = sqlx::query!(
        r#"SELECT count(*) AS "count!", coalesce(max(position), -1) AS "max_position!" FROM environments WHERE project_id = $1"#,
        project_id,
    )
    .fetch_one(conn)
    .await?;
    Ok((row.count, row.max_position))
}

pub async fn insert(conn: &mut PgConnection, environment: &Environment, position: i32) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO environments (id, project_id, name, slug, description, color, server_label, position)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        environment.id,
        environment.project_id,
        environment.name,
        environment.slug,
        environment.description,
        environment.color,
        environment.server_label,
        position,
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn update(conn: &mut PgConnection, environment: &Environment) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE environments SET name = $2, slug = $3, description = $4, color = $5, server_label = $6 WHERE id = $1",
        environment.id,
        environment.name,
        environment.slug,
        environment.description,
        environment.color,
        environment.server_label,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Writes `position = index` for every id, in one statement.
pub async fn set_positions(conn: &mut PgConnection, project_id: Uuid, ordered_ids: &[Uuid]) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE environments e SET position = (o.ord - 1)::int
         FROM unnest($2::uuid[]) WITH ORDINALITY AS o(id, ord)
         WHERE e.id = o.id AND e.project_id = $1",
        project_id,
        ordered_ids,
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn delete_deployments(conn: &mut PgConnection, environment_id: Uuid) -> sqlx::Result<u64> {
    let result =
        sqlx::query!("DELETE FROM deployments WHERE environment_id = $1", environment_id).execute(conn).await?;
    Ok(result.rows_affected())
}

pub async fn delete(conn: &mut PgConnection, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!("DELETE FROM environments WHERE id = $1", id).execute(conn).await?;
    Ok(result.rows_affected() > 0)
}
