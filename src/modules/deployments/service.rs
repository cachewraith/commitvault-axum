use chrono::Utc;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::{
    dto::{
        CompareEnvironmentsQuery, ComparedCommit, ComparisonSide, CreateDeploymentRequest, DeploymentResponse,
        DeploymentSort, EnvironmentComparison, ListDeploymentsQuery, SharedCommit, UpdateDeploymentRequest,
    },
    models::{Deployment, DeploymentStatus},
    repository::{self, ComparisonRow, DeploymentFilter},
};
use crate::{
    errors::{AppError, AppResult, FieldErrors},
    modules::{
        environments::{self, dto::EnvironmentResponse},
        projects,
        tags::{self, models::TagTarget},
    },
    shared::{
        pagination::{Page, SortOrder},
        refs::EnvironmentRef,
        response::Paginated,
        text::{like_contains, like_prefix},
    },
};

const DEPLOYMENT: &str = "Deployment";
/// Maximum commits returned by one comparison.
const COMPARISON_LIMIT: i64 = 300;

/// Shown with every comparison so the result is never over-interpreted.
pub const COMPARISON_NOTICE: &str = "Based only on deployments recorded in CommitVault. A commit without a record \
    in an environment may still be deployed there, and no ordering between environments (ahead or behind) is implied.";

