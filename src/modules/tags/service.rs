use std::collections::BTreeSet;

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::{
    dto::{CreateTagRequest, DEFAULT_TAG_COLOR, ListTagsQuery, TagResponse, UpdateTagRequest},
    models::TagTarget,
    repository::{self, TagFilter},
};
use crate::{
    errors::{AppError, AppResult},
    shared::{
        pagination::{Page, SortOrder},
        response::Paginated,
        text::{like_contains, slug_or_fallback},
    },
};

const TAG: &str = "Tag";

pub async fn list(pool: &PgPool, query: ListTagsQuery) -> AppResult<Paginated<TagResponse>> {
    let page = Page::new(query.page, query.per_page);
    let sort = query.sort.unwrap_or_default();
    let filter = TagFilter {
        name_pattern: query.search.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(like_contains),
        sort,
        // Names read best A→Z; everything else newest/most-used first.
        order: query.order.unwrap_or(match sort {
            super::dto::TagSort::Name => SortOrder::Asc,
            _ => SortOrder::Desc,
        }),
    };
    let (items, total) = repository::list(pool, &filter, page).await?;
    Ok(Paginated::new(items, page, total))
}

pub async fn get(pool: &PgPool, id: Uuid) -> AppResult<TagResponse> {
    repository::find_view(pool, id).await?.ok_or(AppError::NotFound(TAG))
}

pub async fn create(pool: &PgPool, request: CreateTagRequest) -> AppResult<TagResponse> {
    let slug = slug_or_fallback(&request.name, "tag", 60);
    let color = request.color.as_deref().unwrap_or(DEFAULT_TAG_COLOR);
    let id = repository::insert(pool, &request.name, &slug, color).await?;
    get(pool, id).await
}

pub async fn update(pool: &PgPool, id: Uuid, request: UpdateTagRequest) -> AppResult<TagResponse> {
    let mut tx = pool.begin().await?;
    let mut tag = repository::find(&mut tx, id).await?.ok_or(AppError::NotFound(TAG))?;
    if let Some(name) = request.name {
        tag.slug = slug_or_fallback(&name, "tag", 60);
        tag.name = name;
    }
    if let Some(color) = request.color {
        tag.color = color;
    }
    repository::update(&mut tx, &tag).await?;
    tx.commit().await?;
    get(pool, id).await
}

pub async fn delete(pool: &PgPool, id: Uuid) -> AppResult<()> {
    if repository::delete(pool, id).await? { Ok(()) } else { Err(AppError::NotFound(TAG)) }
}

/// Replaces the tags attached to `entity_id`. Must run inside the caller's
/// transaction so the entity and its tags change atomically.
pub async fn set_tags(conn: &mut PgConnection, target: TagTarget, entity_id: Uuid, tag_ids: &[Uuid]) -> AppResult<()> {
    let unique: Vec<Uuid> = tag_ids.iter().copied().collect::<BTreeSet<_>>().into_iter().collect();
    let attached = repository::replace_links(conn, target, entity_id, &unique).await?;
    if attached as usize != unique.len() {
        return Err(AppError::invalid("tag_ids", "One or more tags do not exist"));
    }
    Ok(())
}
