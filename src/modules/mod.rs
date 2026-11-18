//! Feature modules. Each owns its routes, handlers, service, repository,
//! models and DTOs.

pub mod branches;
pub mod commits;
pub mod dashboard;
pub mod deployments;
pub mod environments;
pub mod notes;
pub mod projects;
pub mod releases;
pub mod search;
pub mod tags;

use utoipa_axum::router::OpenApiRouter;

use crate::AppState;

/// All authenticated feature routes.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .merge(dashboard::routes::router())
        .merge(projects::routes::router())
        .merge(environments::routes::router())
        .merge(branches::routes::router())
        .merge(commits::routes::router())
        .merge(deployments::routes::router())
        .merge(releases::routes::router())
        .merge(notes::routes::router())
        .merge(search::routes::router())
        .merge(tags::routes::router())
}
