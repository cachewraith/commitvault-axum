//! Deployment history: user-recorded deployment events.
//!
//! A record states that the user noted a deployment of a commit to an
//! environment. The absence of a record proves nothing, and no ordering
//! between environments ("ahead"/"behind") is ever derived, because the
//! application has no Git ancestry information and never inspects servers.

pub mod dto;
pub mod handlers;
pub mod models;
pub mod repository;
pub mod routes;
pub mod service;
