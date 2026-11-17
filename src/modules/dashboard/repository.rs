use sqlx::PgPool;

use super::dto::{DailyDeployments, DashboardTotals, ProjectActivity};
use crate::shared::sql::PROJECT_REF_JSON;

pub async fn totals(pool: &PgPool) -> sqlx::Result<DashboardTotals> {
    sqlx::query_as!(
        DashboardTotals,
        r#"SELECT (SELECT count(*) FROM projects) AS "projects!",
                  (SELECT count(*) FROM projects WHERE status = 'active') AS "active_projects!",
                  (SELECT count(*) FROM commits) AS "commits!",
                  (SELECT count(*) FROM environments) AS "environments!",
                  (SELECT count(*) FROM deployments) AS "deployments!",
                  (SELECT count(*) FROM releases) AS "releases!",
                  (SELECT count(*) FROM notes) AS "notes!",
                  (SELECT count(*) FROM tags) AS "tags!""#
    )
    .fetch_one(pool)
    .await
}

pub async fn project_activity(pool: &PgPool, limit: i64) -> sqlx::Result<Vec<ProjectActivity>> {
    let sql = format!(
        "SELECT {PROJECT_REF_JSON} AS project, cs.commit_count, ds.deployment_count, cs.last_commit_at, \
                ds.last_deployment_at, greatest(p.updated_at, cs.last_commit_at, ds.last_deployment_at) AS last_activity_at \
         FROM projects p \
         CROSS JOIN LATERAL (SELECT count(*) AS commit_count, max(c.commit_date) AS last_commit_at \
                             FROM commits c WHERE c.project_id = p.id) cs \
         CROSS JOIN LATERAL (SELECT count(*) AS deployment_count, max(d.deployed_at) AS last_deployment_at \
                             FROM deployments d WHERE d.project_id = p.id) ds \
         WHERE p.status = 'active' \
         ORDER BY last_activity_at DESC, p.id LIMIT $1"
    );
    // Built only from constants; the limit is a bind parameter.
    sqlx::query_as::<_, ProjectActivity>(sqlx::AssertSqlSafe(sql)).bind(limit).fetch_all(pool).await
}

/// One row per UTC day for the last `days` days, including days without records.
pub async fn deployment_activity(pool: &PgPool, days: i32) -> sqlx::Result<Vec<DailyDeployments>> {
    sqlx::query_as!(
        DailyDeployments,
        r#"WITH bounds AS (SELECT (now() AT TIME ZONE 'utc')::date AS today),
                days AS (SELECT generate_series(today - ($1::int - 1), today, interval '1 day')::date AS day FROM bounds),
                agg AS (SELECT (d.deployed_at AT TIME ZONE 'utc')::date AS day, d.status, count(*) AS n
                        FROM deployments d, bounds
                        WHERE d.deployed_at >= (bounds.today - $1::int)::timestamp AT TIME ZONE 'utc'
                        GROUP BY 1, 2)
           SELECT days.day AS "date!",
                  coalesce(sum(agg.n) FILTER (WHERE agg.status = 'successful'), 0)::bigint AS "successful!",
                  coalesce(sum(agg.n) FILTER (WHERE agg.status = 'failed'), 0)::bigint AS "failed!",
                  coalesce(sum(agg.n) FILTER (WHERE agg.status = 'rolled_back'), 0)::bigint AS "rolled_back!",
                  coalesce(sum(agg.n) FILTER (WHERE agg.status = 'pending'), 0)::bigint AS "pending!"
           FROM days LEFT JOIN agg ON agg.day = days.day
           GROUP BY days.day ORDER BY days.day"#,
        days,
    )
    .fetch_all(pool)
    .await
}
