use sqlx::PgPool;
use uuid::Uuid;

use super::{
    dto::{
        CreateNoteRequest, ListNotesQuery, NoteCategory, NoteDetailResponse, NoteResponse, NoteSort, UpdateNoteRequest,
    },
    models::Note,
    repository::{self, NoteFilter},
};
use crate::{
    errors::{AppError, AppResult},
    modules::{
        projects,
        tags::{self, models::TagTarget},
    },
    shared::{
        pagination::{Page, SortOrder},
        response::Paginated,
        richtext::{SanitizedDocument, empty_document, sanitize_document},
        text::{excerpt, like_contains},
    },
};

const NOTE: &str = "Note";
const EXCERPT_CHARS: usize = 240;
const MAX_CATEGORIES: i64 = 200;

pub async fn list(pool: &PgPool, query: ListNotesQuery) -> AppResult<Paginated<NoteResponse>> {
    let page = Page::new(query.page, query.per_page);
    let sort = query.sort.unwrap_or_default();
    let filter = NoteFilter {
        project_id: query.project_id,
        global_only: query.global.unwrap_or(false),
        category: query.category.map(|c| c.trim().to_owned()).filter(|c| !c.is_empty()),
        pinned: query.pinned,
        pattern: query.search.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(like_contains),
        tag_id: query.tag_id,
        sort,
        order: query.order.unwrap_or(if matches!(sort, NoteSort::Title) { SortOrder::Asc } else { SortOrder::Desc }),
        pinned_first: true,
    };
    let (items, total) = repository::list(pool, &filter, page).await?;
    Ok(Paginated::new(items, page, total))
}

/// Most recently edited notes, ignoring pins (dashboard).
pub async fn recent(pool: &PgPool, limit: u32) -> AppResult<Vec<NoteResponse>> {
    let filter = NoteFilter { sort: NoteSort::UpdatedAt, order: SortOrder::Desc, ..Default::default() };
    Ok(repository::list(pool, &filter, Page::first(limit)).await?.0)
}

pub async fn get(pool: &PgPool, id: Uuid) -> AppResult<NoteDetailResponse> {
    repository::find_detail(pool, id).await?.ok_or(AppError::NotFound(NOTE))
}

pub async fn create(pool: &PgPool, request: CreateNoteRequest) -> AppResult<NoteDetailResponse> {
    if let Some(project_id) = request.project_id {
        ensure_project(pool, project_id).await?;
    }
    let content = sanitize(&request.content_json.unwrap_or_else(empty_document))?;
    let note = Note {
        id: Uuid::now_v7(),
        project_id: request.project_id,
        title: request.title,
        excerpt: excerpt(&content.text, EXCERPT_CHARS),
        content_json: content.json,
        content_text: content.text,
        category: request.category,
        is_pinned: request.is_pinned.unwrap_or(false),
    };

    let mut tx = pool.begin().await?;
    repository::insert(&mut tx, &note).await?;
    if let Some(tag_ids) = &request.tag_ids {
        tags::service::set_tags(&mut tx, TagTarget::Note, note.id, tag_ids).await?;
    }
    tx.commit().await?;
    get(pool, note.id).await
}

/// Partial update; also the autosave endpoint of the editor.
pub async fn update(pool: &PgPool, id: Uuid, request: UpdateNoteRequest) -> AppResult<NoteDetailResponse> {
    if let Some(Some(project_id)) = request.project_id {
        ensure_project(pool, project_id).await?;
    }
    // Validate the document before taking the row lock.
    let content = request.content_json.as_ref().map(sanitize).transpose()?;

    let mut tx = pool.begin().await?;
    let mut note = repository::find_for_update(&mut tx, id).await?.ok_or(AppError::NotFound(NOTE))?;

    if let Some(project_id) = request.project_id {
        note.project_id = project_id;
    }
    if let Some(title) = request.title {
        note.title = title;
    }
    if let Some(content) = content {
        note.excerpt = excerpt(&content.text, EXCERPT_CHARS);
        note.content_json = content.json;
        note.content_text = content.text;
    }
    if let Some(category) = request.category {
        note.category = category;
    }
    if let Some(is_pinned) = request.is_pinned {
        note.is_pinned = is_pinned;
    }

    repository::update(&mut tx, &note).await?;
    if let Some(tag_ids) = &request.tag_ids {
        tags::service::set_tags(&mut tx, TagTarget::Note, id, tag_ids).await?;
    }
    tx.commit().await?;
    get(pool, id).await
}

pub async fn delete(pool: &PgPool, id: Uuid) -> AppResult<()> {
    if repository::delete(pool, id).await? { Ok(()) } else { Err(AppError::NotFound(NOTE)) }
}

pub async fn categories(pool: &PgPool, project_id: Option<Uuid>) -> AppResult<Vec<NoteCategory>> {
    Ok(repository::categories(pool, project_id, MAX_CATEGORIES).await?)
}

fn sanitize(document: &serde_json::Value) -> AppResult<SanitizedDocument> {
    sanitize_document(document).map_err(|err| AppError::invalid("content_json", err.to_string()))
}

/// A note can only be attached to an existing project; report it on the field.
async fn ensure_project(pool: &PgPool, project_id: Uuid) -> AppResult<()> {
    if projects::repository::exists(pool, project_id).await? {
        Ok(())
    } else {
        Err(AppError::invalid("project_id", "Project does not exist"))
    }
}
