use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::AppState;

/// Endpoints reachable without an authenticated session.
pub fn public_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::csrf))
        .routes(routes!(handlers::login))
        .routes(routes!(handlers::logout))
}

/// Endpoints that require an authenticated session.
pub fn protected_router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(handlers::me)).routes(routes!(handlers::change_password))
}
