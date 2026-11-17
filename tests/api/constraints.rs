//! The database enforces its own integrity, independent of the application
//! layer. These tests write to PostgreSQL directly and expect it to refuse.

use sqlx::PgPool;
use uuid::Uuid;

async fn project(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO projects (name, slug) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn environment(pool: &PgPool, project_id: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO environments (project_id, name, slug) VALUES ($1, $2, $2) RETURNING id")
        .bind(project_id)
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn commit(pool: &PgPool, project_id: Uuid, hash: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO commits (project_id, branch_name, commit_hash, short_hash, commit_message, commit_date)
         VALUES ($1, 'main', $2, left($2, 7), 'msg', now()) RETURNING id",
    )
    .bind(project_id)
    .bind(hash)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn deployment(pool: &PgPool, project_id: Uuid, environment_id: Uuid, commit_id: Uuid) -> sqlx::Result<Uuid> {
    sqlx::query_scalar(
        "INSERT INTO deployments (project_id, environment_id, commit_id, deployment_title, deployed_at)
         VALUES ($1, $2, $3, 'deploy', now()) RETURNING id",
    )
    .bind(project_id)
    .bind(environment_id)
    .bind(commit_id)
    .fetch_one(pool)
    .await
}

fn constraint(err: &sqlx::Error) -> String {
    err.as_database_error().and_then(|e| e.constraint()).unwrap_or_default().to_owned()
}

#[sqlx::test]
async fn commit_hash_is_unique_per_project(pool: PgPool) {
    let one = project(&pool, "one").await;
    let two = project(&pool, "two").await;
    commit(&pool, one, "aaaaaaa1").await;
    commit(&pool, two, "aaaaaaa1").await;

    let err = sqlx::query("INSERT INTO commits (project_id, branch_name, commit_hash, short_hash, commit_message, commit_date) VALUES ($1, 'dev', 'aaaaaaa1', 'aaaaaaa', 'again', now())")
        .bind(one)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(constraint(&err), "commits_project_hash_key");
}

#[sqlx::test]
async fn deployments_cannot_cross_projects(pool: PgPool) {
    let one = project(&pool, "one").await;
    let two = project(&pool, "two").await;
    let env_one = environment(&pool, one, "prod").await;
    let env_two = environment(&pool, two, "prod").await;
    let commit_one = commit(&pool, one, "aaaaaaa1").await;
    let commit_two = commit(&pool, two, "bbbbbbb1").await;

    deployment(&pool, one, env_one, commit_one).await.expect("same project is fine");

    let foreign_env = deployment(&pool, one, env_two, commit_one).await.unwrap_err();
    assert_eq!(constraint(&foreign_env), "deployments_environment_fkey");
    let foreign_commit = deployment(&pool, one, env_one, commit_two).await.unwrap_err();
    assert_eq!(constraint(&foreign_commit), "deployments_commit_fkey");
}

#[sqlx::test]
async fn deployment_history_blocks_deleting_its_commit_or_environment(pool: PgPool) {
    let one = project(&pool, "one").await;
    let env = environment(&pool, one, "prod").await;
    let c = commit(&pool, one, "aaaaaaa1").await;
    deployment(&pool, one, env, c).await.unwrap();

    let err = sqlx::query("DELETE FROM commits WHERE id = $1").bind(c).execute(&pool).await.unwrap_err();
    assert_eq!(constraint(&err), "deployments_commit_fkey");
    let err = sqlx::query("DELETE FROM environments WHERE id = $1").bind(env).execute(&pool).await.unwrap_err();
    assert_eq!(constraint(&err), "deployments_environment_fkey");

    // …while deleting the whole project still cascades cleanly.
    sqlx::query("DELETE FROM projects WHERE id = $1").bind(one).execute(&pool).await.unwrap();
    let left: i64 = sqlx::query_scalar("SELECT (SELECT count(*) FROM deployments) + (SELECT count(*) FROM commits) + (SELECT count(*) FROM environments)")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[sqlx::test]
async fn release_commits_cannot_cross_projects(pool: PgPool) {
    let one = project(&pool, "one").await;
    let two = project(&pool, "two").await;
    let foreign = commit(&pool, two, "bbbbbbb1").await;
    let release: Uuid =
        sqlx::query_scalar("INSERT INTO releases (project_id, version, title) VALUES ($1, '1.0', 'r') RETURNING id")
            .bind(one)
            .fetch_one(&pool)
            .await
            .unwrap();

    // Claiming the release's project for a foreign commit…
    let err = sqlx::query("INSERT INTO release_commits (release_id, commit_id, project_id) VALUES ($1, $2, $3)")
        .bind(release)
        .bind(foreign)
        .bind(one)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(constraint(&err), "release_commits_commit_fkey");
    // …or the commit's project for the release both fail.
    let err = sqlx::query("INSERT INTO release_commits (release_id, commit_id, project_id) VALUES ($1, $2, $3)")
        .bind(release)
        .bind(foreign)
        .bind(two)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(constraint(&err), "release_commits_release_fkey");
}

#[sqlx::test]
async fn check_constraints_reject_invalid_values(pool: PgPool) {
    let one = project(&pool, "one").await;
    let env = environment(&pool, one, "prod").await;
    let c = commit(&pool, one, "aaaaaaa1").await;

    let cases: [(&str, &str); 7] = [
        ("INSERT INTO projects (name, slug) VALUES ('x', 'Not A Slug')", "projects_slug_check"),
        ("INSERT INTO projects (name, slug, status) VALUES ('x', 'x', 'deleted')", "projects_status_check"),
        ("INSERT INTO projects (name, slug, status) VALUES ('x', 'x', 'archived')", "projects_archived_consistency"),
        ("INSERT INTO tags (name, slug, color) VALUES ('x', 'x', 'red')", "tags_color_check"),
        ("INSERT INTO users (email, password_hash) VALUES ('Mixed@Case.com', 'x')", "users_email_check"),
        ("INSERT INTO notes (title, content_json) VALUES ('', '{}')", "notes_title_check"),
        (
            "INSERT INTO releases (project_id, version, title, status) SELECT id, '1', 't', 'shipped' FROM projects LIMIT 1",
            "releases_status_check",
        ),
    ];
    for (sql, expected) in cases {
        let err = sqlx::query(sqlx::AssertSqlSafe(sql.to_owned())).execute(&pool).await.unwrap_err();
        assert_eq!(constraint(&err), expected, "{sql}");
    }

    let err = sqlx::query("INSERT INTO commits (project_id, branch_name, commit_hash, short_hash, commit_message, commit_date) VALUES ($1, 'main', 'NOT-HEX', 'aaaaaaa', 'm', now())")
        .bind(one)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(constraint(&err), "commits_commit_hash_check");

    let err = sqlx::query("INSERT INTO deployments (project_id, environment_id, commit_id, deployment_title, deployed_at, status) VALUES ($1, $2, $3, 't', now(), 'done')")
        .bind(one)
        .bind(env)
        .bind(c)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(constraint(&err), "deployments_status_check");
}

#[sqlx::test]
async fn unique_constraints_on_slugs_versions_and_emails(pool: PgPool) {
    let one = project(&pool, "one").await;
    environment(&pool, one, "prod").await;

    let err = sqlx::query("INSERT INTO projects (name, slug) VALUES ('dup', 'one')").execute(&pool).await.unwrap_err();
    assert_eq!(constraint(&err), "projects_slug_key");
    let err = sqlx::query("INSERT INTO environments (project_id, name, slug) VALUES ($1, 'Prod', 'prod')")
        .bind(one)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(constraint(&err), "environments_project_slug_key");

    sqlx::query("INSERT INTO releases (project_id, version, title) VALUES ($1, '1.0', 'a')")
        .bind(one)
        .execute(&pool)
        .await
        .unwrap();
    let err = sqlx::query("INSERT INTO releases (project_id, version, title) VALUES ($1, '1.0', 'b')")
        .bind(one)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(constraint(&err), "releases_project_version_key");

    sqlx::query("INSERT INTO users (email, password_hash) VALUES ('a@b.c', 'x')").execute(&pool).await.unwrap();
    let err =
        sqlx::query("INSERT INTO users (email, password_hash) VALUES ('a@b.c', 'y')").execute(&pool).await.unwrap_err();
    assert_eq!(constraint(&err), "users_email_key");
}

#[sqlx::test]
async fn updated_at_is_maintained_by_the_database(pool: PgPool) {
    let id = project(&pool, "one").await;
    let (created, before): (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) =
        sqlx::query_as("SELECT created_at, updated_at FROM projects WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(created, before);

    tokio::time::sleep(std::time::Duration::from_millis(15)).await;
    sqlx::query("UPDATE projects SET name = 'renamed' WHERE id = $1").bind(id).execute(&pool).await.unwrap();
    let after: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT updated_at FROM projects WHERE id = $1").bind(id).fetch_one(&pool).await.unwrap();
    assert!(after > before);
}

#[sqlx::test]
async fn expected_indexes_exist(pool: PgPool) {
    let indexes: Vec<String> = sqlx::query_scalar("SELECT indexname FROM pg_indexes WHERE schemaname = 'public'")
        .fetch_all(&pool)
        .await
        .unwrap();
    for expected in [
        "projects_slug_key",
        "commits_project_hash_key",
        "commits_project_branch_idx",
        "commits_project_date_idx",
        "deployments_project_env_date_idx",
        "deployments_commit_idx",
        "notes_project_updated_idx",
        "releases_project_released_idx",
        "sessions_expires_at_idx",
    ] {
        assert!(indexes.iter().any(|name| name == expected), "missing index {expected}");
    }
}
