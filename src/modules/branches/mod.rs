//! Branches: named lines of work inside a project. Every commit belongs to
//! exactly one branch of its own project. CommitVault never reads Git; a
//! branch is a record the user created.

pub mod dto;
pub mod handlers;
pub mod models;
pub mod repository;
pub mod routes;
pub mod service;
