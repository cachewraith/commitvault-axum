use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    dto::{CreateProjectRequest, ListProjectsQuery, ProjectResponse, ProjectSort, UpdateProjectRequest},
    models::ProjectStatus,
    repository::{self, NewProject, ProjectFilter},
};
use crate::{
    errors::{AppError, AppResult},
    modules::tags::{self, models::TagTarget},
    shared::{
        pagination::{Page, SortOrder},
        response::Paginated,
        text::{like_contains, slug_or_fallback},
    },
};

const PROJECT: &str = "Project";
const MAX_SLUG_LEN: usize = 140;

pub async fn list(pool: &PgPool, query: ListProjectsQuery) -> AppResult<Paginated<ProjectResponse>> {
    let page = Page::new(query.page, query.per_page);
    let sort = query.sort.unwrap_or_default();
    let filter = ProjectFilter {
        pattern: query.search.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(like_contains),
        status: query.status,
        tag_id: query.tag_id,
        sort,
        order: query.order.unwrap_or(if matches!(sort, ProjectSort::Name) { SortOrder::Asc } else { SortOrder::Desc }),
    };
    let (items, total) = repository::list(pool, &filter, page).await?;
    Ok(Paginated::new(items, page, total))
}

pub async fn get(pool: &PgPool, id: Uuid) -> AppResult<ProjectResponse> {
    repository::find_view(pool, id).await?.ok_or(AppError::NotFound(PROJECT))
}

/// Fails with 404 unless the project exists. Used by child-resource services.
pub async fn ensure_exists(pool: &PgPool, id: Uuid) -> AppResult<()> {
    if repository::exists(pool, id).await? { Ok(()) } else { Err(AppError::NotFound(PROJECT)) }
}

pub async fn create(pool: &PgPool, request: CreateProjectRequest) -> AppResult<ProjectResponse> {
    // An explicit slug must be free (the unique index reports a conflict);
    // a generated one is made unique by appending a counter.
    let slug = match request.slug {
        Some(slug) => slug,
        None => unique_slug(pool, &request.name).await?,
    };

    let mut tx = pool.begin().await?;
    let id = repository::insert(
        &mut tx,
        &NewProject {
            name: &request.name,
            slug: &slug,
            description: request.description.as_deref(),
            repository_url: request.repository_url.as_deref(),
            default_branch: request.default_branch.as_deref().unwrap_or("main"),
        },
    )
    .await?;
    if let Some(tag_ids) = &request.tag_ids {
        tags::service::set_tags(&mut tx, TagTarget::Project, id, tag_ids).await?;
    }
    tx.commit().await?;
    get(pool, id).await
}

pub async fn update(pool: &PgPool, id: Uuid, request: UpdateProjectRequest) -> AppResult<ProjectResponse> {
    let mut tx = pool.begin().await?;
    let mut project = repository::find_for_update(&mut tx, id).await?.ok_or(AppError::NotFound(PROJECT))?;

    if let Some(name) = request.name {
        project.name = name;
    }
    if let Some(slug) = request.slug {
        project.slug = slug;
    }
    if let Some(description) = request.description {
        project.description = description;
    }
    if let Some(repository_url) = request.repository_url {
        project.repository_url = repository_url;
    }
    if let Some(default_branch) = request.default_branch {
        project.default_branch = default_branch;
    }
    if let Some(status) = request.status
        && status != project.status
    {
        project.status = status;
        project.archived_at = (status == ProjectStatus::Archived).then(Utc::now);
    }

    repository::update(&mut tx, &project).await?;
    if let Some(tag_ids) = &request.tag_ids {
        tags::service::set_tags(&mut tx, TagTarget::Project, id, tag_ids).await?;
    }
    tx.commit().await?;
    get(pool, id).await
}

pub async fn delete(pool: &PgPool, id: Uuid) -> AppResult<()> {
    if repository::delete(pool, id).await? { Ok(()) } else { Err(AppError::NotFound(PROJECT)) }
}

/// `my-app`, or `my-app-2`, `my-app-3`… when taken. A concurrent insert can
/// still win the race; the unique index then reports a conflict.
async fn unique_slug(pool: &PgPool, name: &str) -> AppResult<String> {
    let base = slug_or_fallback(name, "project", MAX_SLUG_LEN - 8);
    let taken = repository::slugs_like(pool, &base).await?;
    Ok(first_free_slug(&base, &taken))
}

fn first_free_slug(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|slug| slug == base) {
        return base.to_owned();
    }
    (2u32..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !taken.contains(candidate))
        .expect("an unbounded counter always yields a free slug")
}

#[cfg(test)]
mod tests {
    use super::first_free_slug;

    #[test]
    fn picks_the_base_slug_when_free() {
        assert_eq!(first_free_slug("api", &[]), "api");
        assert_eq!(first_free_slug("api", &["api-gateway".into()]), "api");
    }

    #[test]
    fn appends_the_first_free_counter() {
        let taken = ["api".to_owned(), "api-2".to_owned(), "api-4".to_owned(), "api-gateway".to_owned()];
        assert_eq!(first_free_slug("api", &taken), "api-3");
    }
}
