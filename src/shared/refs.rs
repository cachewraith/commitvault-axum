//! Compact references to related records, embedded in other resources so the
//! client never needs a follow-up request per row (no N+1 on either side).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProjectRef {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentRef {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CommitBrief {
    pub id: Uuid,
    pub short_hash: String,
    pub commit_hash: String,
    pub commit_message: String,
    pub branch_name: String,
    pub commit_date: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TagRef {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub color: String,
}

/// The essentials of one recorded deployment.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeploymentBrief {
    pub id: Uuid,
    pub deployment_title: String,
    pub status: crate::modules::deployments::models::DeploymentStatus,
    pub deployed_at: DateTime<Utc>,
    pub commit: CommitBrief,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReleaseRef {
    pub id: Uuid,
    pub version: String,
    pub title: String,
    pub status: crate::modules::releases::models::ReleaseStatus,
}
