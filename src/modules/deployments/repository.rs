use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use super::{
    dto::{ComparisonCounts, DeploymentResponse, DeploymentSort},
    models::{Deployment, DeploymentStatus},
};
use crate::{
    modules::tags::models::TagTarget,
    shared::{
        pagination::{Page, SortOrder},
        refs::CommitBrief,
        sql::{COMMIT_BRIEF_JSON, ENVIRONMENT_REF_JSON, PROJECT_REF_JSON},
    },
};

const FROM_SQL: &str = " FROM deployments d \
     JOIN projects p ON p.id = d.project_id \
     JOIN environments e ON e.id = d.environment_id \
     JOIN commits c ON c.id = d.commit_id";

static VIEW_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT d.id, d.project_id, {PROJECT_REF_JSON} AS project, {ENVIRONMENT_REF_JSON} AS environment, \
                {COMMIT_BRIEF_JSON} AS commit, d.deployment_title, d.deployment_notes, d.deployed_at, d.status, \
                d.created_at, d.updated_at, {tags} AS tags{FROM_SQL}",
        tags = TagTarget::Deployment.json_sql("d.id"),
    )
});

#[derive(Debug, Default)]
pub struct DeploymentFilter {
    pub project_id: Option<Uuid>,
    pub environment_id: Option<Uuid>,
    pub commit_id: Option<Uuid>,
    pub status: Option<DeploymentStatus>,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
    /// Escaped `%term%` pattern.
    pub text_pattern: Option<String>,
    /// Escaped `term%` pattern matched against the commit hash.
    pub search_hash_prefix: Option<String>,
    pub tag_id: Option<Uuid>,
    pub sort: DeploymentSort,
    pub order: SortOrder,
}

fn push_filters(query: &mut QueryBuilder<Postgres>, filter: &DeploymentFilter) {
    query.push(" WHERE true");
    if let Some(project_id) = filter.project_id {
        query.push(" AND d.project_id = ").push_bind(project_id);
    }
    if let Some(environment_id) = filter.environment_id {
        query.push(" AND d.environment_id = ").push_bind(environment_id);
    }
    if let Some(commit_id) = filter.commit_id {
        query.push(" AND d.commit_id = ").push_bind(commit_id);
    }
    if let Some(status) = filter.status {
        query.push(" AND d.status = ").push_bind(status.as_str());
    }
    if let Some(from) = filter.date_from {
        query.push(" AND d.deployed_at >= ").push_bind(from);
    }
    if let Some(to) = filter.date_to {
        query.push(" AND d.deployed_at <= ").push_bind(to);
    }
    if let Some(pattern) = &filter.text_pattern {
        query.push(" AND (d.deployment_title ILIKE ").push_bind(pattern);
        query.push(" OR d.deployment_notes ILIKE ").push_bind(pattern);
        query.push(" OR c.commit_message ILIKE ").push_bind(pattern);
        if let Some(prefix) = &filter.search_hash_prefix {
            query.push(" OR c.commit_hash LIKE ").push_bind(prefix);
        }
        query.push(")");
    }
    if let Some(tag_id) = filter.tag_id {
        query.push(" AND EXISTS (SELECT 1 FROM deployment_tags dt WHERE dt.deployment_id = d.id AND dt.tag_id = ");
        query.push_bind(tag_id).push(")");
    }
}

pub async fn list(
    pool: &PgPool,
    filter: &DeploymentFilter,
    page: Page,
) -> sqlx::Result<(Vec<DeploymentResponse>, i64)> {
    let mut rows: QueryBuilder<Postgres> = QueryBuilder::new(VIEW_SQL.as_str());
    push_filters(&mut rows, filter);
    let column = match filter.sort {
        DeploymentSort::DeployedAt => "d.deployed_at",
        DeploymentSort::CreatedAt => "d.created_at",
        DeploymentSort::Title => "lower(d.deployment_title)",
        DeploymentSort::Status => {
            "CASE d.status WHEN 'failed' THEN 0 WHEN 'rolled_back' THEN 1 WHEN 'pending' THEN 2 ELSE 3 END"
        }
    };
    rows.push(" ORDER BY ").push(column).push(' ').push(filter.order.sql()).push(", d.id DESC");
    rows.push(" LIMIT ").push_bind(page.limit()).push(" OFFSET ").push_bind(page.offset());

    let mut count: QueryBuilder<Postgres> = QueryBuilder::new(format!("SELECT count(*){FROM_SQL}"));
    push_filters(&mut count, filter);

    tokio::try_join!(
        rows.build_query_as::<DeploymentResponse>().fetch_all(pool),
        count.build_query_scalar::<i64>().fetch_one(pool),
    )
}

