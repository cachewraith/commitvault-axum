use axum::{Json, extract::State, http::StatusCode};
use uuid::Uuid;

use super::{
    dto::{
        BranchSummary, CommitResponse, CreateCommitRequest, DeleteCommitQuery, ListCommitsQuery, UpdateCommitRequest,
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

/// List commits across projects
#[utoipa::path(get, path = "/commits", tag = "commits", params(ListCommitsQuery),
    responses((status = 200, body = Paginated<CommitResponse>)))]
pub async fn list(
    State(state): State<AppState>,
    ValidQuery(query): ValidQuery<ListCommitsQuery>,
) -> AppResult<Json<Paginated<CommitResponse>>> {
    Ok(Json(service::list(&state.db, query).await?))
}

/// List a project's commits
///
/// Same filters as `GET /commits`; `project_id` is taken from the path.
#[utoipa::path(get, path = "/projects/{project_id}/commits", tag = "commits",
    params(("project_id" = Uuid, Path, description = "Project id"), ListCommitsQuery),
    responses((status = 200, body = Paginated<CommitResponse>)))]
pub async fn list_for_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    ValidQuery(mut query): ValidQuery<ListCommitsQuery>,
) -> AppResult<Json<Paginated<CommitResponse>>> {
    query.project_id = Some(project_id);
    Ok(Json(service::list(&state.db, query).await?))
}

/// Record a commit
#[utoipa::path(post, path = "/projects/{project_id}/commits", tag = "commits",
    params(("project_id" = Uuid, Path, description = "Project id")), request_body = CreateCommitRequest,
    responses(
        (status = 201, body = Data<CommitResponse>),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse, description = "Hash already recorded for this project"),
        (status = 422, body = ErrorResponse),
    ))]
pub async fn create(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    ValidJson(request): ValidJson<CreateCommitRequest>,
) -> AppResult<(StatusCode, Json<Data<CommitResponse>>)> {
    Ok((StatusCode::CREATED, Data::json(service::create(&state.db, project_id, request).await?)))
}

/// List a project's branch names
///
/// Distinct branches seen on recorded commits, most recently used first
/// (for autocompletion and filters).
#[utoipa::path(get, path = "/projects/{project_id}/branches", tag = "commits",
    params(("project_id" = Uuid, Path, description = "Project id")),
    responses((status = 200, body = Data<Vec<BranchSummary>>), (status = 404, body = ErrorResponse)))]
pub async fn branches(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> AppResult<Json<Data<Vec<BranchSummary>>>> {
    Ok(Data::json(service::branches(&state.db, project_id).await?))
}

/// Get a commit
#[utoipa::path(get, path = "/commits/{id}", tag = "commits", params(("id" = Uuid, Path, description = "Commit id")),
    responses((status = 200, body = Data<CommitResponse>), (status = 404, body = ErrorResponse)))]
pub async fn get(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<Data<CommitResponse>>> {
    Ok(Data::json(service::get(&state.db, id).await?))
}

/// Update a commit
#[utoipa::path(patch, path = "/commits/{id}", tag = "commits", params(("id" = Uuid, Path, description = "Commit id")),
    request_body = UpdateCommitRequest,
    responses(
        (status = 200, body = Data<CommitResponse>),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
    ))]
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<UpdateCommitRequest>,
) -> AppResult<Json<Data<CommitResponse>>> {
    Ok(Data::json(service::update(&state.db, id, request).await?))
}

/// Delete a commit
#[utoipa::path(delete, path = "/commits/{id}", tag = "commits",
    params(("id" = Uuid, Path, description = "Commit id"), DeleteCommitQuery),
    responses(
        (status = 204),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse, description = "Has deployment records and `cascade` was not set"),
    ))]
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    ValidQuery(query): ValidQuery<DeleteCommitQuery>,
) -> AppResult<StatusCode> {
    service::delete(&state.db, id, query.cascade.unwrap_or(false)).await?;
    Ok(StatusCode::NO_CONTENT)
}
