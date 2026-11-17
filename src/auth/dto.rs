use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::models::User;

pub const MIN_PASSWORD_LEN: u64 = 12;
pub const MAX_PASSWORD_LEN: u64 = 256;

#[derive(Deserialize, Validate, ToSchema)]
pub struct LoginRequest {
    #[validate(length(min = 3, max = 254, message = "Enter your email address"))]
    #[serde(deserialize_with = "crate::shared::serde_helpers::trimmed")]
    pub email: String,
    /// Not trimmed: whitespace can be part of a passphrase.
    #[validate(length(min = 1, max = 256, message = "Enter your password"))]
    pub password: String,
}

#[derive(Deserialize, Validate, ToSchema)]
pub struct ChangePasswordRequest {
    #[validate(length(min = 1, max = 256, message = "Enter your current password"))]
    pub current_password: String,
    #[validate(length(min = 12, max = 256, message = "Use between 12 and 256 characters"))]
    pub new_password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserResponse {
    pub id: Uuid,
    pub email: String,
    pub created_at: DateTime<Utc>,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self { id: user.id, email: user.email, created_at: user.created_at }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionResponse {
    pub user: UserResponse,
    /// Send this value in the `X-CSRF-Token` header of every state-changing request.
    pub csrf_token: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CsrfTokenResponse {
    pub csrf_token: String,
}
