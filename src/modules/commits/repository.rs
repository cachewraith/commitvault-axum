use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use super::{
    dto::{BranchSummary, CommitDeploymentFilter, CommitResponse, CommitSort},
    models::Commit,
};
use crate::{
    modules::tags::models::TagTarget,
    shared::{
        pagination::{Page, SortOrder},
        sql::PROJECT_REF_JSON,
    },
};

/// Read model. The related data (tags, per-environment deployment summary,
/// releases) is aggregated by correlated sub-selects, so a page of commits is
/// always exactly one round trip — no N+1.
static VIEW_SQL: LazyLock<String> = LazyLock::new(|| {
    format!(
        "SELECT c.id, c.project_id, {PROJECT_REF_JSON} AS project, c.branch_name, c.commit_hash, c.short_hash, \
                c.commit_message, c.description, c.commit_date, c.author_name, c.external_url, \
                c.created_at, c.updated_at, {tags} AS tags, \
                (SELECT count(*) FROM deployments d WHERE d.commit_id = c.id) AS deployment_count, \
                (SELECT coalesce(jsonb_agg(x.obj ORDER BY x.position, x.name), '[]'::jsonb) FROM ( \
                    SELECT e.position, e.name, jsonb_build_object( \
                        'id', e.id, 'name', e.name, 'slug', e.slug, 'color', e.color, \
                        'deployment_count', count(*), \
                        'latest_deployed_at', max(d.deployed_at), \
                        'latest_status', (array_agg(d.status ORDER BY d.deployed_at DESC, d.created_at DESC))[1]) AS obj \
                    FROM deployments d JOIN environments e ON e.id = d.environment_id \
                    WHERE d.commit_id = c.id GROUP BY e.id) x) AS environments, \
                (SELECT coalesce(jsonb_agg(jsonb_build_object('id', r.id, 'version', r.version, 'title', r.title, \
                        'status', r.status) ORDER BY r.created_at DESC), '[]'::jsonb) \
                 FROM release_commits rc JOIN releases r ON r.id = rc.release_id \
                 WHERE rc.commit_id = c.id) AS releases \
         FROM commits c JOIN projects p ON p.id = c.project_id",
        tags = TagTarget::Commit.json_sql("c.id"),
    )
});

#[derive(Debug, Default)]
pub struct CommitFilter {
    pub project_id: Option<Uuid>,
    /// Escaped `%term%` pattern for text columns.
    pub text_pattern: Option<String>,
    /// Escaped `term%` pattern matched against the hash (search box).
    pub search_hash_prefix: Option<String>,
    pub branch: Option<String>,
    /// Escaped `prefix%` pattern (explicit hash filter).
    pub hash_prefix: Option<String>,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
    pub deployment: Option<CommitDeploymentFilter>,
    pub environment_id: Option<Uuid>,
    pub tag_id: Option<Uuid>,
    pub release_id: Option<Uuid>,
    pub sort: CommitSort,
    pub order: SortOrder,
}

fn push_filters(query: &mut QueryBuilder<Postgres>, filter: &CommitFilter) {
    query.push(" WHERE true");
    if let Some(project_id) = filter.project_id {
        query.push(" AND c.project_id = ").push_bind(project_id);
    }
    if let Some(pattern) = &filter.text_pattern {
        query.push(" AND (c.commit_message ILIKE ").push_bind(pattern);
        query.push(" OR c.branch_name ILIKE ").push_bind(pattern);
        query.push(" OR c.author_name ILIKE ").push_bind(pattern);
        query.push(" OR c.description ILIKE ").push_bind(pattern);
        if let Some(prefix) = &filter.search_hash_prefix {
            query.push(" OR c.commit_hash LIKE ").push_bind(prefix);
        }
        query.push(")");
    }
    if let Some(branch) = &filter.branch {
        query.push(" AND c.branch_name = ").push_bind(branch);
    }
    if let Some(prefix) = &filter.hash_prefix {
        query.push(" AND c.commit_hash LIKE ").push_bind(prefix);
    }
    if let Some(from) = filter.date_from {
        query.push(" AND c.commit_date >= ").push_bind(from);
    }
    if let Some(to) = filter.date_to {
        query.push(" AND c.commit_date <= ").push_bind(to);
    }
    // An environment on its own means "has a record in that environment".
    let deployment = filter.deployment.or(filter.environment_id.map(|_| CommitDeploymentFilter::Any));
    if let Some(deployment) = deployment {
        query.push(if deployment == CommitDeploymentFilter::None { " AND NOT EXISTS (" } else { " AND EXISTS (" });
        query.push("SELECT 1 FROM deployments d WHERE d.commit_id = c.id");
        if let Some(environment_id) = filter.environment_id {
            query.push(" AND d.environment_id = ").push_bind(environment_id);
        }
        if let Some(status) = deployment.status() {
            query.push(" AND d.status = ").push_bind(status.as_str());
        }
        query.push(")");
    }
    if let Some(tag_id) = filter.tag_id {
        query.push(" AND EXISTS (SELECT 1 FROM commit_tags ct WHERE ct.commit_id = c.id AND ct.tag_id = ");
        query.push_bind(tag_id).push(")");
    }
    if let Some(release_id) = filter.release_id {
        query.push(" AND EXISTS (SELECT 1 FROM release_commits rc WHERE rc.commit_id = c.id AND rc.release_id = ");
        query.push_bind(release_id).push(")");
    }
}