pub async fn find_view(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<DeploymentResponse>> {
    let mut query: QueryBuilder<Postgres> = QueryBuilder::new(VIEW_SQL.as_str());
    query.push(" WHERE d.id = ").push_bind(id);
    query.build_query_as::<DeploymentResponse>().fetch_optional(pool).await
}

pub async fn find_for_update(conn: &mut PgConnection, id: Uuid) -> sqlx::Result<Option<Deployment>> {
    sqlx::query_as!(
        Deployment,
        r#"SELECT id, project_id, environment_id, commit_id, deployment_title, deployment_notes, deployed_at,
                  status AS "status: DeploymentStatus"
           FROM deployments WHERE id = $1 FOR UPDATE"#,
        id,
    )
    .fetch_optional(conn)
    .await
}

/// `(environment belongs to project, commit belongs to project)`.
pub async fn references_belong_to_project(
    conn: &mut PgConnection,
    project_id: Uuid,
    environment_id: Uuid,
    commit_id: Uuid,
) -> sqlx::Result<(bool, bool)> {
    let row = sqlx::query!(
        r#"SELECT EXISTS (SELECT 1 FROM environments WHERE id = $2 AND project_id = $1) AS "environment_ok!",
                  EXISTS (SELECT 1 FROM commits WHERE id = $3 AND project_id = $1) AS "commit_ok!""#,
        project_id,
        environment_id,
        commit_id,
    )
    .fetch_one(conn)
    .await?;
    Ok((row.environment_ok, row.commit_ok))
}

pub async fn insert(conn: &mut PgConnection, deployment: &Deployment) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO deployments (id, project_id, environment_id, commit_id, deployment_title, deployment_notes,
                                  deployed_at, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        deployment.id,
        deployment.project_id,
        deployment.environment_id,
        deployment.commit_id,
        deployment.deployment_title,
        deployment.deployment_notes,
        deployment.deployed_at,
        deployment.status.as_str(),
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn update(conn: &mut PgConnection, deployment: &Deployment) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE deployments
         SET environment_id = $2, commit_id = $3, deployment_title = $4, deployment_notes = $5,
             deployed_at = $6, status = $7
         WHERE id = $1",
        deployment.id,
        deployment.environment_id,
        deployment.commit_id,
        deployment.deployment_title,
        deployment.deployment_notes,
        deployment.deployed_at,
        deployment.status.as_str(),
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!("DELETE FROM deployments WHERE id = $1", id).execute(pool).await?;
    Ok(result.rows_affected() > 0)
}

/// One commit with its recorded-deployment aggregates in two environments.
#[derive(Debug, sqlx::FromRow)]
pub struct ComparisonRow {
    #[sqlx(json)]
    pub commit: CommitBrief,
    pub base_count: i64,
    pub base_last: Option<DateTime<Utc>>,
    pub target_count: i64,
    pub target_last: Option<DateTime<Utc>>,
}

/// Commits that have a (considered) deployment record in `base` and/or
/// `target`, most recently deployed first.
pub async fn compare(
    pool: &PgPool,
    project_id: Uuid,
    base: Uuid,
    target: Uuid,
    all_statuses: bool,
    limit: i64,
) -> sqlx::Result<Vec<ComparisonRow>> {
    let sql = format!(
        "SELECT {COMMIT_BRIEF_JSON} AS commit, \
                count(*) FILTER (WHERE d.environment_id = $2) AS base_count, \
                max(d.deployed_at) FILTER (WHERE d.environment_id = $2) AS base_last, \
                count(*) FILTER (WHERE d.environment_id = $3) AS target_count, \
                max(d.deployed_at) FILTER (WHERE d.environment_id = $3) AS target_last \
         FROM deployments d JOIN commits c ON c.id = d.commit_id \
         WHERE d.project_id = $1 AND d.environment_id IN ($2, $3) AND ($4 OR d.status = 'successful') \
         GROUP BY c.id ORDER BY max(d.deployed_at) DESC, c.id LIMIT $5"
    );
    // Built only from constants; every value is a bind parameter.
    sqlx::query_as::<_, ComparisonRow>(sqlx::AssertSqlSafe(sql))
        .bind(project_id)
        .bind(base)
        .bind(target)
        .bind(all_statuses)
        .bind(limit)
        .fetch_all(pool)
        .await
}

pub async fn compare_counts(
    pool: &PgPool,
    project_id: Uuid,
    base: Uuid,
    target: Uuid,
    all_statuses: bool,
) -> sqlx::Result<ComparisonCounts> {
    sqlx::query_as!(
        ComparisonCounts,
        r#"SELECT count(*) FILTER (WHERE in_base AND NOT in_target) AS "only_in_base!",
                  count(*) FILTER (WHERE in_target AND NOT in_base) AS "only_in_target!",
                  count(*) FILTER (WHERE in_base AND in_target) AS "in_both!"
           FROM (SELECT bool_or(environment_id = $2) AS in_base, bool_or(environment_id = $3) AS in_target
                 FROM deployments
                 WHERE project_id = $1 AND environment_id IN ($2, $3) AND ($4 OR status = 'successful')
                 GROUP BY commit_id) per_commit"#,
        project_id,
        base,
        target,
        all_statuses,
    )
    .fetch_one(pool)
    .await
}
