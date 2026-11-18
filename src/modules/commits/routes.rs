use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::AppState;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(handlers::list))
        .routes(routes!(handlers::list_for_project, handlers::create))
        .routes(routes!(handlers::get, handlers::update, handlers::delete))
}
