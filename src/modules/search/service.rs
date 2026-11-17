use sqlx::PgPool;

use super::{
    dto::{DEFAULT_LIMIT, MAX_LIMIT, SearchHit, SearchKind, SearchQuery, SearchResponse},
    repository::{self, HitRow, SearchParams},
};
use crate::{
    errors::{AppError, AppResult},
    shared::text::{like_contains, like_prefix},
};

const MIN_QUERY_CHARS: usize = 2;
const MAX_QUERY_CHARS: usize = 100;

pub async fn search(pool: &PgPool, query: SearchQuery) -> AppResult<SearchResponse> {
    let term = query.q.trim();
    let chars = term.chars().count();
    if !(MIN_QUERY_CHARS..=MAX_QUERY_CHARS).contains(&chars) {
        return Err(AppError::invalid(
            "q",
            format!("Enter between {MIN_QUERY_CHARS} and {MAX_QUERY_CHARS} characters"),
        ));
    }

    let pattern = like_contains(term);
    let hash_prefix = looks_like_hash(term).then(|| like_prefix(&term.to_lowercase()));
    let params = SearchParams {
        pattern: &pattern,
        hash_prefix: hash_prefix.as_deref(),
        project_id: query.project_id,
        limit: i64::from(query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)),
    };

    let (projects, commits, deployments, releases, notes, tags) = tokio::try_join!(
        repository::projects(pool, &params),
        repository::commits(pool, &params),
        repository::deployments(pool, &params),
        repository::releases(pool, &params),
        repository::notes(pool, &params),
        repository::tags(pool, &params),
    )?;

    let hits = |rows: Vec<HitRow>, kind: SearchKind| -> Vec<SearchHit> {
        rows.into_iter()
            .map(|row| SearchHit {
                id: row.id,
                kind,
                title: row.title,
                subtitle: row.subtitle,
                project: row.project,
                occurred_at: row.occurred_at,
            })
            .collect()
    };

    let response = SearchResponse {
        query: term.to_owned(),
        projects: hits(projects, SearchKind::Project),
        commits: hits(commits, SearchKind::Commit),
        deployments: hits(deployments, SearchKind::Deployment),
        releases: hits(releases, SearchKind::Release),
        notes: hits(notes, SearchKind::Note),
        tags: hits(tags, SearchKind::Tag),
        total: 0,
    };
    let total = response.projects.len()
        + response.commits.len()
        + response.deployments.len()
        + response.releases.len()
        + response.notes.len()
        + response.tags.len();
    Ok(SearchResponse { total, ..response })
}

/// Hex strings of at least four characters are also tried as hash prefixes.
fn looks_like_hash(term: &str) -> bool {
    (4..=64).contains(&term.len()) && term.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::looks_like_hash;

    #[test]
    fn detects_hash_like_terms() {
        assert!(looks_like_hash("a1b2"));
        assert!(looks_like_hash("A1B2C3D4"));
        assert!(!looks_like_hash("abc"), "too short to be a useful prefix");
        assert!(!looks_like_hash("login fix"));
        assert!(!looks_like_hash(&"a".repeat(65)));
    }
}
