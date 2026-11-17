use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::models::ReleaseStatus;
use crate::shared::{
    pagination::SortOrder,
    refs::{CommitBrief, ProjectRef, TagRef},
    serde_helpers, validation,
};

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateReleaseRequest {
    /// Free-form version label. Semantic versions (`1.4.0`) are supported but
    /// not required; it only has to be unique inside the project.
    #[validate(length(min = 1, max = 64, message = "Must be 1–64 characters"))]
    #[serde(deserialize_with = "serde_helpers::trimmed")]
    pub version: String,
    #[validate(length(min = 1, max = 200, message = "Must be 1–200 characters"))]
    #[serde(deserialize_with = "serde_helpers::trimmed")]
    pub title: String,
    #[validate(length(max = 5000, message = "Must be at most 5000 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub description: Option<String>,
    /// Tiptap JSON document.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub release_notes: Option<serde_json::Value>,
    #[serde(default)]
    pub released_at: Option<DateTime<Utc>>,
    /// Defaults to `draft`.
    #[serde(default)]
    pub status: Option<ReleaseStatus>,
    /// Commits of the same project to associate with the release.
    #[validate(custom(function = "validation::id_list"))]
    #[serde(default)]
    pub commit_ids: Option<Vec<Uuid>>,
    #[validate(custom(function = "validation::id_list"))]
    #[serde(default)]
    pub tag_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateReleaseRequest {
    #[validate(length(min = 1, max = 64, message = "Must be 1–64 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub version: Option<String>,
    #[validate(length(min = 1, max = 200, message = "Must be 1–200 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub title: Option<String>,
    #[validate(length(max = 5000, message = "Must be at most 5000 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::patch_text")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    /// Tiptap JSON document; `null` clears the notes.
    #[serde(default, deserialize_with = "serde_helpers::patch")]
    #[schema(value_type = Option<Object>)]
    pub release_notes: Option<Option<serde_json::Value>>,
    #[serde(default, deserialize_with = "serde_helpers::patch")]
    #[schema(value_type = Option<DateTime<Utc>>)]
    pub released_at: Option<Option<DateTime<Utc>>>,
    #[serde(default)]
    pub status: Option<ReleaseStatus>,
    /// Replaces the associated commits.
    #[validate(custom(function = "validation::id_list"))]
    #[serde(default)]
    pub commit_ids: Option<Vec<Uuid>>,
    /// Replaces the release's tags.
    #[validate(custom(function = "validation::id_list"))]
    #[serde(default)]
    pub tag_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseSort {
    #[default]
    ReleasedAt,
    CreatedAt,
    Version,
}

#[derive(Debug, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListReleasesQuery {
    pub project_id: Option<Uuid>,
    pub status: Option<ReleaseStatus>,
    /// Matches version, title, description and release-note text.
    #[validate(length(max = 200))]
    pub search: Option<String>,
    pub tag_id: Option<Uuid>,
    /// Releases that include this commit.
    pub commit_id: Option<Uuid>,
    pub sort: Option<ReleaseSort>,
    pub order: Option<SortOrder>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct ReleaseResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    #[sqlx(json)]
    pub project: ProjectRef,
    pub version: String,
    pub title: String,
    pub description: Option<String>,
    pub released_at: Option<DateTime<Utc>>,
    pub status: ReleaseStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[sqlx(json)]
    pub tags: Vec<TagRef>,
    pub commit_count: i64,
}

/// A release with its notes and associated commits.
#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct ReleaseDetailResponse {
    #[serde(flatten)]
    #[sqlx(flatten)]
    pub release: ReleaseResponse,
    /// Tiptap JSON document.
    #[schema(value_type = Option<Object>)]
    pub release_notes: Option<serde_json::Value>,
    /// Associated commits, newest first.
    #[sqlx(json)]
    pub commits: Vec<CommitBrief>,
}
