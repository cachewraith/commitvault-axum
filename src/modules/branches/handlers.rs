use axum::{Json, extract::State, http::StatusCode};
use uuid::Uuid;

use super::{
    dto::{
        BranchDetailResponse, BranchResponse, CreateBranchRequest, DeleteBranchQuery, ListBranchesQuery,
        UpdateBranchRequest,
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

/// List a project's branches
///
/// Each branch carries its commit count and latest commit date. Default
/// order: most recent commit first.
#[utoipa::path(get, path = "/projects/{project_id}/branches", tag = "branches",
    params(("project_id" = Uuid, Path, description = "Project id"), ListBranchesQuery),
    responses((status = 200, body = Paginated<BranchResponse>), (status = 404, body = ErrorResponse)))]
pub async fn list_for_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    ValidQuery(query): ValidQuery<ListBranchesQuery>,
) -> AppResult<Json<Paginated<BranchResponse>>> {
    Ok(Json(service::list(&state.db, project_id, query).await?))
}

/// Create a branch
#[utoipa::path(post, path = "/projects/{project_id}/branches", tag = "branches",
    params(("project_id" = Uuid, Path, description = "Project id")), request_body = CreateBranchRequest,
    responses(
        (status = 201, body = Data<BranchResponse>),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse, description = "Name already used in this project"),
        (status = 422, body = ErrorResponse),
    ))]
pub async fn create(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    ValidJson(request): ValidJson<CreateBranchRequest>,
) -> AppResult<(StatusCode, Json<Data<BranchResponse>>)> {
    Ok((StatusCode::CREATED, Data::json(service::create(&state.db, project_id, request).await?)))
}

/// Get a branch
///
/// Includes, per environment, the latest deployment record of a commit on
/// this branch.
#[utoipa::path(get, path = "/branches/{id}", tag = "branches", params(("id" = Uuid, Path, description = "Branch id")),
    responses((status = 200, body = Data<BranchDetailResponse>), (status = 404, body = ErrorResponse)))]
pub async fn get(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<Data<BranchDetailResponse>>> {
    Ok(Data::json(service::get_detail(&state.db, id).await?))
}

/// Update a branch
///
/// Renaming the project's default branch also updates the project's
/// `default_branch`.
#[utoipa::path(patch, path = "/branches/{id}", tag = "branches", params(("id" = Uuid, Path, description = "Branch id")),
    request_body = UpdateBranchRequest,
    responses(
        (status = 200, body = Data<BranchResponse>),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse, description = "Name already used in this project"),
        (status = 422, body = ErrorResponse),
    ))]
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<UpdateBranchRequest>,
) -> AppResult<Json<Data<BranchResponse>>> {
    Ok(Data::json(service::update(&state.db, id, request).await?))
}

/// Delete a branch
#[utoipa::path(delete, path = "/branches/{id}", tag = "branches",
    params(("id" = Uuid, Path, description = "Branch id"), DeleteBranchQuery),
    responses(
        (status = 204),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse, description = "Has commits and `cascade` was not set"),
    ))]
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    ValidQuery(query): ValidQuery<DeleteBranchQuery>,
) -> AppResult<StatusCode> {
    service::delete(&state.db, id, query.cascade.unwrap_or(false)).await?;
    Ok(StatusCode::NO_CONTENT)
}
