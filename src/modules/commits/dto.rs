use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::{
    modules::deployments::models::DeploymentStatus,
    shared::{
        pagination::SortOrder,
        refs::{BranchRef, ProjectRef, ReleaseRef, TagRef},
        serde_helpers, validation,
    },
};

pub const SHORT_HASH_LEN: usize = 7;

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateCommitRequest {
    /// A branch of the same project (`POST /projects/{project_id}/branches` creates one).
    pub branch_id: Uuid,
    /// Full or abbreviated hash: 7–64 hex characters (stored lower-case).
    #[validate(custom(function = "validation::commit_hash"))]
    #[serde(deserialize_with = "serde_helpers::lowercase")]
    pub commit_hash: String,
    /// Defaults to the first 7 characters of `commit_hash`.
    #[validate(custom(function = "validation::short_hash"))]
    #[serde(default, deserialize_with = "serde_helpers::lowercase_opt")]
    pub short_hash: Option<String>,
    #[validate(length(min = 1, max = 2000, message = "Must be 1–2000 characters"))]
    #[serde(deserialize_with = "serde_helpers::trimmed")]
    pub commit_message: String,
    #[validate(length(max = 20000, message = "Must be at most 20000 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub description: Option<String>,
    pub commit_date: DateTime<Utc>,
    #[validate(length(max = 200, message = "Must be at most 200 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub author_name: Option<String>,
    /// Link to the commit on the Git host (http/https only).
    #[validate(custom(function = "validation::http_url"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub external_url: Option<String>,
    #[validate(custom(function = "validation::id_list"))]
    #[serde(default)]
    pub tag_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateCommitRequest {
    /// Move the commit to another branch of the same project.
    #[serde(default)]
    pub branch_id: Option<Uuid>,
    #[validate(custom(function = "validation::commit_hash"))]
    #[serde(default, deserialize_with = "serde_helpers::lowercase_opt")]
    pub commit_hash: Option<String>,
    #[validate(custom(function = "validation::short_hash"))]
    #[serde(default, deserialize_with = "serde_helpers::lowercase_opt")]
    pub short_hash: Option<String>,
    #[validate(length(min = 1, max = 2000, message = "Must be 1–2000 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub commit_message: Option<String>,
    #[validate(length(max = 20000, message = "Must be at most 20000 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::patch_text")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    #[serde(default)]
    pub commit_date: Option<DateTime<Utc>>,
    #[validate(length(max = 200, message = "Must be at most 200 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::patch_text")]
    #[schema(value_type = Option<String>)]
    pub author_name: Option<Option<String>>,
    #[validate(custom(function = "validation::http_url"))]
    #[serde(default, deserialize_with = "serde_helpers::patch_text")]
    #[schema(value_type = Option<String>)]
    pub external_url: Option<Option<String>>,
    /// Replaces the commit's tags.
    #[validate(custom(function = "validation::id_list"))]
    #[serde(default)]
    pub tag_ids: Option<Vec<Uuid>>,
}

/// Filter on *recorded* deployments. `none` means "no deployment record
/// exists", which is not proof that the commit was never deployed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CommitDeploymentFilter {
    /// At least one deployment record.
    Any,
    /// No deployment record.
    None,
    Successful,
    Failed,
    RolledBack,
    Pending,
}

impl CommitDeploymentFilter {
    pub fn status(self) -> Option<DeploymentStatus> {
        match self {
            Self::Any | Self::None => None,
            Self::Successful => Some(DeploymentStatus::Successful),
            Self::Failed => Some(DeploymentStatus::Failed),
            Self::RolledBack => Some(DeploymentStatus::RolledBack),
            Self::Pending => Some(DeploymentStatus::Pending),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CommitSort {
    #[default]
    CommitDate,
    CreatedAt,
    /// Project, then branch name; newest commit first within a branch.
    BranchName,
}

#[derive(Debug, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListCommitsQuery {
    pub project_id: Option<Uuid>,
    /// Matches message, description, branch name, author and hash prefix.
    #[validate(length(max = 200))]
    pub search: Option<String>,
    /// Commits of this branch.
    pub branch_id: Option<Uuid>,
    /// Hash prefix.
    #[validate(length(max = 64))]
    pub hash: Option<String>,
    /// Commits dated at or after this instant.
    pub date_from: Option<DateTime<Utc>>,
    /// Commits dated at or before this instant.
    pub date_to: Option<DateTime<Utc>>,
    /// Filter by recorded deployments (optionally scoped by `environment_id`).
    pub deployment_status: Option<CommitDeploymentFilter>,
    /// Only consider deployment records of this environment. On its own it
    /// selects commits with at least one record there.
    pub environment_id: Option<Uuid>,
    pub tag_id: Option<Uuid>,
    /// Commits associated with this release.
    pub release_id: Option<Uuid>,
    pub sort: Option<CommitSort>,
    pub order: Option<SortOrder>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DeleteCommitQuery {
    /// Also delete the commit's deployment records. Without it, deleting a
    /// commit that has deployment history fails with 409.
    pub cascade: Option<bool>,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct CommitResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    #[sqlx(json)]
    pub project: ProjectRef,
    pub branch_id: Uuid,
    #[sqlx(json)]
    pub branch: BranchRef,
    pub commit_hash: String,
    pub short_hash: String,
    pub commit_message: String,
    pub description: Option<String>,
    pub commit_date: DateTime<Utc>,
    pub author_name: Option<String>,
    pub external_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[sqlx(json)]
    pub tags: Vec<TagRef>,
    /// Total number of deployment records for this commit.
    pub deployment_count: i64,
    /// Environments with at least one deployment record for this commit.
    #[sqlx(json)]
    pub environments: Vec<CommitEnvironment>,
    /// Releases this commit is associated with.
    #[sqlx(json)]
    pub releases: Vec<ReleaseRef>,
}

/// Summary of the deployment records of one commit in one environment.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CommitEnvironment {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub color: String,
    pub deployment_count: i64,
    pub latest_status: DeploymentStatus,
    pub latest_deployed_at: DateTime<Utc>,
}
