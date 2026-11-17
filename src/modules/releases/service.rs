use std::collections::BTreeSet;

use chrono::Utc;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::{
    dto::{
        CreateReleaseRequest, ListReleasesQuery, ReleaseDetailResponse, ReleaseResponse, ReleaseSort,
        UpdateReleaseRequest,
    },
    models::{Release, ReleaseStatus},
    repository::{self, ReleaseFilter},
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
        richtext::sanitize_document,
        text::like_contains,
    },
};

const RELEASE: &str = "Release";

pub async fn list(pool: &PgPool, query: ListReleasesQuery) -> AppResult<Paginated<ReleaseResponse>> {
    let page = Page::new(query.page, query.per_page);
    let filter = ReleaseFilter {
        project_id: query.project_id,
        status: query.status,
        pattern: query.search.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(like_contains),
        tag_id: query.tag_id,
        commit_id: query.commit_id,
        sort: query.sort.unwrap_or(ReleaseSort::ReleasedAt),
        order: query.order.unwrap_or(SortOrder::Desc),
    };
    let (items, total) = repository::list(pool, &filter, page).await?;
    Ok(Paginated::new(items, page, total))
}

pub async fn get(pool: &PgPool, id: Uuid) -> AppResult<ReleaseDetailResponse> {
    repository::find_detail(pool, id).await?.ok_or(AppError::NotFound(RELEASE))
}

pub async fn create(
    pool: &PgPool,
    project_id: Uuid,
    request: CreateReleaseRequest,
) -> AppResult<ReleaseDetailResponse> {
    projects::service::ensure_exists(pool, project_id).await?;

    let (release_notes, release_notes_text) = sanitize_notes(request.release_notes.as_ref())?;
    let status = request.status.unwrap_or(ReleaseStatus::Draft);
    let release = Release {
        id: Uuid::now_v7(),
        project_id,
        version: request.version,
        title: request.title,
        description: request.description,
        release_notes,
        release_notes_text,
        released_at: resolve_released_at(status, request.released_at),
        status,
    };

    let mut tx = pool.begin().await?;
    repository::insert(&mut tx, &release).await?;
    if let Some(commit_ids) = &request.commit_ids {
        set_commits(&mut tx, &release, commit_ids).await?;
    }
    if let Some(tag_ids) = &request.tag_ids {
        tags::service::set_tags(&mut tx, TagTarget::Release, release.id, tag_ids).await?;
    }
    tx.commit().await?;
    get(pool, release.id).await
}

pub async fn update(pool: &PgPool, id: Uuid, request: UpdateReleaseRequest) -> AppResult<ReleaseDetailResponse> {
    let mut tx = pool.begin().await?;
    let mut release = repository::find_for_update(&mut tx, id).await?.ok_or(AppError::NotFound(RELEASE))?;

    if let Some(version) = request.version {
        release.version = version;
    }
    if let Some(title) = request.title {
        release.title = title;
    }
    if let Some(description) = request.description {
        release.description = description;
    }
    if let Some(notes) = request.release_notes {
        (release.release_notes, release.release_notes_text) = sanitize_notes(notes.as_ref())?;
    }
    if let Some(released_at) = request.released_at {
        release.released_at = released_at;
    }
    if let Some(status) = request.status {
        release.status = status;
    }
    release.released_at = resolve_released_at(release.status, release.released_at);

    repository::update(&mut tx, &release).await?;
    if let Some(commit_ids) = &request.commit_ids {
        set_commits(&mut tx, &release, commit_ids).await?;
    }
    if let Some(tag_ids) = &request.tag_ids {
        tags::service::set_tags(&mut tx, TagTarget::Release, id, tag_ids).await?;
    }
    tx.commit().await?;
    get(pool, id).await
}

pub async fn delete(pool: &PgPool, id: Uuid) -> AppResult<()> {
    if repository::delete(pool, id).await? { Ok(()) } else { Err(AppError::NotFound(RELEASE)) }
}

/// A release marked `released` always has a release date; when none was
/// given it is the moment the status was set.
fn resolve_released_at(
    status: ReleaseStatus,
    released_at: Option<chrono::DateTime<Utc>>,
) -> Option<chrono::DateTime<Utc>> {
    match (status, released_at) {
        (ReleaseStatus::Released, None) => Some(Utc::now()),
        (_, released_at) => released_at,
    }
}

fn sanitize_notes(notes: Option<&serde_json::Value>) -> AppResult<(Option<serde_json::Value>, String)> {
    match notes {
        None => Ok((None, String::new())),
        Some(document) => {
            let sanitized =
                sanitize_document(document).map_err(|err| AppError::invalid("release_notes", err.to_string()))?;
            Ok((Some(sanitized.json), sanitized.text))
        }
    }
}

async fn set_commits(conn: &mut PgConnection, release: &Release, commit_ids: &[Uuid]) -> AppResult<()> {
    let unique: Vec<Uuid> = commit_ids.iter().copied().collect::<BTreeSet<_>>().into_iter().collect();
    let attached = repository::replace_commits(conn, release.id, release.project_id, &unique).await?;
    if attached as usize != unique.len() {
        return Err(AppError::invalid("commit_ids", "One or more commits do not exist in this project"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_status_gets_a_date_when_missing() {
        assert!(resolve_released_at(ReleaseStatus::Released, None).is_some());
        assert!(resolve_released_at(ReleaseStatus::Draft, None).is_none());
        let explicit = Utc::now() - chrono::Duration::days(3);
        assert_eq!(resolve_released_at(ReleaseStatus::Released, Some(explicit)), Some(explicit));
        assert_eq!(resolve_released_at(ReleaseStatus::Draft, Some(explicit)), Some(explicit), "planned dates are kept");
    }
}
