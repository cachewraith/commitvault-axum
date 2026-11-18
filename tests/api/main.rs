//! API integration tests. One test binary (faster to build and link than one
//! per file); each test runs against its own freshly migrated database.

mod common;

mod auth;
mod auth_codes;
mod branches;
mod commits;
mod constraints;
mod dashboard_search;
mod deployments;
mod notes_tags;
mod openapi;
mod projects_environments;
mod releases;
mod sorting;
