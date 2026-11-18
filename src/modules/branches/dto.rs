use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::shared::{
    pagination::SortOrder,
    refs::{DeploymentBrief, ProjectRef},
    serde_helpers, validation,
};

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateBranchRequest {
    /// Git-style branch name, unique inside the project.
    #[validate(custom(function = "validation::branch_name"))]
    #[serde(deserialize_with = "serde_helpers::trimmed")]
    pub name: String,
    #[validate(length(max = 2000, message = "Must be at most 2000 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateBranchRequest {
    /// Renaming moves every commit of the branch along with it.
    #[validate(custom(function = "validation::branch_name"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub name: Option<String>,
    #[validate(length(max = 2000, message = "Must be at most 2000 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::patch_text")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BranchSort {
    /// Most recent commit date (branches without commits last).
    LastCommit,
    Name,
    /// Default: newest first.
    #[default]
    CreatedAt,
    /// Number of commits on the branch.
    Commits,
}

#[derive(Debug, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListBranchesQuery {
    /// Matches the branch name.
    #[validate(length(max = 200))]
    pub search: Option<String>,
    pub sort: Option<BranchSort>,
    /// Defaults to `asc` for `name`, `desc` otherwise.
    pub order: Option<SortOrder>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DeleteBranchQuery {
    /// Also delete the branch's commits and their deployment records. Without
    /// it, deleting a branch that still has commits fails with 409.
    pub cascade: Option<bool>,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct BranchResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    #[sqlx(json)]
    pub project: ProjectRef,
    pub name: String,
    pub description: Option<String>,
    /// The branch is the project's `default_branch`.
    pub is_default: bool,
    pub commit_count: i64,
    /// Date of the most recent commit recorded on this branch.
    pub last_commit_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A branch together with where its commits have recorded deployments.
#[derive(Debug, Serialize, ToSchema)]
pub struct BranchDetailResponse {
    #[serde(flatten)]
    pub branch: BranchResponse,
    /// Environments with at least one deployment record of a commit on this
    /// branch, in display order. An environment that is missing here has no
    /// record — which does not mean the branch was never deployed there.
    pub environments: Vec<BranchEnvironment>,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct BranchEnvironment {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub color: String,
    /// Deployment records of this branch's commits in the environment.
    pub deployment_count: i64,
    /// The most recent of those records.
    #[sqlx(json)]
    pub latest_deployment: DeploymentBrief,
}
