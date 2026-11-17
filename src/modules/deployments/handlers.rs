use axum::{Json, extract::State, http::StatusCode};
use uuid::Uuid;

use super::{
    dto::{
        CompareEnvironmentsQuery, CreateDeploymentRequest, DeploymentResponse, EnvironmentComparison,
        ListDeploymentsQuery, UpdateDeploymentRequest,
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

/// List deployment records across projects
#[utoipa::path(get, path = "/deployments", tag = "deployments", params(ListDeploymentsQuery),
    responses((status = 200, body = Paginated<DeploymentResponse>)))]
pub async fn list(
    State(state): State<AppState>,
    ValidQuery(query): ValidQuery<ListDeploymentsQuery>,
) -> AppResult<Json<Paginated<DeploymentResponse>>> {
    Ok(Json(service::list(&state.db, query).await?))
}

/// List a project's deployment history
///
/// Same filters as `GET /deployments`; `project_id` is taken from the path.
#[utoipa::path(get, path = "/projects/{project_id}/deployments", tag = "deployments",
    params(("project_id" = Uuid, Path, description = "Project id"), ListDeploymentsQuery),
    responses((status = 200, body = Paginated<DeploymentResponse>)))]
pub async fn list_for_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    ValidQuery(mut query): ValidQuery<ListDeploymentsQuery>,
) -> AppResult<Json<Paginated<DeploymentResponse>>> {
    query.project_id = Some(project_id);
    Ok(Json(service::list(&state.db, query).await?))
}

/// Record a deployment
///
/// Stores the fact that a commit was deployed to an environment. Nothing is
/// executed or verified against any server.
#[utoipa::path(post, path = "/projects/{project_id}/deployments", tag = "deployments",
    params(("project_id" = Uuid, Path, description = "Project id")), request_body = CreateDeploymentRequest,
    responses(
        (status = 201, body = Data<DeploymentResponse>),
        (status = 404, body = ErrorResponse),
        (status = 422, body = ErrorResponse, description = "Invalid input, or environment/commit of another project"),
    ))]
pub async fn create(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    ValidJson(request): ValidJson<CreateDeploymentRequest>,
) -> AppResult<(StatusCode, Json<Data<DeploymentResponse>>)> {
    Ok((StatusCode::CREATED, Data::json(service::create(&state.db, project_id, request).await?)))
}

/// Compare the recorded deployments of two environments
///
/// Lists commits recorded in one environment, the other, or both. This is a
/// comparison of records only: it does not establish that a commit is absent
/// from an environment, nor that one environment is ahead of or behind another.
#[utoipa::path(get, path = "/projects/{project_id}/deployments/compare", tag = "deployments",
    params(("project_id" = Uuid, Path, description = "Project id"), CompareEnvironmentsQuery),
    responses((status = 200, body = Data<EnvironmentComparison>), (status = 422, body = ErrorResponse)))]
pub async fn compare(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    ValidQuery(query): ValidQuery<CompareEnvironmentsQuery>,
) -> AppResult<Json<Data<EnvironmentComparison>>> {
    Ok(Data::json(service::compare(&state.db, project_id, query).await?))
}

/// Get a deployment record
#[utoipa::path(get, path = "/deployments/{id}", tag = "deployments",
    params(("id" = Uuid, Path, description = "Deployment id")),
    responses((status = 200, body = Data<DeploymentResponse>), (status = 404, body = ErrorResponse)))]
pub async fn get(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<Data<DeploymentResponse>>> {
    Ok(Data::json(service::get(&state.db, id).await?))
}

/// Update a deployment record
#[utoipa::path(patch, path = "/deployments/{id}", tag = "deployments",
    params(("id" = Uuid, Path, description = "Deployment id")), request_body = UpdateDeploymentRequest,
    responses(
        (status = 200, body = Data<DeploymentResponse>),
        (status = 404, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
    ))]
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<UpdateDeploymentRequest>,
) -> AppResult<Json<Data<DeploymentResponse>>> {
    Ok(Data::json(service::update(&state.db, id, request).await?))
}

/// Delete a deployment record
#[utoipa::path(delete, path = "/deployments/{id}", tag = "deployments",
    params(("id" = Uuid, Path, description = "Deployment id")),
    responses((status = 204), (status = 404, body = ErrorResponse)))]
pub async fn delete(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    service::delete(&state.db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
