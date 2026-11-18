use std::collections::HashSet;

use sqlx::PgPool;
use uuid::Uuid;

use super::{
    dto::{
        CreateEnvironmentRequest, DEFAULT_ENVIRONMENT_COLOR, EnvironmentResponse, EnvironmentSort,
        ListEnvironmentsQuery, MAX_ENVIRONMENTS_PER_PROJECT, ReorderEnvironmentsRequest, UpdateEnvironmentRequest,
    },
    models::Environment,
    repository::{self, EnvironmentFilter},
};
use crate::{
    errors::{AppError, AppResult},
    modules::projects,
    shared::{
        pagination::{Page, SortOrder},
        response::Paginated,
        text::{like_contains, slug_or_fallback},
    },
};

const ENVIRONMENT: &str = "Environment";

pub async fn list(pool: &PgPool, query: ListEnvironmentsQuery) -> AppResult<Paginated<EnvironmentResponse>> {
    let page = Page::new(query.page, query.per_page);
    let sort = query.sort.unwrap_or_default();
    let filter = EnvironmentFilter {
        project_id: query.project_id,
        pattern: query.search.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(like_contains),
        sort,
        order: query.order.unwrap_or(match sort {
            EnvironmentSort::Name | EnvironmentSort::Project => SortOrder::Asc,
            _ => SortOrder::Desc,
        }),
        ..Default::default()
    };
    let (items, total) = repository::list(pool, &filter, page).await?;
    Ok(Paginated::new(items, page, total))
}

pub async fn list_for_project(pool: &PgPool, project_id: Uuid) -> AppResult<Vec<EnvironmentResponse>> {
    projects::service::ensure_exists(pool, project_id).await?;
    Ok(repository::list_for_project(pool, project_id).await?)
}

pub async fn get(pool: &PgPool, id: Uuid) -> AppResult<EnvironmentResponse> {
    repository::find_view(pool, id).await?.ok_or(AppError::NotFound(ENVIRONMENT))
}

pub async fn create(
    pool: &PgPool,
    project_id: Uuid,
    request: CreateEnvironmentRequest,
) -> AppResult<EnvironmentResponse> {
    let mut tx = pool.begin().await?;
    // Locking the project row serialises concurrent creations, so the limit
    // and the next position are computed against a stable set.
    projects::repository::find_for_update(&mut tx, project_id).await?.ok_or(AppError::NotFound("Project"))?;

    let (count, max_position) = repository::count_and_max_position(&mut tx, project_id).await?;
    if count >= MAX_ENVIRONMENTS_PER_PROJECT {
        return Err(AppError::conflict(format!(
            "A project can have at most {MAX_ENVIRONMENTS_PER_PROJECT} environments"
        )));
    }

    let environment = Environment {
        id: Uuid::now_v7(),
        project_id,
        slug: slug_or_fallback(&request.name, "env", 100),
        name: request.name,
        description: request.description,
        color: request.color.unwrap_or_else(|| DEFAULT_ENVIRONMENT_COLOR.to_owned()),
        server_label: request.server_label,
    };
    repository::insert(&mut tx, &environment, max_position + 1).await?;
    tx.commit().await?;
    get(pool, environment.id).await
}

pub async fn update(pool: &PgPool, id: Uuid, request: UpdateEnvironmentRequest) -> AppResult<EnvironmentResponse> {
    let mut tx = pool.begin().await?;
    let mut environment = repository::find_for_update(&mut tx, id).await?.ok_or(AppError::NotFound(ENVIRONMENT))?;

    if let Some(name) = request.name {
        environment.slug = slug_or_fallback(&name, "env", 100);
        environment.name = name;
    }
    if let Some(description) = request.description {
        environment.description = description;
    }
    if let Some(color) = request.color {
        environment.color = color;
    }
    if let Some(server_label) = request.server_label {
        environment.server_label = server_label;
    }

    repository::update(&mut tx, &environment).await?;
    tx.commit().await?;
    get(pool, id).await
}

/// Applies a new display order. The request must list every environment of
/// the project exactly once, so a stale client cannot silently drop or
/// duplicate positions.
pub async fn reorder(
    pool: &PgPool,
    project_id: Uuid,
    request: ReorderEnvironmentsRequest,
) -> AppResult<Vec<EnvironmentResponse>> {
    projects::service::ensure_exists(pool, project_id).await?;

    let mut tx = pool.begin().await?;
    let existing: HashSet<Uuid> = repository::ids_for_update(&mut tx, project_id).await?.into_iter().collect();
    let requested: HashSet<Uuid> = request.environment_ids.iter().copied().collect();
    if requested.len() != request.environment_ids.len() || requested != existing {
        return Err(AppError::invalid("environment_ids", "Must contain every environment of the project exactly once"));
    }
    repository::set_positions(&mut tx, project_id, &request.environment_ids).await?;
    tx.commit().await?;

    Ok(repository::list_for_project(pool, project_id).await?)
}

/// Deletes an environment. Deployment history is only removed when the caller
/// asks for it (`cascade`); otherwise the foreign key reports a conflict.
pub async fn delete(pool: &PgPool, id: Uuid, cascade: bool) -> AppResult<()> {
    let mut tx = pool.begin().await?;
    if cascade {
        repository::delete_deployments(&mut tx, id).await?;
    }
    if !repository::delete(&mut tx, id).await? {
        return Err(AppError::NotFound(ENVIRONMENT));
    }
    tx.commit().await?;
    Ok(())
}
