use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum DeploymentStatus {
    Successful,
    Failed,
    RolledBack,
    Pending,
}

impl DeploymentStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Successful => "successful",
            Self::Failed => "failed",
            Self::RolledBack => "rolled_back",
            Self::Pending => "pending",
        }
    }
}

/// Row of the `deployments` table (write side).
#[derive(Debug, Clone)]
pub struct Deployment {
    pub id: uuid::Uuid,
    pub project_id: uuid::Uuid,
    pub environment_id: uuid::Uuid,
    pub commit_id: uuid::Uuid,
    pub deployment_title: String,
    pub deployment_notes: Option<String>,
    pub deployed_at: chrono::DateTime<chrono::Utc>,
    pub status: DeploymentStatus,
}