pub async fn list(pool: &PgPool, filter: &CommitFilter, page: Page) -> sqlx::Result<(Vec<CommitResponse>, i64)> {
    let mut rows: QueryBuilder<Postgres> = QueryBuilder::new(VIEW_SQL.as_str());
    push_filters(&mut rows, filter);
    let column = match filter.sort {
        CommitSort::CommitDate => "c.commit_date",
        CommitSort::CreatedAt => "c.created_at",
        CommitSort::BranchName => "c.branch_name",
    };
    rows.push(" ORDER BY ").push(column).push(' ').push(filter.order.sql()).push(", c.id DESC");
    rows.push(" LIMIT ").push_bind(page.limit()).push(" OFFSET ").push_bind(page.offset());

    let mut count: QueryBuilder<Postgres> = QueryBuilder::new("SELECT count(*) FROM commits c");
    push_filters(&mut count, filter);

    tokio::try_join!(
        rows.build_query_as::<CommitResponse>().fetch_all(pool),
        count.build_query_scalar::<i64>().fetch_one(pool),
    )
}

pub async fn find_view(pool: &PgPool, id: Uuid) -> sqlx::Result<Option<CommitResponse>> {
    let mut query: QueryBuilder<Postgres> = QueryBuilder::new(VIEW_SQL.as_str());
    query.push(" WHERE c.id = ").push_bind(id);
    query.build_query_as::<CommitResponse>().fetch_optional(pool).await
}

pub async fn find_for_update(conn: &mut PgConnection, id: Uuid) -> sqlx::Result<Option<Commit>> {
    sqlx::query_as!(
        Commit,
        "SELECT id, project_id, branch_name, commit_hash, short_hash, commit_message, description,
                commit_date, author_name, external_url
         FROM commits WHERE id = $1 FOR UPDATE",
        id,
    )
    .fetch_optional(conn)
    .await
}

/// The project a commit belongs to, if the commit exists.
pub async fn project_id_of(conn: &mut PgConnection, id: Uuid) -> sqlx::Result<Option<Uuid>> {
    sqlx::query_scalar!("SELECT project_id FROM commits WHERE id = $1", id).fetch_optional(conn).await
}

pub async fn insert(conn: &mut PgConnection, commit: &Commit) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO commits (id, project_id, branch_name, commit_hash, short_hash, commit_message, description,
                              commit_date, author_name, external_url)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        commit.id,
        commit.project_id,
        commit.branch_name,
        commit.commit_hash,
        commit.short_hash,
        commit.commit_message,
        commit.description,
        commit.commit_date,
        commit.author_name,
        commit.external_url,
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn update(conn: &mut PgConnection, commit: &Commit) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE commits
         SET branch_name = $2, commit_hash = $3, short_hash = $4, commit_message = $5, description = $6,
             commit_date = $7, author_name = $8, external_url = $9
         WHERE id = $1",
        commit.id,
        commit.branch_name,
        commit.commit_hash,
        commit.short_hash,
        commit.commit_message,
        commit.description,
        commit.commit_date,
        commit.author_name,
        commit.external_url,
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn delete_deployments(conn: &mut PgConnection, commit_id: Uuid) -> sqlx::Result<u64> {
    let result = sqlx::query!("DELETE FROM deployments WHERE commit_id = $1", commit_id).execute(conn).await?;
    Ok(result.rows_affected())
}

pub async fn delete(conn: &mut PgConnection, id: Uuid) -> sqlx::Result<bool> {
    let result = sqlx::query!("DELETE FROM commits WHERE id = $1", id).execute(conn).await?;
    Ok(result.rows_affected() > 0)
}

/// Distinct branch names of a project, most recently used first.
pub async fn branches(pool: &PgPool, project_id: Uuid, limit: i64) -> sqlx::Result<Vec<BranchSummary>> {
    sqlx::query_as!(
        BranchSummary,
        r#"SELECT branch_name AS name, count(*) AS "commit_count!", max(commit_date) AS "last_commit_at!"
           FROM commits WHERE project_id = $1
           GROUP BY branch_name ORDER BY max(commit_date) DESC, branch_name LIMIT $2"#,
        project_id,
        limit,
    )
    .fetch_all(pool)
    .await
}
