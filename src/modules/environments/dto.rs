use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::shared::{
    refs::{DeploymentBrief, ProjectRef},
    serde_helpers, validation,
};

pub const DEFAULT_ENVIRONMENT_COLOR: &str = "#71717a";
/// Upper bound per project; keeps the per-project list endpoint bounded.
pub const MAX_ENVIRONMENTS_PER_PROJECT: i64 = 50;

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateEnvironmentRequest {
    #[validate(length(min = 1, max = 80, message = "Must be 1–80 characters"))]
    #[serde(deserialize_with = "serde_helpers::trimmed")]
    pub name: String,
    #[validate(length(max = 2000, message = "Must be at most 2000 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub description: Option<String>,
    /// `#rrggbb` used for badges. Defaults to a neutral grey.
    #[validate(custom(function = "validation::hex_color"))]
    #[serde(default, deserialize_with = "serde_helpers::lowercase_opt")]
    pub color: Option<String>,
    /// Free-form label of the server or cluster, e.g. `vps-eu-1`.
    #[validate(length(max = 120, message = "Must be at most 120 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub server_label: Option<String>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateEnvironmentRequest {
    #[validate(length(min = 1, max = 80, message = "Must be 1–80 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::trimmed_opt")]
    pub name: Option<String>,
    #[validate(length(max = 2000, message = "Must be at most 2000 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::patch_text")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    #[validate(custom(function = "validation::hex_color"))]
    #[serde(default, deserialize_with = "serde_helpers::lowercase_opt")]
    pub color: Option<String>,
    #[validate(length(max = 120, message = "Must be at most 120 characters"))]
    #[serde(default, deserialize_with = "serde_helpers::patch_text")]
    #[schema(value_type = Option<String>)]
    pub server_label: Option<Option<String>>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct ReorderEnvironmentsRequest {
    /// Every environment id of the project, in the desired display order.
    #[validate(length(max = 50, message = "Too many ids"))]
    pub environment_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListEnvironmentsQuery {
    pub project_id: Option<Uuid>,
    /// Matches environment name and server label.
    #[validate(length(max = 100))]
    pub search: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

#[derive(Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DeleteEnvironmentQuery {
    /// Also delete the deployment records of this environment. Without it,
    /// deleting an environment that has deployment history fails with 409.
    pub cascade: Option<bool>,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct EnvironmentResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    #[sqlx(json)]
    pub project: ProjectRef,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub color: String,
    pub server_label: Option<String>,
    pub position: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Number of deployment records for this environment.
    pub deployment_count: i64,
    /// Most recent recorded deployment of any status.
    #[sqlx(json(nullable))]
    pub latest_deployment: Option<DeploymentBrief>,
    /// Most recent recorded deployment with status `successful`.
    #[sqlx(json(nullable))]
    pub latest_successful_deployment: Option<DeploymentBrief>,
}
