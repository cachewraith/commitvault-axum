use sqlx::PgPool;
use uuid::Uuid;

use super::{
    dto::{BranchSummary, CommitResponse, CreateCommitRequest, ListCommitsQuery, SHORT_HASH_LEN, UpdateCommitRequest},
    models::Commit,
    repository::{self, CommitFilter},
};
use crate::{
    errors::{AppError, AppResult},
    modules::{
        projects,
        tags::{self, models::TagTarget},
    },
    shared::{
        pagination::Page,
        response::Paginated,
        text::{like_contains, like_prefix},
    },
};

const COMMIT: &str = "Commit";
const MAX_BRANCHES: i64 = 200;

pub async fn list(pool: &PgPool, query: ListCommitsQuery) -> AppResult<Paginated<CommitResponse>> {
    let page = Page::new(query.page, query.per_page);
    let search = query.search.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let filter = CommitFilter {
        project_id: query.project_id,
        text_pattern: search.map(like_contains),
        // Only treat the search term as a hash prefix when it looks like one.
        search_hash_prefix: search.filter(|s| is_hex(s)).map(|s| like_prefix(&s.to_lowercase())),
        branch: query.branch.map(|b| b.trim().to_owned()).filter(|b| !b.is_empty()),
        hash_prefix: query
            .hash
            .as_deref()
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .map(|h| like_prefix(&h.to_lowercase())),
        date_from: query.date_from,
        date_to: query.date_to,
        deployment: query.deployment_status,
        environment_id: query.environment_id,
        tag_id: query.tag_id,
        release_id: query.release_id,
        sort: query.sort.unwrap_or_default(),
        order: query.order.unwrap_or_default(),
    };
    let (items, total) = repository::list(pool, &filter, page).await?;
    Ok(Paginated::new(items, page, total))
}

fn is_hex(value: &str) -> bool {
    value.len() <= 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

pub async fn get(pool: &PgPool, id: Uuid) -> AppResult<CommitResponse> {
    repository::find_view(pool, id).await?.ok_or(AppError::NotFound(COMMIT))
}

pub async fn create(pool: &PgPool, project_id: Uuid, request: CreateCommitRequest) -> AppResult<CommitResponse> {
    projects::service::ensure_exists(pool, project_id).await?;

    let short_hash = resolve_short_hash(&request.commit_hash, request.short_hash)?;
    let commit = Commit {
        id: Uuid::now_v7(),
        project_id,
        branch_name: request.branch_name,
        commit_hash: request.commit_hash,
        short_hash,
        commit_message: request.commit_message,
        description: request.description,
        commit_date: request.commit_date,
        author_name: request.author_name,
        external_url: request.external_url,
    };

    let mut tx = pool.begin().await?;
    // UNIQUE(project_id, commit_hash) turns a duplicate into a 409.
    repository::insert(&mut tx, &commit).await?;
    if let Some(tag_ids) = &request.tag_ids {
        tags::service::set_tags(&mut tx, TagTarget::Commit, commit.id, tag_ids).await?;
    }
    tx.commit().await?;
    get(pool, commit.id).await
}

pub async fn update(pool: &PgPool, id: Uuid, request: UpdateCommitRequest) -> AppResult<CommitResponse> {
    let mut tx = pool.begin().await?;
    let mut commit = repository::find_for_update(&mut tx, id).await?.ok_or(AppError::NotFound(COMMIT))?;

    if let Some(branch_name) = request.branch_name {
        commit.branch_name = branch_name;
    }
    let hash_changed = request.commit_hash.as_ref().is_some_and(|hash| *hash != commit.commit_hash);
    if let Some(commit_hash) = request.commit_hash {
        commit.commit_hash = commit_hash;
    }
    if request.short_hash.is_some() || hash_changed {
        // A new hash invalidates the old abbreviation unless one was supplied.
        commit.short_hash = resolve_short_hash(&commit.commit_hash, request.short_hash)?;
    }
    if let Some(commit_message) = request.commit_message {
        commit.commit_message = commit_message;
    }
    if let Some(description) = request.description {
        commit.description = description;
    }
    if let Some(commit_date) = request.commit_date {
        commit.commit_date = commit_date;
    }
    if let Some(author_name) = request.author_name {
        commit.author_name = author_name;
    }
    if let Some(external_url) = request.external_url {
        commit.external_url = external_url;
    }

    repository::update(&mut tx, &commit).await?;
    if let Some(tag_ids) = &request.tag_ids {
        tags::service::set_tags(&mut tx, TagTarget::Commit, id, tag_ids).await?;
    }
    tx.commit().await?;
    get(pool, id).await
}

/// Deletes a commit. Deployment history is only removed when the caller asks
/// for it (`cascade`); otherwise the foreign key reports a conflict.
pub async fn delete(pool: &PgPool, id: Uuid, cascade: bool) -> AppResult<()> {
    let mut tx = pool.begin().await?;
    if cascade {
        repository::delete_deployments(&mut tx, id).await?;
    }
    if !repository::delete(&mut tx, id).await? {
        return Err(AppError::NotFound(COMMIT));
    }
    tx.commit().await?;
    Ok(())
}

pub async fn branches(pool: &PgPool, project_id: Uuid) -> AppResult<Vec<BranchSummary>> {
    projects::service::ensure_exists(pool, project_id).await?;
    Ok(repository::branches(pool, project_id, MAX_BRANCHES).await?)
}

/// The abbreviation must actually abbreviate the hash it is shown for.
fn resolve_short_hash(commit_hash: &str, short_hash: Option<String>) -> AppResult<String> {
    match short_hash {
        Some(short) if commit_hash.starts_with(&short) => Ok(short),
        Some(_) => Err(AppError::invalid("short_hash", "Must be a prefix of the commit hash")),
        None => Ok(commit_hash[..SHORT_HASH_LEN.min(commit_hash.len())].to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_hash_defaults_to_seven_characters() {
        let hash = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(resolve_short_hash(hash, None).unwrap(), "0123456");
        assert_eq!(resolve_short_hash("abcdef1", None).unwrap(), "abcdef1");
    }

    #[test]
    fn explicit_short_hash_must_be_a_prefix() {
        let hash = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(resolve_short_hash(hash, Some("0123456789".into())).unwrap(), "0123456789");
        assert!(resolve_short_hash(hash, Some("abcdef0".into())).is_err());
    }

    #[test]
    fn search_terms_are_only_hash_prefixes_when_hexadecimal() {
        assert!(is_hex("a1B2c3"));
        assert!(!is_hex("fix login"));
        assert!(!is_hex(&"a".repeat(65)));
    }
}
