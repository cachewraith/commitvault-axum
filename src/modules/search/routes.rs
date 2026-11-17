use utoipa_axum::{router::OpenApiRouter, routes};

use super::handlers;
use crate::AppState;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(handlers::search))
}
