use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::shared::{
    pagination::SortOrder,
    refs::{ProjectRef, TagRef},
    serde_helpers, validation,
};

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateNoteRequest {
    /// Omit (or `null`) for a global note.
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[validate(length(min = 1, max = 200, message = "Must be 1–200 characters"))]
    #[serde(deserialize_with = "serde_helpers::trimmed")]
    pub title: String,
    /// Tiptap JSON document. Defaults to an empty document.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub content_json: Option<serde_json::Value>,
    #[validate(length(max = 60, message = "Must be at most 60 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub category: Option<String>,
    #[serde(default)]
    pub is_pinned: Option<bool>,
    #[validate(custom(function = "validation::id_list"))]
    #[serde(default)]
    pub tag_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateNoteRequest {
    /// `null` turns the note into a global note.
    #[serde(default, deserialize_with = "serde_helpers::patch")]
    #[schema(value_type = Option<Uuid>)]
    pub project_id: Option<Option<Uuid>>,
    #[validate(length(min = 1, max = 200, message = "Must be 1–200 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub title: Option<String>,
    /// Tiptap JSON document.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub content_json: Option<serde_json::Value>,
    #[validate(length(max = 60, message = "Must be at most 60 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::patch_text")]
    #[schema(value_type = Option<String>)]
    pub category: Option<Option<String>>,
    #[serde(default)]
    pub is_pinned: Option<bool>,
    /// Replaces the note's tags.
    #[validate(custom(function = "validation::id_list"))]
    #[serde(default)]
    pub tag_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NoteSort {
    UpdatedAt,
    /// Default: newest first.
    #[default]
    CreatedAt,
    Title,
}

#[derive(Debug, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListNotesQuery {
    pub project_id: Option<Uuid>,
    /// `true` selects only global notes (those without a project).
    pub global: Option<bool>,
    /// Exact category.
    #[validate(length(max = 60))]
    pub category: Option<String>,
    pub pinned: Option<bool>,
    /// Matches title, category and note text.
    #[validate(length(max = 200))]
    pub search: Option<String>,
    pub tag_id: Option<Uuid>,
    pub sort: Option<NoteSort>,
    pub order: Option<SortOrder>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct NoteCategoriesQuery {
    pub project_id: Option<Uuid>,
}

/// A note without its document body (list views).
#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct NoteResponse {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    #[sqlx(json(nullable))]
    pub project: Option<ProjectRef>,
    pub title: String,
    /// Plain-text preview of the content.
    pub excerpt: String,
    pub category: Option<String>,
    pub is_pinned: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[sqlx(json)]
    pub tags: Vec<TagRef>,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct NoteDetailResponse {
    #[serde(flatten)]
    #[sqlx(flatten)]
    pub note: NoteResponse,
    /// Tiptap JSON document.
    #[schema(value_type = Object)]
    pub content_json: serde_json::Value,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct NoteCategory {
    pub category: String,
    pub note_count: i64,
}
