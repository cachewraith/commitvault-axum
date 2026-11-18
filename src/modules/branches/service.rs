use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::{
    dto::{
        BranchDetailResponse, BranchResponse, BranchSort, CreateBranchRequest, ListBranchesQuery, UpdateBranchRequest,
    },
    models::Branch,
    repository::{self, BranchFilter},
};
use crate::{
    errors::{AppError, AppResult},
    modules::projects,
    shared::{
        pagination::{Page, SortOrder},
        response::Paginated,
        text::like_contains,
    },
};

const BRANCH: &str = "Branch";

pub async fn list(pool: &PgPool, project_id: Uuid, query: ListBranchesQuery) -> AppResult<Paginated<BranchResponse>> {
    projects::service::ensure_exists(pool, project_id).await?;
    let page = Page::new(query.page, query.per_page);
    let sort = query.sort.unwrap_or_default();
    let filter = BranchFilter {
        project_id,
        pattern: query.search.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(like_contains),
        sort,
        order: query.order.unwrap_or(if sort == BranchSort::Name { SortOrder::Asc } else { SortOrder::Desc }),
    };
    let (items, total) = repository::list(pool, &filter, page).await?;
    Ok(Paginated::new(items, page, total))
}

pub async fn get(pool: &PgPool, id: Uuid) -> AppResult<BranchResponse> {
    repository::find_view(pool, id).await?.ok_or(AppError::NotFound(BRANCH))
}

pub async fn get_detail(pool: &PgPool, id: Uuid) -> AppResult<BranchDetailResponse> {
    let (branch, environments) = tokio::try_join!(repository::find_view(pool, id), repository::environments(pool, id))?;
    let branch = branch.ok_or(AppError::NotFound(BRANCH))?;
    Ok(BranchDetailResponse { branch, environments })
}

pub async fn create(pool: &PgPool, project_id: Uuid, request: CreateBranchRequest) -> AppResult<BranchResponse> {
    projects::service::ensure_exists(pool, project_id).await?;
    let branch = Branch { id: Uuid::now_v7(), project_id, name: request.name, description: request.description };
    let mut tx = pool.begin().await?;
    // UNIQUE(project_id, name) turns a duplicate into a 409 on `name`.
    repository::insert(&mut tx, &branch).await?;
    tx.commit().await?;
    get(pool, branch.id).await
}

pub async fn update(pool: &PgPool, id: Uuid, request: UpdateBranchRequest) -> AppResult<BranchResponse> {
    let mut tx = pool.begin().await?;
    let mut branch = repository::find_for_update(&mut tx, id).await?.ok_or(AppError::NotFound(BRANCH))?;

    if let Some(name) = request.name
        && name != branch.name
    {
        repository::rename_default_branch(&mut tx, branch.project_id, &branch.name, &name).await?;
        branch.name = name;
    }
    if let Some(description) = request.description {
        branch.description = description;
    }

    repository::update(&mut tx, &branch).await?;
    tx.commit().await?;
    get(pool, id).await
}

/// Deletes a branch. Its commits (and their deployment records) are only
/// removed when the caller asks for it (`cascade`); otherwise the foreign key
/// reports a conflict.
pub async fn delete(pool: &PgPool, id: Uuid, cascade: bool) -> AppResult<()> {
    let mut tx = pool.begin().await?;
    if cascade {
        repository::delete_commits(&mut tx, id).await?;
    }
    if !repository::delete(&mut tx, id).await? {
        return Err(AppError::NotFound(BRANCH));
    }
    tx.commit().await?;
    Ok(())
}

/// A commit may only reference a branch of its own project. The composite
/// foreign key enforces the same rule; checking first gives a field error.
pub async fn ensure_in_project(conn: &mut PgConnection, project_id: Uuid, branch_id: Uuid) -> AppResult<()> {
    match repository::project_id_of(conn, branch_id).await? {
        Some(owner) if owner == project_id => Ok(()),
        _ => Err(AppError::invalid("branch_id", "Choose a branch of this project")),
    }
}

/// Makes sure the project has a branch called `name` (its default branch).
pub async fn ensure_named(conn: &mut PgConnection, project_id: Uuid, name: &str) -> AppResult<()> {
    Ok(repository::insert_if_missing(conn, project_id, name).await?)
}
