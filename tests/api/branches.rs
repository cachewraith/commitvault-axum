//! Branches as records: listing with commit statistics, rename, deletion
//! rules and where a branch's commits have recorded deployments.

use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use crate::common::{TestApp, commit_hash, ids};

fn names(response: &crate::common::TestResponse) -> Vec<String> {
    response.data().as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap().to_owned()).collect()
}

#[sqlx::test]
async fn a_new_project_starts_with_its_default_branch(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app
        .post("/api/v1/projects", json!({ "name": "Shop", "default_branch": "trunk" }))
        .await
        .assert_status(StatusCode::CREATED)
        .id();

    let list = app.get(&format!("/api/v1/projects/{project}/branches")).await.assert_status(StatusCode::OK);
    assert_eq!(names(&list), ["trunk"]);
    let trunk = &list.data()[0];
    assert_eq!(trunk["is_default"], true);
    assert_eq!(trunk["commit_count"], 0);
    assert_eq!(trunk["last_commit_at"], json!(null));
    assert_eq!(trunk["project"]["name"], "Shop");

    // Changing the default branch makes sure that branch exists too.
    app.patch(&format!("/api/v1/projects/{project}"), json!({ "default_branch": "main" }))
        .await
        .assert_status(StatusCode::OK);
    let list = app.get(&format!("/api/v1/projects/{project}/branches?sort=name")).await;
    assert_eq!(names(&list), ["main", "trunk"]);
    assert_eq!(list.data()[0]["is_default"], true);
    assert_eq!(list.data()[1]["is_default"], false);

    app.get(&format!("/api/v1/projects/{}/branches", uuid::Uuid::now_v7())).await.assert_status(StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn branch_crud_and_validation(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let path = format!("/api/v1/projects/{project}/branches");

    let created = app
        .post(&path, json!({ "name": " feature/login ", "description": "Login form" }))
        .await
        .assert_status(StatusCode::CREATED);
    assert_eq!(created.data()["name"], "feature/login", "names are trimmed");
    assert_eq!(created.data()["description"], "Login form");
    let id = created.id();

    let duplicate = app.post(&path, json!({ "name": "feature/login" })).await.assert_status(StatusCode::CONFLICT);
    assert!(duplicate.body["error"]["details"]["name"].is_array());
    // The same name in another project is fine.
    let other = app.create_project("Blog").await;
    app.create_branch(&other, "feature/login").await;

    for bad in ["has space", "a..b", "/leading", "x".repeat(256).as_str()] {
        let invalid = app.post(&path, json!({ "name": bad })).await.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        assert!(invalid.body["error"]["details"]["name"].is_array(), "{bad}");
    }

    let updated = app
        .patch(&format!("/api/v1/branches/{id}"), json!({ "description": null }))
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(updated.data()["description"], json!(null));
    assert_eq!(updated.data()["name"], "feature/login");

    app.patch(&format!("/api/v1/branches/{id}"), json!({ "name": "main" })).await.assert_status(StatusCode::CONFLICT);

    app.delete(&format!("/api/v1/branches/{id}")).await.assert_status(StatusCode::NO_CONTENT);
    app.get(&format!("/api/v1/branches/{id}")).await.assert_status(StatusCode::NOT_FOUND);
    app.delete(&format!("/api/v1/branches/{id}")).await.assert_status(StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn list_counts_commits_sorts_and_searches(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let commit = |seed: char, branch: &str, date: &str| json!({ "branch_name": branch, "commit_hash": commit_hash(seed), "commit_message": "m", "commit_date": date });
    app.create_commit_with(&project, commit('a', "feature/login", "2026-01-10T00:00:00Z")).await;
    app.create_commit_with(&project, commit('b', "feature/login", "2026-01-12T00:00:00Z")).await;
    app.create_commit_with(&project, commit('c', "feature/cart", "2026-02-10T00:00:00Z")).await;
    app.create_branch(&project, "release/1.0").await;

    let path = format!("/api/v1/projects/{project}/branches");
    // Default: newest branch first.
    let list = app.get(&path).await.assert_status(StatusCode::OK);
    assert_eq!(names(&list), ["release/1.0", "feature/cart", "feature/login", "main"]);
    // Latest commit first, branches without commits last.
    let list = app.get(&format!("{path}?sort=last_commit")).await.assert_status(StatusCode::OK);
    assert_eq!(names(&list), ["feature/cart", "feature/login", "main", "release/1.0"]);
    assert_eq!(names(&app.get(&format!("{path}?sort=commits")).await)[0], "feature/login");
    assert_eq!(list.data()[1]["commit_count"], 2);
    assert_eq!(list.data()[1]["last_commit_at"], "2026-01-12T00:00:00Z");
    assert_eq!(list.body["meta"]["total"], 4);

    assert_eq!(
        names(&app.get(&format!("{path}?sort=name")).await),
        ["feature/cart", "feature/login", "main", "release/1.0"]
    );
    assert_eq!(names(&app.get(&format!("{path}?sort=name&order=desc&per_page=1")).await), ["release/1.0"]);
    assert_eq!(names(&app.get(&format!("{path}?search=FEATURE")).await), ["feature/cart", "feature/login"]);
    assert_eq!(names(&app.get(&format!("{path}?search=%25")).await), Vec::<String>::new(), "wildcards are literal");
}

#[sqlx::test]
async fn renaming_moves_commits_and_the_default_branch(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let commit = app.create_commit(&project, 'a', "feat: thing").await;
    let main = app.branch_id(&project, "main").await;

    let renamed =
        app.patch(&format!("/api/v1/branches/{main}"), json!({ "name": "trunk" })).await.assert_status(StatusCode::OK);
    assert_eq!(renamed.data()["is_default"], true);

    let fetched = app.get(&format!("/api/v1/commits/{commit}")).await;
    assert_eq!(fetched.data()["branch"]["name"], "trunk");
    let project_view = app.get(&format!("/api/v1/projects/{project}")).await;
    assert_eq!(project_view.data()["default_branch"], "trunk");
    // Search follows the new name.
    assert_eq!(ids(&app.get("/api/v1/commits?search=trunk").await), [commit.as_str()]);
}

#[sqlx::test]
async fn detail_shows_where_the_branch_has_recorded_deployments(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let staging = app.create_environment(&project, "Staging").await;
    let prod = app.create_environment(&project, "Production").await;
    app.create_environment(&project, "Preview").await;
    let older = app.create_commit(&project, 'a', "feat: one").await;
    let newer = app.create_commit(&project, 'b', "feat: two").await;
    let elsewhere = app
        .create_commit_with(
            &project,
            json!({ "branch_name": "feature/x", "commit_hash": commit_hash('c'), "commit_message": "x", "commit_date": "2026-03-01T10:00:00Z" }),
        )
        .await;

    app.create_deployment(&project, &staging, &older, "successful", "2026-03-02T10:00:00Z").await;
    app.create_deployment(&project, &staging, &newer, "failed", "2026-03-03T10:00:00Z").await;
    app.create_deployment(&project, &prod, &older, "successful", "2026-03-04T10:00:00Z").await;
    // A later record from another branch does not count for main.
    app.create_deployment(&project, &prod, &elsewhere, "successful", "2026-03-05T10:00:00Z").await;

    let main = app.branch_id(&project, "main").await;
    let detail = app.get(&format!("/api/v1/branches/{main}")).await.assert_status(StatusCode::OK);
    assert_eq!(detail.data()["name"], "main");
    assert_eq!(detail.data()["commit_count"], 2);
    let environments = detail.data()["environments"].as_array().unwrap();
    assert_eq!(environments.len(), 2, "Preview has no record and is not listed: {environments:?}");
    assert_eq!(environments[0]["name"], "Staging");
    assert_eq!(environments[0]["deployment_count"], 2);
    assert_eq!(environments[0]["latest_deployment"]["status"], "failed");
    assert_eq!(environments[0]["latest_deployment"]["commit"]["id"], newer.as_str());
    assert_eq!(environments[0]["latest_deployment"]["commit"]["branch_name"], "main");
    assert_eq!(environments[1]["name"], "Production");
    assert_eq!(environments[1]["deployment_count"], 1);
    assert_eq!(environments[1]["latest_deployment"]["commit"]["id"], older.as_str());

    // Commits of a branch.
    let commits = app.get(&format!("/api/v1/commits?branch_id={main}")).await;
    assert_eq!(commits.body["meta"]["total"], 2);
}

#[sqlx::test]
async fn deleting_a_branch_with_commits_requires_cascade(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let env = app.create_environment(&project, "Production").await;
    let commit = app
        .create_commit_with(
            &project,
            json!({ "branch_name": "feature/x", "commit_hash": commit_hash('a'), "commit_message": "x", "commit_date": "2026-03-01T10:00:00Z" }),
        )
        .await;
    let kept = app.create_commit(&project, 'b', "on main").await;
    let deployment = app.create_deployment(&project, &env, &commit, "successful", "2026-03-02T10:00:00Z").await;
    let branch = app.branch_id(&project, "feature/x").await;

    let blocked = app.delete(&format!("/api/v1/branches/{branch}")).await.assert_status(StatusCode::CONFLICT);
    assert!(blocked.body["error"]["message"].as_str().unwrap().contains("recorded commits"));
    app.get(&format!("/api/v1/commits/{commit}")).await.assert_status(StatusCode::OK);

    app.delete(&format!("/api/v1/branches/{branch}?cascade=true")).await.assert_status(StatusCode::NO_CONTENT);
    app.get(&format!("/api/v1/branches/{branch}")).await.assert_status(StatusCode::NOT_FOUND);
    app.get(&format!("/api/v1/commits/{commit}")).await.assert_status(StatusCode::NOT_FOUND);
    app.get(&format!("/api/v1/deployments/{deployment}")).await.assert_status(StatusCode::NOT_FOUND);
    // Other branches and the environment are untouched.
    app.get(&format!("/api/v1/commits/{kept}")).await.assert_status(StatusCode::OK);
    app.get(&format!("/api/v1/environments/{env}")).await.assert_status(StatusCode::OK);
}
