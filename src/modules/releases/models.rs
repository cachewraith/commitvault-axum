use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum ReleaseStatus {
    Draft,
    Released,
    Deprecated,
}

impl ReleaseStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Released => "released",
            Self::Deprecated => "deprecated",
        }
    }
}

/// Row of the `releases` table (write side).
#[derive(Debug, Clone)]
pub struct Release {
    pub id: Uuid,
    pub project_id: Uuid,
    pub version: String,
    pub title: String,
    pub description: Option<String>,
    pub release_notes: Option<serde_json::Value>,
    pub release_notes_text: String,
    pub released_at: Option<DateTime<Utc>>,
    pub status: ReleaseStatus,
}
