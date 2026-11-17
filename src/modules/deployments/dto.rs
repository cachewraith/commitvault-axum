use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::models::DeploymentStatus;
use crate::shared::{
    pagination::SortOrder,
    refs::{CommitBrief, DeploymentBrief, EnvironmentRef, ProjectRef, TagRef},
    serde_helpers, validation,
};

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateDeploymentRequest {
    /// Must belong to the project in the path.
    pub environment_id: Uuid,
    /// Must belong to the project in the path.
    pub commit_id: Uuid,
    #[validate(length(min = 1, max = 200, message = "Must be 1–200 characters"))]
    #[serde(deserialize_with = "serde_helpers::trimmed")]
    pub deployment_title: String,
    /// Plain text: what changed, commands to run, migration steps. Stored as
    /// documentation only — CommitVault never executes anything.
    #[validate(length(max = 20000, message = "Must be at most 20000 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub deployment_notes: Option<String>,
    /// Defaults to the current time.
    #[serde(default)]
    pub deployed_at: Option<DateTime<Utc>>,
    /// Defaults to `successful`.
    #[serde(default)]
    pub status: Option<DeploymentStatus>,
    #[validate(custom(function = "validation::id_list"))]
    #[serde(default)]
    pub tag_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateDeploymentRequest {
    #[serde(default)]
    pub environment_id: Option<Uuid>,
    #[serde(default)]
    pub commit_id: Option<Uuid>,
    #[validate(length(min = 1, max = 200, message = "Must be 1–200 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub deployment_title: Option<String>,
    #[validate(length(max = 20000, message = "Must be at most 20000 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::patch_text")]
    #[schema(value_type = Option<String>)]
    pub deployment_notes: Option<Option<String>>,
    #[serde(default)]
    pub deployed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub status: Option<DeploymentStatus>,
    /// Replaces the deployment's tags.
    #[validate(custom(function = "validation::id_list"))]
    #[serde(default)]
    pub tag_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentSort {
    #[default]
    DeployedAt,
    CreatedAt,
}

#[derive(Debug, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListDeploymentsQuery {
    pub project_id: Option<Uuid>,
    pub environment_id: Option<Uuid>,
    pub commit_id: Option<Uuid>,
    pub status: Option<DeploymentStatus>,
    /// Deployed at or after this instant.
    pub date_from: Option<DateTime<Utc>>,
    /// Deployed at or before this instant.
    pub date_to: Option<DateTime<Utc>>,
    /// Matches title, notes, commit message and commit hash prefix.
    #[validate(length(max = 200))]
    pub search: Option<String>,
    pub tag_id: Option<Uuid>,
    pub sort: Option<DeploymentSort>,
    pub order: Option<SortOrder>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct DeploymentResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    #[sqlx(json)]
    pub project: ProjectRef,
    #[sqlx(json)]
    pub environment: EnvironmentRef,
    #[sqlx(json)]
    pub commit: CommitBrief,
    pub deployment_title: String,
    pub deployment_notes: Option<String>,
    pub deployed_at: DateTime<Utc>,
    pub status: DeploymentStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[sqlx(json)]
    pub tags: Vec<TagRef>,
}

#[derive(Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CompareEnvironmentsQuery {
    /// First environment.
    pub base: Uuid,
    /// Second environment.
    pub target: Uuid,
    /// Count records of every status instead of only `successful` ones.
    pub include_unsuccessful: Option<bool>,
}

/// Side-by-side view of the deployment *records* of two environments.
#[derive(Debug, Serialize, ToSchema)]
pub struct EnvironmentComparison {
    pub base: ComparisonSide,
    pub target: ComparisonSide,
    /// Statuses that were taken into account.
    pub statuses_considered: Vec<DeploymentStatus>,
    pub counts: ComparisonCounts,
    /// Commits with a record in `base` and none in `target`.
    pub only_in_base: Vec<ComparedCommit>,
    /// Commits with a record in `target` and none in `base`.
    pub only_in_target: Vec<ComparedCommit>,
    /// Commits with a record in both environments.
    pub in_both: Vec<SharedCommit>,
    /// `true` when the commit lists were cut off at the row limit
    /// (`counts` always reflects the full result).
    pub truncated: bool,
    /// How to read this comparison; shown verbatim in the UI.
    pub notice: &'static str,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ComparisonSide {
    pub environment: EnvironmentRef,
    /// Latest record among the considered statuses.
    pub latest_deployment: Option<DeploymentBrief>,
    /// Distinct commits with a considered record in this environment.
    pub recorded_commit_count: i64,
}

#[derive(Debug, Clone, Copy, Serialize, sqlx::FromRow, ToSchema)]
pub struct ComparisonCounts {
    pub only_in_base: i64,
    pub only_in_target: i64,
    pub in_both: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ComparedCommit {
    pub commit: CommitBrief,
    pub deployment_count: i64,
    pub last_deployed_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SharedCommit {
    pub commit: CommitBrief,
    pub base_last_deployed_at: DateTime<Utc>,
    pub target_last_deployed_at: DateTime<Utc>,
}