pub async fn list(pool: &PgPool, query: ListDeploymentsQuery) -> AppResult<Paginated<DeploymentResponse>> {
    let page = Page::new(query.page, query.per_page);
    let search = query.search.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let filter = DeploymentFilter {
        project_id: query.project_id,
        environment_id: query.environment_id,
        commit_id: query.commit_id,
        status: query.status,
        date_from: query.date_from,
        date_to: query.date_to,
        text_pattern: search.map(like_contains),
        search_hash_prefix: search
            .filter(|s| s.len() <= 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(|s| like_prefix(&s.to_lowercase())),
        tag_id: query.tag_id,
        sort: query.sort.unwrap_or_default(),
        order: query.order.unwrap_or(match query.sort {
            Some(DeploymentSort::Title | DeploymentSort::Status) => SortOrder::Asc,
            _ => SortOrder::Desc,
        }),
    };
    let (items, total) = repository::list(pool, &filter, page).await?;
    Ok(Paginated::new(items, page, total))
}

pub async fn get(pool: &PgPool, id: Uuid) -> AppResult<DeploymentResponse> {
    repository::find_view(pool, id).await?.ok_or(AppError::NotFound(DEPLOYMENT))
}

pub async fn create(
    pool: &PgPool,
    project_id: Uuid,
    request: CreateDeploymentRequest,
) -> AppResult<DeploymentResponse> {
    projects::service::ensure_exists(pool, project_id).await?;

    let deployment = Deployment {
        id: Uuid::now_v7(),
        project_id,
        environment_id: request.environment_id,
        commit_id: request.commit_id,
        deployment_title: request.deployment_title,
        deployment_notes: request.deployment_notes,
        deployed_at: request.deployed_at.unwrap_or_else(Utc::now),
        status: request.status.unwrap_or(DeploymentStatus::Successful),
    };

    let mut tx = pool.begin().await?;
    ensure_same_project(&mut tx, &deployment).await?;
    repository::insert(&mut tx, &deployment).await?;
    if let Some(tag_ids) = &request.tag_ids {
        tags::service::set_tags(&mut tx, TagTarget::Deployment, deployment.id, tag_ids).await?;
    }
    tx.commit().await?;
    get(pool, deployment.id).await
}

pub async fn update(pool: &PgPool, id: Uuid, request: UpdateDeploymentRequest) -> AppResult<DeploymentResponse> {
    let mut tx = pool.begin().await?;
    let mut deployment = repository::find_for_update(&mut tx, id).await?.ok_or(AppError::NotFound(DEPLOYMENT))?;

    let references_changed = request.environment_id.is_some() || request.commit_id.is_some();
    if let Some(environment_id) = request.environment_id {
        deployment.environment_id = environment_id;
    }
    if let Some(commit_id) = request.commit_id {
        deployment.commit_id = commit_id;
    }
    if let Some(title) = request.deployment_title {
        deployment.deployment_title = title;
    }
    if let Some(notes) = request.deployment_notes {
        deployment.deployment_notes = notes;
    }
    if let Some(deployed_at) = request.deployed_at {
        deployment.deployed_at = deployed_at;
    }
    if let Some(status) = request.status {
        deployment.status = status;
    }

    if references_changed {
        ensure_same_project(&mut tx, &deployment).await?;
    }
    repository::update(&mut tx, &deployment).await?;
    if let Some(tag_ids) = &request.tag_ids {
        tags::service::set_tags(&mut tx, TagTarget::Deployment, id, tag_ids).await?;
    }
    tx.commit().await?;
    get(pool, id).await
}

pub async fn delete(pool: &PgPool, id: Uuid) -> AppResult<()> {
    if repository::delete(pool, id).await? { Ok(()) } else { Err(AppError::NotFound(DEPLOYMENT)) }
}

/// A deployment may only reference an environment and a commit of its own
/// project. The composite foreign keys enforce this too; checking here turns
/// a violation into a precise, per-field validation error.
async fn ensure_same_project(conn: &mut PgConnection, deployment: &Deployment) -> AppResult<()> {
    let (environment_ok, commit_ok) = repository::references_belong_to_project(
        conn,
        deployment.project_id,
        deployment.environment_id,
        deployment.commit_id,
    )
    .await?;

    let mut errors = FieldErrors::new();
    if !environment_ok {
        errors.insert("environment_id".into(), vec!["Environment does not exist in this project".into()]);
    }
    if !commit_ok {
        errors.insert("commit_id".into(), vec!["Commit does not exist in this project".into()]);
    }
    if errors.is_empty() { Ok(()) } else { Err(AppError::Validation(errors)) }
}

/// Compares what has been *recorded* for two environments of a project.
pub async fn compare(
    pool: &PgPool,
    project_id: Uuid,
    query: CompareEnvironmentsQuery,
) -> AppResult<EnvironmentComparison> {
    if query.base == query.target {
        return Err(AppError::invalid("target", "Choose two different environments"));
    }
    let all_statuses = query.include_unsuccessful.unwrap_or(false);

    let (base, target) = tokio::try_join!(
        environments::repository::find_view(pool, query.base),
        environments::repository::find_view(pool, query.target),
    )?;
    let in_project = |env: Option<EnvironmentResponse>, field: &str| {
        env.filter(|e| e.project_id == project_id)
            .ok_or_else(|| AppError::invalid(field, "Environment does not exist in this project"))
    };
    let (base, target) = (in_project(base, "base")?, in_project(target, "target")?);

    let (rows, counts) = tokio::try_join!(
        repository::compare(pool, project_id, base.id, target.id, all_statuses, COMPARISON_LIMIT + 1),
        repository::compare_counts(pool, project_id, base.id, target.id, all_statuses),
    )?;

    let truncated = rows.len() as i64 > COMPARISON_LIMIT;
    let (only_in_base, only_in_target, in_both) = partition(rows.into_iter().take(COMPARISON_LIMIT as usize));

    let side = |env: EnvironmentResponse, recorded_commit_count: i64| ComparisonSide {
        latest_deployment: if all_statuses { env.latest_deployment } else { env.latest_successful_deployment },
        environment: EnvironmentRef { id: env.id, name: env.name, slug: env.slug, color: env.color },
        recorded_commit_count,
    };

    Ok(EnvironmentComparison {
        base: side(base, counts.only_in_base + counts.in_both),
        target: side(target, counts.only_in_target + counts.in_both),
        statuses_considered: if all_statuses {
            vec![
                DeploymentStatus::Successful,
                DeploymentStatus::Failed,
                DeploymentStatus::RolledBack,
                DeploymentStatus::Pending,
            ]
        } else {
            vec![DeploymentStatus::Successful]
        },
        counts,
        only_in_base,
        only_in_target,
        in_both,
        truncated,
        notice: COMPARISON_NOTICE,
    })
}

fn partition(
    rows: impl Iterator<Item = ComparisonRow>,
) -> (Vec<ComparedCommit>, Vec<ComparedCommit>, Vec<SharedCommit>) {
    let (mut only_in_base, mut only_in_target, mut in_both) = (Vec::new(), Vec::new(), Vec::new());
    for row in rows {
        match (row.base_last, row.target_last) {
            (Some(base_last), Some(target_last)) => in_both.push(SharedCommit {
                commit: row.commit,
                base_last_deployed_at: base_last,
                target_last_deployed_at: target_last,
            }),
            (Some(last), None) => only_in_base.push(ComparedCommit {
                commit: row.commit,
                deployment_count: row.base_count,
                last_deployed_at: last,
            }),
            (None, Some(last)) => only_in_target.push(ComparedCommit {
                commit: row.commit,
                deployment_count: row.target_count,
                last_deployed_at: last,
            }),
            (None, None) => {}
        }
    }
    (only_in_base, only_in_target, in_both)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::shared::refs::CommitBrief;

    fn row(hash: &str, base: Option<i64>, target: Option<i64>) -> ComparisonRow {
        let at = |secs: i64| Utc.timestamp_opt(secs, 0).unwrap();
        ComparisonRow {
            commit: CommitBrief {
                id: Uuid::now_v7(),
                short_hash: hash.into(),
                commit_hash: hash.into(),
                commit_message: "msg".into(),
                branch_name: "main".into(),
                commit_date: at(0),
            },
            base_count: i64::from(base.is_some()),
            base_last: base.map(at),
            target_count: i64::from(target.is_some()),
            target_last: target.map(at),
        }
    }

    #[test]
    fn partitions_commits_by_where_records_exist() {
        let rows =
            vec![row("aaaaaaa", Some(10), None), row("bbbbbbb", None, Some(20)), row("ccccccc", Some(5), Some(6))];
        let (only_base, only_target, both) = partition(rows.into_iter());
        assert_eq!(only_base.iter().map(|c| c.commit.short_hash.as_str()).collect::<Vec<_>>(), ["aaaaaaa"]);
        assert_eq!(only_target.iter().map(|c| c.commit.short_hash.as_str()).collect::<Vec<_>>(), ["bbbbbbb"]);
        assert_eq!(both.len(), 1);
        assert_eq!(both[0].base_last_deployed_at.timestamp(), 5);
        assert_eq!(both[0].target_last_deployed_at.timestamp(), 6);
    }

    #[test]
    fn notice_never_claims_ancestry() {
        let notice = COMPARISON_NOTICE.to_lowercase();
        assert!(notice.contains("recorded"));
        assert!(notice.contains("no ordering"));
    }
}
