use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::shared::refs::ProjectRef;

pub const DEFAULT_LIMIT: u32 = 5;
pub const MAX_LIMIT: u32 = 20;

#[derive(Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SearchQuery {
    /// Search term (2–100 characters).
    #[validate(length(max = 200, message = "Search term is too long"))]
    pub q: String,
    /// Restrict results to one project (tags are always global).
    pub project_id: Option<Uuid>,
    /// Maximum hits per record type (default 5, maximum 20).
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    Project,
    Commit,
    Deployment,
    Release,
    Note,
    Tag,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchHit {
    pub id: Uuid,
    pub kind: SearchKind,
    pub title: String,
    /// Secondary line: hash and branch, environment and status, excerpt…
    pub subtitle: Option<String>,
    /// Owning project (absent for projects, tags and global notes).
    pub project: Option<ProjectRef>,
    /// The record's most relevant date (commit date, deployment time, …).
    pub occurred_at: Option<DateTime<Utc>>,
}

/// Hits grouped by record type, each group capped at `limit`.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResponse {
    pub query: String,
    pub projects: Vec<SearchHit>,
    pub commits: Vec<SearchHit>,
    pub deployments: Vec<SearchHit>,
    pub releases: Vec<SearchHit>,
    pub notes: Vec<SearchHit>,
    pub tags: Vec<SearchHit>,
    /// Number of hits returned across all groups.
    pub total: usize,
}
