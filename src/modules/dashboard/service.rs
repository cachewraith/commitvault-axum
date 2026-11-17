use sqlx::PgPool;

use super::{dto::DashboardResponse, repository};
use crate::{
    errors::AppResult,
    modules::{commits, deployments, environments, notes},
    shared::pagination::Page,
};

const RECENT_DEPLOYMENTS: u32 = 8;
const RECENT_COMMITS: u32 = 8;
const RECENT_NOTES: u32 = 6;
const ACTIVE_PROJECTS: i64 = 8;
const ENVIRONMENTS: u32 = 12;
const ACTIVITY_DAYS: i32 = 30;

/// Everything the dashboard shows, from live data. The seven queries are
/// independent, so they run concurrently on the pool.
pub async fn overview(pool: &PgPool) -> AppResult<DashboardResponse> {
    let deployment_filter = deployments::repository::DeploymentFilter::default();
    let commit_filter = commits::repository::CommitFilter::default();
    let environment_filter = environments::repository::EnvironmentFilter {
        active_projects_only: true,
        recent_first: true,
        ..Default::default()
    };

    let (
        totals,
        recent_deployments,
        recent_commits,
        recent_notes,
        project_activity,
        environment_overview,
        deployment_activity,
    ) = tokio::try_join!(
        async { Ok(repository::totals(pool).await?) },
        async { Ok(deployments::repository::list(pool, &deployment_filter, Page::first(RECENT_DEPLOYMENTS)).await?.0) },
        async { Ok(commits::repository::list(pool, &commit_filter, Page::first(RECENT_COMMITS)).await?.0) },
        notes::service::recent(pool, RECENT_NOTES),
        async { Ok(repository::project_activity(pool, ACTIVE_PROJECTS).await?) },
        async { Ok(environments::repository::list(pool, &environment_filter, Page::first(ENVIRONMENTS)).await?.0) },
        async { Ok(repository::deployment_activity(pool, ACTIVITY_DAYS).await?) },
    )?;

    Ok(DashboardResponse {
        totals,
        recent_deployments,
        recent_commits,
        recent_notes,
        project_activity,
        environment_overview,
        deployment_activity,
    })
}
