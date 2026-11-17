use axum::{Json, extract::State, http::StatusCode};
use uuid::Uuid;

use super::{
    dto::{CreateTagRequest, ListTagsQuery, TagResponse, UpdateTagRequest},
    service,
};
use crate::{
    AppState,
    errors::{AppResult, ErrorResponse},
    shared::{
        extractors::{Path, ValidJson, ValidQuery},
        response::{Data, Paginated},
    },
};

/// List tags
#[utoipa::path(get, path = "/tags", tag = "tags", params(ListTagsQuery),
    responses((status = 200, body = Paginated<TagResponse>), (status = 401, body = ErrorResponse)))]
pub async fn list(
    State(state): State<AppState>,
    ValidQuery(query): ValidQuery<ListTagsQuery>,
) -> AppResult<Json<Paginated<TagResponse>>> {
    Ok(Json(service::list(&state.db, query).await?))
}

/// Create a tag
#[utoipa::path(post, path = "/tags", tag = "tags", request_body = CreateTagRequest,
    responses(
        (status = 201, body = Data<TagResponse>),
        (status = 409, body = ErrorResponse, description = "A tag with this name already exists"),
        (status = 422, body = ErrorResponse),
    ))]
pub async fn create(
    State(state): State<AppState>,
    ValidJson(request): ValidJson<CreateTagRequest>,
) -> AppResult<(StatusCode, Json<Data<TagResponse>>)> {
    Ok((StatusCode::CREATED, Data::json(service::create(&state.db, request).await?)))
}

/// Get a tag
#[utoipa::path(get, path = "/tags/{id}", tag = "tags", params(("id" = Uuid, Path, description = "Tag id")),
    responses((status = 200, body = Data<TagResponse>), (status = 404, body = ErrorResponse)))]
pub async fn get(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<Data<TagResponse>>> {
    Ok(Data::json(service::get(&state.db, id).await?))
}

/// Update a tag
#[utoipa::path(patch, path = "/tags/{id}", tag = "tags", params(("id" = Uuid, Path, description = "Tag id")),
    request_body = UpdateTagRequest,
    responses(
        (status = 200, body = Data<TagResponse>),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
    ))]
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<UpdateTagRequest>,
) -> AppResult<Json<Data<TagResponse>>> {
    Ok(Data::json(service::update(&state.db, id, request).await?))
}

/// Delete a tag
///
/// Detaches the tag from every record; the records themselves are untouched.
#[utoipa::path(delete, path = "/tags/{id}", tag = "tags", params(("id" = Uuid, Path, description = "Tag id")),
    responses((status = 204), (status = 404, body = ErrorResponse)))]
pub async fn delete(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    service::delete(&state.db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
