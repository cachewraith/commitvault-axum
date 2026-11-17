use axum::{Json, extract::State, http::StatusCode};
use uuid::Uuid;

use super::{
    dto::{CreateProjectRequest, ListProjectsQuery, ProjectResponse, UpdateProjectRequest},
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

/// List projects
#[utoipa::path(get, path = "/projects", tag = "projects", params(ListProjectsQuery),
    responses((status = 200, body = Paginated<ProjectResponse>), (status = 401, body = ErrorResponse)))]
pub async fn list(
    State(state): State<AppState>,
    ValidQuery(query): ValidQuery<ListProjectsQuery>,
) -> AppResult<Json<Paginated<ProjectResponse>>> {
    Ok(Json(service::list(&state.db, query).await?))
}

/// Create a project
#[utoipa::path(post, path = "/projects", tag = "projects", request_body = CreateProjectRequest,
    responses(
        (status = 201, body = Data<ProjectResponse>),
        (status = 409, body = ErrorResponse, description = "Slug already in use"),
        (status = 422, body = ErrorResponse),
    ))]
pub async fn create(
    State(state): State<AppState>,
    ValidJson(request): ValidJson<CreateProjectRequest>,
) -> AppResult<(StatusCode, Json<Data<ProjectResponse>>)> {
    Ok((StatusCode::CREATED, Data::json(service::create(&state.db, request).await?)))
}

/// Get a project
#[utoipa::path(get, path = "/projects/{id}", tag = "projects", params(("id" = Uuid, Path, description = "Project id")),
    responses((status = 200, body = Data<ProjectResponse>), (status = 404, body = ErrorResponse)))]
pub async fn get(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<Data<ProjectResponse>>> {
    Ok(Data::json(service::get(&state.db, id).await?))
}

/// Update or archive a project
#[utoipa::path(patch, path = "/projects/{id}", tag = "projects", params(("id" = Uuid, Path, description = "Project id")),
    request_body = UpdateProjectRequest,
    responses(
        (status = 200, body = Data<ProjectResponse>),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
    ))]
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<UpdateProjectRequest>,
) -> AppResult<Json<Data<ProjectResponse>>> {
    Ok(Data::json(service::update(&state.db, id, request).await?))
}

/// Delete a project
///
/// Permanently removes the project together with its environments, commits,
/// deployments, releases and project notes.
#[utoipa::path(delete, path = "/projects/{id}", tag = "projects", params(("id" = Uuid, Path, description = "Project id")),
    responses((status = 204), (status = 404, body = ErrorResponse)))]
pub async fn delete(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    service::delete(&state.db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
