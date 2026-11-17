use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    modules::{
        commits::dto::CommitResponse, deployments::dto::DeploymentResponse, environments::dto::EnvironmentResponse,
        notes::dto::NoteResponse,
    },
    shared::refs::ProjectRef,
};

#[derive(Debug, Serialize, ToSchema)]
pub struct DashboardResponse {
    pub totals: DashboardTotals,
    /// Latest deployment records, newest first.
    pub recent_deployments: Vec<DeploymentResponse>,
    /// Most recently dated commits.
    pub recent_commits: Vec<CommitResponse>,
    /// Most recently edited notes.
    pub recent_notes: Vec<NoteResponse>,
    /// Active projects ordered by their latest activity.
    pub project_activity: Vec<ProjectActivity>,
    /// Environments of active projects with their latest recorded deployment,
    /// most recently deployed first.
    pub environment_overview: Vec<EnvironmentResponse>,
    /// Deployment records per UTC day for the last 30 days (oldest first).
    pub deployment_activity: Vec<DailyDeployments>,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct DashboardTotals {
    pub projects: i64,
    pub active_projects: i64,
    pub commits: i64,
    pub environments: i64,
    pub deployments: i64,
    pub releases: i64,
    pub notes: i64,
    pub tags: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct ProjectActivity {
    #[sqlx(json)]
    pub project: ProjectRef,
    pub commit_count: i64,
    pub deployment_count: i64,
    pub last_commit_at: Option<DateTime<Utc>>,
    pub last_deployment_at: Option<DateTime<Utc>>,
    /// Latest of: project update, last commit date, last deployment.
    pub last_activity_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct DailyDeployments {
    pub date: NaiveDate,
    pub successful: i64,
    pub failed: i64,
    pub rolled_back: i64,
    pub pending: i64,
}
