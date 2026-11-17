use axum::{Json, extract::State, http::StatusCode};
use uuid::Uuid;

use super::{
    dto::{CreateReleaseRequest, ListReleasesQuery, ReleaseDetailResponse, ReleaseResponse, UpdateReleaseRequest},
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

/// List releases across projects
#[utoipa::path(get, path = "/releases", tag = "releases", params(ListReleasesQuery),
    responses((status = 200, body = Paginated<ReleaseResponse>)))]
pub async fn list(
    State(state): State<AppState>,
    ValidQuery(query): ValidQuery<ListReleasesQuery>,
) -> AppResult<Json<Paginated<ReleaseResponse>>> {
    Ok(Json(service::list(&state.db, query).await?))
}

/// List a project's releases
#[utoipa::path(get, path = "/projects/{project_id}/releases", tag = "releases",
    params(("project_id" = Uuid, Path, description = "Project id"), ListReleasesQuery),
    responses((status = 200, body = Paginated<ReleaseResponse>)))]
pub async fn list_for_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    ValidQuery(mut query): ValidQuery<ListReleasesQuery>,
) -> AppResult<Json<Paginated<ReleaseResponse>>> {
    query.project_id = Some(project_id);
    Ok(Json(service::list(&state.db, query).await?))
}

/// Create a release
#[utoipa::path(post, path = "/projects/{project_id}/releases", tag = "releases",
    params(("project_id" = Uuid, Path, description = "Project id")), request_body = CreateReleaseRequest,
    responses(
        (status = 201, body = Data<ReleaseDetailResponse>),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse, description = "Version already used in this project"),
        (status = 422, body = ErrorResponse),
    ))]
pub async fn create(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    ValidJson(request): ValidJson<CreateReleaseRequest>,
) -> AppResult<(StatusCode, Json<Data<ReleaseDetailResponse>>)> {
    Ok((StatusCode::CREATED, Data::json(service::create(&state.db, project_id, request).await?)))
}

/// Get a release with its notes and commits
#[utoipa::path(get, path = "/releases/{id}", tag = "releases", params(("id" = Uuid, Path, description = "Release id")),
    responses((status = 200, body = Data<ReleaseDetailResponse>), (status = 404, body = ErrorResponse)))]
pub async fn get(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<Data<ReleaseDetailResponse>>> {
    Ok(Data::json(service::get(&state.db, id).await?))
}

/// Update a release
///
/// `commit_ids` and `tag_ids`, when present, replace the current sets.
#[utoipa::path(patch, path = "/releases/{id}", tag = "releases", params(("id" = Uuid, Path, description = "Release id")),
    request_body = UpdateReleaseRequest,
    responses(
        (status = 200, body = Data<ReleaseDetailResponse>),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
    ))]
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<UpdateReleaseRequest>,
) -> AppResult<Json<Data<ReleaseDetailResponse>>> {
    Ok(Data::json(service::update(&state.db, id, request).await?))
}

/// Delete a release
///
/// The associated commits are kept; only the association is removed.
#[utoipa::path(delete, path = "/releases/{id}", tag = "releases", params(("id" = Uuid, Path, description = "Release id")),
    responses((status = 204), (status = 404, body = ErrorResponse)))]
pub async fn delete(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    service::delete(&state.db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
