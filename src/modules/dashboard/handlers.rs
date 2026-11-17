use axum::{Json, extract::State};

use super::{dto::DashboardResponse, service};
use crate::{
    AppState,
    errors::{AppResult, ErrorResponse},
    shared::response::Data,
};

/// Workspace overview
///
/// Totals, recent activity and the latest recorded deployment per environment.
#[utoipa::path(get, path = "/dashboard", tag = "dashboard",
    responses((status = 200, body = Data<DashboardResponse>), (status = 401, body = ErrorResponse)))]
pub async fn overview(State(state): State<AppState>) -> AppResult<Json<Data<DashboardResponse>>> {
    Ok(Data::json(service::overview(&state.db).await?))
}
