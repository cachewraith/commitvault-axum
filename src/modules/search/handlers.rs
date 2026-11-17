use axum::{Json, extract::State};

use super::{
    dto::{SearchQuery, SearchResponse},
    service,
};
use crate::{
    AppState,
    errors::{AppResult, ErrorResponse},
    shared::{extractors::ValidQuery, response::Data},
};

/// Search everything
///
/// Searches projects, commits (message, branch, author, hash prefix),
/// deployments, releases, notes (including their text) and tags.
#[utoipa::path(get, path = "/search", tag = "search", params(SearchQuery),
    responses((status = 200, body = Data<SearchResponse>), (status = 422, body = ErrorResponse)))]
pub async fn search(
    State(state): State<AppState>,
    ValidQuery(query): ValidQuery<SearchQuery>,
) -> AppResult<Json<Data<SearchResponse>>> {
    Ok(Data::json(service::search(&state.db, query).await?))
}
