use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::models::ProjectStatus;
use crate::shared::{pagination::SortOrder, refs::TagRef, serde_helpers, validation};

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateProjectRequest {
    #[validate(length(min = 1, max = 120, message = "Must be 1–120 characters"))]
    #[serde(deserialize_with = "serde_helpers::trimmed")]
    pub name: String,
    /// URL-friendly identifier. Generated from the name when omitted.
    #[validate(custom(function = "validation::slug"))]
    #[serde(default, deserialize_with = "serde_helpers::lowercase_opt")]
    pub slug: Option<String>,
    #[validate(length(max = 5000, message = "Must be at most 5000 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub description: Option<String>,
    #[validate(custom(function = "validation::http_url"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub repository_url: Option<String>,
    /// Defaults to `main`.
    #[validate(custom(function = "validation::branch_name"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub default_branch: Option<String>,
    #[validate(custom(function = "validation::id_list"))]
    #[serde(default)]
    pub tag_ids: Option<Vec<Uuid>>,
}

/// Partial update. Omitted fields are left unchanged; nullable fields are
/// cleared by sending `null`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateProjectRequest {
    #[validate(length(min = 1, max = 120, message = "Must be 1–120 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub name: Option<String>,
    #[validate(custom(function = "validation::slug"))]
    #[serde(default, deserialize_with = "serde_helpers::lowercase_opt")]
    pub slug: Option<String>,
    #[validate(length(max = 5000, message = "Must be at most 5000 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::patch_text")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    #[validate(custom(function = "validation::http_url"))]
    #[serde(default, deserialize_with = "serde_helpers::patch_text")]
    #[schema(value_type = Option<String>)]
    pub repository_url: Option<Option<String>>,
    #[validate(custom(function = "validation::branch_name"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub default_branch: Option<String>,
    /// `archived` archives the project, `active` restores it.
    #[serde(default)]
    pub status: Option<ProjectStatus>,
    /// Replaces the project's tags.
    #[validate(custom(function = "validation::id_list"))]
    #[serde(default)]
    pub tag_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProjectSort {
    Name,
    /// Default: newest first.
    #[default]
    CreatedAt,
    UpdatedAt,
    /// Latest recorded deployment (projects without one last).
    LastDeployedAt,
    /// Number of recorded commits.
    Commits,
}

#[derive(Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListProjectsQuery {
    /// Matches name, slug and description.
    #[validate(length(max = 100))]
    pub search: Option<String>,
    pub status: Option<ProjectStatus>,
    pub tag_id: Option<Uuid>,
    pub sort: Option<ProjectSort>,
    pub order: Option<SortOrder>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct ProjectResponse {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub repository_url: Option<String>,
    pub default_branch: String,
    pub status: ProjectStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    #[sqlx(json)]
    pub tags: Vec<TagRef>,
    #[sqlx(flatten)]
    pub stats: ProjectStats,
}

/// Record counts of a project.
#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct ProjectStats {
    pub commit_count: i64,
    pub environment_count: i64,
    pub deployment_count: i64,
    pub release_count: i64,
    pub note_count: i64,
    /// Time of the most recent recorded deployment, if any.
    pub last_deployed_at: Option<DateTime<Utc>>,
}
