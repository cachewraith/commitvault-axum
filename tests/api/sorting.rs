//! Every list defaults to newest-created first and accepts the documented
//! sort options (text sorts default to A→Z).

use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use crate::common::{TestApp, commit_hash, ids};

fn field(response: &crate::common::TestResponse, name: &str) -> Vec<String> {
    response.data().as_array().unwrap().iter().map(|item| item[name].as_str().unwrap_or_default().to_owned()).collect()
}

#[sqlx::test]
async fn lists_default_to_newest_created_first(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let alpha = app.create_project("Alpha").await;
    let beta = app.create_project("Beta").await;
    // Created later, but committed earlier: proves the default is not commit date.
    let first = app
        .create_commit_with(&alpha, json!({ "branch_name": "main", "commit_hash": commit_hash('a'), "commit_message": "later date", "commit_date": "2026-03-01T10:00:00Z" }))
        .await;
    let second = app
        .create_commit_with(&alpha, json!({ "branch_name": "main", "commit_hash": commit_hash('b'), "commit_message": "earlier date", "commit_date": "2026-01-01T10:00:00Z" }))
        .await;
    let staging = app.create_environment(&alpha, "Staging").await;
    let prod = app.create_environment(&beta, "Production").await;
    let tag_a = app.create_tag("zeta").await;
    let tag_b = app.create_tag("alpha").await;

    assert_eq!(ids(&app.get("/api/v1/projects").await), [beta.clone(), alpha.clone()]);
    assert_eq!(ids(&app.get("/api/v1/commits").await), [second.clone(), first.clone()]);
    assert_eq!(ids(&app.get("/api/v1/environments").await), [prod.clone(), staging.clone()]);
    assert_eq!(ids(&app.get("/api/v1/tags").await), [tag_b.clone(), tag_a.clone()]);

    // Other options still work, with sensible default directions.
    assert_eq!(ids(&app.get("/api/v1/commits?sort=commit_date").await), [first.clone(), second.clone()]);
    assert_eq!(field(&app.get("/api/v1/commits?sort=message").await, "commit_message"), ["earlier date", "later date"]);
    assert_eq!(field(&app.get("/api/v1/tags?sort=name").await, "name"), ["alpha", "zeta"]);
    assert_eq!(ids(&app.get("/api/v1/environments?sort=name").await), [prod.clone(), staging.clone()]);
    assert_eq!(ids(&app.get("/api/v1/environments?sort=project").await), [staging.clone(), prod.clone()]);
    assert_eq!(ids(&app.get("/api/v1/projects?sort=commits").await), [alpha.clone(), beta.clone()]);
}

#[sqlx::test]
async fn deployments_releases_and_projects_sort_by_their_own_fields(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let shop = app.create_project("Shop").await;
    let blog = app.create_project("Blog").await;
    let env = app.create_environment(&shop, "Production").await;
    let commit = app.create_commit(&shop, 'a', "feat: thing").await;
    let ok = app.create_deployment(&shop, &env, &commit, "successful", "2026-03-01T10:00:00Z").await;
    let failed = app.create_deployment(&shop, &env, &commit, "failed", "2026-02-01T10:00:00Z").await;

    assert_eq!(ids(&app.get("/api/v1/deployments").await), [failed.clone(), ok.clone()], "newest record first");
    assert_eq!(ids(&app.get("/api/v1/deployments?sort=deployed_at").await), [ok.clone(), failed.clone()]);
    assert_eq!(ids(&app.get("/api/v1/deployments?sort=status").await), [failed.clone(), ok.clone()], "problems first");

    // Projects without a recorded deployment sort last.
    assert_eq!(ids(&app.get("/api/v1/projects?sort=last_deployed_at").await), [shop.clone(), blog.clone()]);
    assert_eq!(ids(&app.get("/api/v1/projects?sort=last_deployed_at&order=asc").await), [shop.clone(), blog.clone()]);

    let make = |version: &str, title: &str| json!({ "version": version, "title": title });
    let v1 = app
        .post(&format!("/api/v1/projects/{shop}/releases"), make("1.0.0", "Zebra"))
        .await
        .assert_status(StatusCode::CREATED)
        .id();
    let v2 = app
        .post(&format!("/api/v1/projects/{shop}/releases"), make("2.0.0", "Apple"))
        .await
        .assert_status(StatusCode::CREATED)
        .id();
    assert_eq!(ids(&app.get("/api/v1/releases").await), [v2.clone(), v1.clone()]);
    assert_eq!(ids(&app.get("/api/v1/releases?sort=title").await), [v2.clone(), v1.clone()]);
    assert_eq!(ids(&app.get("/api/v1/releases?sort=version").await), [v1.clone(), v2.clone()]);

    app.get("/api/v1/projects?sort=bogus").await.assert_status(StatusCode::BAD_REQUEST);
}
