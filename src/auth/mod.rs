//! Single-user authentication: Argon2id password hashing, server-side
//! sessions (tower-sessions backed by PostgreSQL), CSRF tokens, login rate
//! limiting, an emailed one-time code for browsers that are not trusted yet,
//! and password reset by emailed code. There is no registration endpoint; the only account is created
//! from the bootstrap environment variables on first start.

pub mod bootstrap;
pub mod codes;
pub mod device;
pub mod dto;
pub mod handlers;
pub mod models;
pub mod password;
pub mod rate_limit;
pub mod repository;
pub mod routes;
pub mod service;
pub mod session_store;

/// Session keys.
pub const SESSION_USER_ID: &str = "user_id";
pub const SESSION_CSRF_TOKEN: &str = "csrf_token";
/// Set after a correct password while the emailed code is still outstanding.
pub const SESSION_PENDING_USER_ID: &str = "pending_user_id";

/// Request header that must carry the session's CSRF token on unsafe methods.
pub const CSRF_HEADER: &str = "x-csrf-token";
