use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::shared::{pagination::SortOrder, serde_helpers, validation};

pub const DEFAULT_TAG_COLOR: &str = "#71717a";

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateTagRequest {
    #[validate(length(min = 1, max = 50, message = "Must be 1–50 characters"))]
    #[serde(deserialize_with = "serde_helpers::trimmed")]
    pub name: String,
    /// `#rrggbb`. Defaults to a neutral grey.
    #[validate(custom(function = "validation::hex_color"))]
    #[serde(default, deserialize_with = "serde_helpers::lowercase_opt")]
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateTagRequest {
    #[validate(length(min = 1, max = 50, message = "Must be 1–50 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub name: Option<String>,
    #[validate(custom(function = "validation::hex_color"))]
    #[serde(default, deserialize_with = "serde_helpers::lowercase_opt")]
    pub color: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TagSort {
    Name,
    /// Default: newest first.
    #[default]
    CreatedAt,
    /// Number of records using the tag.
    Usage,
}

#[derive(Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListTagsQuery {
    /// Case-insensitive match on the tag name.
    #[validate(length(max = 100))]
    pub search: Option<String>,
    pub sort: Option<TagSort>,
    pub order: Option<SortOrder>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct TagResponse {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub color: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[sqlx(flatten)]
    pub usage: TagUsage,
}

/// How many records of each kind carry the tag.
#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct TagUsage {
    pub project_count: i64,
    pub commit_count: i64,
    pub deployment_count: i64,
    pub release_count: i64,
    pub note_count: i64,
    pub total: i64,
}
