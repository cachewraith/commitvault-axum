use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Row of the `users` table. `password_hash` is deliberately not `Serialize`:
/// this type never leaves the backend (see `dto::UserResponse`).
#[derive(Debug, Clone)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub created_at: DateTime<Utc>,
}

/// The authenticated user, inserted into request extensions by the
/// `require_auth` middleware.
#[derive(Debug, Clone, Copy)]
pub struct CurrentUser {
    pub id: Uuid,
}
