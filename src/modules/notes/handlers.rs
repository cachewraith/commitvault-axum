use axum::{Json, extract::State, http::StatusCode};
use uuid::Uuid;

use super::{
    dto::{
        CreateNoteRequest, ListNotesQuery, NoteCategoriesQuery, NoteCategory, NoteDetailResponse, NoteResponse,
        UpdateNoteRequest,
    },
    service,
};
use crate::{
    AppState,
    errors::{AppError, AppResult, ErrorResponse},
    modules::projects,
    shared::{
        extractors::{Path, ValidJson, ValidQuery},
        response::{Data, Paginated},
    },
};

/// List notes
///
/// Pinned notes come first. The document body is not included; fetch a
/// single note for that.
#[utoipa::path(get, path = "/notes", tag = "notes", params(ListNotesQuery),
    responses((status = 200, body = Paginated<NoteResponse>)))]
pub async fn list(
    State(state): State<AppState>,
    ValidQuery(query): ValidQuery<ListNotesQuery>,
) -> AppResult<Json<Paginated<NoteResponse>>> {
    Ok(Json(service::list(&state.db, query).await?))
}

/// Create a note
///
/// Set `project_id` to attach it to a project, omit it for a global note.
#[utoipa::path(post, path = "/notes", tag = "notes", request_body = CreateNoteRequest,
    responses((status = 201, body = Data<NoteDetailResponse>), (status = 422, body = ErrorResponse)))]
pub async fn create(
    State(state): State<AppState>,
    ValidJson(request): ValidJson<CreateNoteRequest>,
) -> AppResult<(StatusCode, Json<Data<NoteDetailResponse>>)> {
    Ok((StatusCode::CREATED, Data::json(service::create(&state.db, request).await?)))
}

/// List note categories in use
#[utoipa::path(get, path = "/notes/categories", tag = "notes", params(NoteCategoriesQuery),
    responses((status = 200, body = Data<Vec<NoteCategory>>)))]
pub async fn categories(
    State(state): State<AppState>,
    ValidQuery(query): ValidQuery<NoteCategoriesQuery>,
) -> AppResult<Json<Data<Vec<NoteCategory>>>> {
    Ok(Data::json(service::categories(&state.db, query.project_id).await?))
}

/// List a project's notes
#[utoipa::path(get, path = "/projects/{project_id}/notes", tag = "notes",
    params(("project_id" = Uuid, Path, description = "Project id"), ListNotesQuery),
    responses((status = 200, body = Paginated<NoteResponse>)))]
pub async fn list_for_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    ValidQuery(mut query): ValidQuery<ListNotesQuery>,
) -> AppResult<Json<Paginated<NoteResponse>>> {
    query.project_id = Some(project_id);
    query.global = None;
    Ok(Json(service::list(&state.db, query).await?))
}

/// Create a note in a project
#[utoipa::path(post, path = "/projects/{project_id}/notes", tag = "notes",
    params(("project_id" = Uuid, Path, description = "Project id")), request_body = CreateNoteRequest,
    responses(
        (status = 201, body = Data<NoteDetailResponse>),
        (status = 404, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
    ))]
pub async fn create_for_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    ValidJson(mut request): ValidJson<CreateNoteRequest>,
) -> AppResult<(StatusCode, Json<Data<NoteDetailResponse>>)> {
    if !projects::repository::exists(&state.db, project_id).await? {
        return Err(AppError::NotFound("Project"));
    }
    request.project_id = Some(project_id);
    Ok((StatusCode::CREATED, Data::json(service::create(&state.db, request).await?)))
}

/// Get a note with its document
#[utoipa::path(get, path = "/notes/{id}", tag = "notes", params(("id" = Uuid, Path, description = "Note id")),
    responses((status = 200, body = Data<NoteDetailResponse>), (status = 404, body = ErrorResponse)))]
pub async fn get(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<Data<NoteDetailResponse>>> {
    Ok(Data::json(service::get(&state.db, id).await?))
}

/// Update a note
///
/// Partial update, also used by the editor's autosave. The document is
/// validated against the supported rich-text schema.
#[utoipa::path(patch, path = "/notes/{id}", tag = "notes", params(("id" = Uuid, Path, description = "Note id")),
    request_body = UpdateNoteRequest,
    responses(
        (status = 200, body = Data<NoteDetailResponse>),
        (status = 404, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
    ))]
pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    ValidJson(request): ValidJson<UpdateNoteRequest>,
) -> AppResult<Json<Data<NoteDetailResponse>>> {
    Ok(Data::json(service::update(&state.db, id, request).await?))
}

/// Delete a note
#[utoipa::path(delete, path = "/notes/{id}", tag = "notes", params(("id" = Uuid, Path, description = "Note id")),
    responses((status = 204), (status = 404, body = ErrorResponse)))]
pub async fn delete(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    service::delete(&state.db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
