//! Single-user authentication: Argon2id password hashing, server-side
//! sessions (tower-sessions backed by PostgreSQL), CSRF tokens and login rate
//! limiting. There is no registration endpoint; the only account is created
//! from the bootstrap environment variables on first start.

pub mod bootstrap;
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

/// Request header that must carry the session's CSRF token on unsafe methods.
pub const CSRF_HEADER: &str = "x-csrf-token";
