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

/// Result of `POST /auth/login`.
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LoginResponse {
    /// Signed in.
    Authenticated(SessionResponse),
    /// Password accepted. A code was emailed; send it to `POST /auth/login/verify`.
    CodeRequired(CodeChallenge),
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CodeChallenge {
    /// CSRF token of the pending session (use it for verify / resend).
    pub csrf_token: String,
    /// Masked address the code was sent to, e.g. `a•••@example.com`.
    pub email_hint: String,
    pub expires_at: DateTime<Utc>,
    /// Earliest time `POST /auth/login/resend` will send another code.
    pub resend_available_at: DateTime<Utc>,
}

#[derive(Deserialize, Validate, ToSchema)]
pub struct VerifyLoginCodeRequest {
    /// The six-digit code from the email.
    #[validate(custom(function = "crate::shared::validation::one_time_code"))]
    #[serde(deserialize_with = "crate::shared::serde_helpers::trimmed")]
    pub code: String,
}

#[derive(Deserialize, Validate, ToSchema)]
pub struct ForgotPasswordRequest {
    #[validate(length(min = 3, max = 254, message = "Enter your email address"))]
    #[serde(deserialize_with = "crate::shared::serde_helpers::trimmed")]
    pub email: String,
}

#[derive(Deserialize, Validate, ToSchema)]
pub struct ResetPasswordRequest {
    #[validate(length(min = 3, max = 254, message = "Enter your email address"))]
    #[serde(deserialize_with = "crate::shared::serde_helpers::trimmed")]
    pub email: String,
    /// The six-digit code from the password reset email.
    #[validate(custom(function = "crate::shared::validation::one_time_code"))]
    #[serde(deserialize_with = "crate::shared::serde_helpers::trimmed")]
    pub code: String,
    #[validate(length(min = 12, max = 256, message = "Use between 12 and 256 characters"))]
    pub new_password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthOptionsResponse {
    /// Outgoing email is configured: "Forgot password?" works and new
    /// browsers are asked for an emailed code after the password.
    pub email_enabled: bool,
}
