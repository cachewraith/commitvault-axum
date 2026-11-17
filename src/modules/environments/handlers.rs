use axum::{Json, extract::State, http::StatusCode};
use uuid::Uuid;

use super::{
    dto::{
        CreateEnvironmentRequest, DeleteEnvironmentQuery, EnvironmentResponse, ListEnvironmentsQuery,
        ReorderEnvironmentsRequest, UpdateEnvironmentRequest,
    },
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

/// List environments across projects
#[utoipa::path(get, path = "/environments", tag = "environments", params(ListEnvironmentsQuery),
    responses((status = 200, body = Paginated<EnvironmentResponse>)))]
pub async fn list(
    State(state): State<AppState>,
    ValidQuery(query): ValidQuery<ListEnvironmentsQuery>,
) -> AppResult<Json<Paginated<EnvironmentResponse>>> {
    Ok(Json(service::list(&state.db, query).await?))
}

/// List a project's environments
///
/// Returned in display order, each with its latest recorded deployment.
#[utoipa::path(get, path = "/projects/{project_id}/environments", tag = "environments",
    params(("project_id" = Uuid, Path, description = "Project id")),
    responses((status = 200, body = Data<Vec<EnvironmentResponse>>), (status = 404, body = ErrorResponse)))]
pub async fn list_for_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> AppResult<Json<Data<Vec<EnvironmentResponse>>>> {
    Ok(Data::json(service::list_for_project(&state.db, project_id).await?))
}

/// Create an environment
#[utoipa::path(post, path = "/projects/{project_id}/environments", tag = "environments",
    params(("project_id" = Uuid, Path, description = "Project id")), request_body = CreateEnvironmentRequest,
    responses(
        (status = 201, body = Data<EnvironmentResponse>),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse, description = "Name already used in this project"),
        (status = 422, body = ErrorResponse),
    ))]
pub async fn create(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    ValidJson(request): ValidJson<CreateEnvironmentRequest>,
) -> AppResult<(StatusCode, Json<Data<EnvironmentResponse>>)> {
    Ok((StatusCode::CREATED, Data::json(service::create(&state.db, project_id, request).await?)))
}

/// Reorder a project's environments
#[utoipa::path(put, path = "/projects/{project_id}/environments/order", tag = "environments",
    params(("project_id" = Uuid, Path, description = "Project id")), request_body = ReorderEnvironmentsRequest,
    responses(
        (status = 200, body = Data<Vec<EnvironmentResponse>>),
        (status = 404, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
    ))]
pub async fn reorder(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    ValidJson(request): ValidJson<ReorderEnvironmentsRequest>,
) -> AppResult<Json<Data<Vec<EnvironmentResponse>>>> {
    Ok(Data::json(service::reorder(&state.db, project_id, request).await?))
}

/// Get an environment
#[utoipa::path(get, path = "/environments/{id}", tag = "environments",
    params(("id" = Uuid, Path, description = "Environment id")),
    responses((status = 200, body = Data<EnvironmentResponse>), (status = 404, body = ErrorResponse)))]
pub async fn get(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<Data<EnvironmentResponse>>> {
    Ok(Data::json(service::get(&state.db, id).await?))
}

/// Update an environment
#[utoipa::path(patch, path = "/environments/{id}", tag = "environments",
    params(("id" = Uuid, Path, description = "Environment id")), request_body = UpdateEnvironmentRequest,
    responses(
        (status = 200, body = Data<EnvironmentResponse>),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
    ))]
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<UpdateEnvironmentRequest>,
) -> AppResult<Json<Data<EnvironmentResponse>>> {
    Ok(Data::json(service::update(&state.db, id, request).await?))
}

/// Delete an environment
#[utoipa::path(delete, path = "/environments/{id}", tag = "environments",
    params(("id" = Uuid, Path, description = "Environment id"), DeleteEnvironmentQuery),
    responses(
        (status = 204),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse, description = "Has deployment records and `cascade` was not set"),
    ))]
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    ValidQuery(query): ValidQuery<DeleteEnvironmentQuery>,
) -> AppResult<StatusCode> {
    service::delete(&state.db, id, query.cascade.unwrap_or(false)).await?;
    Ok(StatusCode::NO_CONTENT)
}
