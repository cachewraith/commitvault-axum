use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use crate::common::{TestApp, ids};

#[sqlx::test]
async fn deployment_crud_lifecycle(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let staging = app.create_environment(&project, "Staging").await;
    let prod = app.create_environment(&project, "Production").await;
    let commit = app.create_commit(&project, 'a', "feat: checkout").await;

    let created = app
        .post(
            &format!("/api/v1/projects/{project}/deployments"),
            json!({
                "environment_id": staging,
                "commit_id": commit,
                "deployment_title": "Checkout to staging",
                "deployment_notes": "Run: sqlx migrate run\nThen restart the worker.",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);
    let deployment = created.data();
    assert_eq!(deployment["status"], "successful", "default status");
    assert!(deployment["deployed_at"].is_string(), "defaults to now");
    assert_eq!(deployment["environment"]["name"], "Staging");
    assert_eq!(deployment["commit"]["commit_message"], "feat: checkout");
    assert_eq!(deployment["project"]["name"], "Shop");
    assert_eq!(deployment["deployment_notes"], "Run: sqlx migrate run\nThen restart the worker.");
    let id = created.id();

    let updated = app
        .patch(
            &format!("/api/v1/deployments/{id}"),
            json!({ "status": "rolled_back", "environment_id": prod, "deployed_at": "2026-03-05T08:30:00Z", "deployment_notes": null }),
        )
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(updated.data()["status"], "rolled_back");
    assert_eq!(updated.data()["environment"]["name"], "Production");
    assert_eq!(updated.data()["deployed_at"], "2026-03-05T08:30:00Z");
    assert_eq!(updated.data()["deployment_notes"], json!(null));

    app.patch(&format!("/api/v1/deployments/{id}"), json!({ "status": "exploded" }))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    app.delete(&format!("/api/v1/deployments/{id}")).await.assert_status(StatusCode::NO_CONTENT);
    app.get(&format!("/api/v1/deployments/{id}")).await.assert_status(StatusCode::NOT_FOUND);
    // Deleting the record leaves the commit and the environment alone.
    app.get(&format!("/api/v1/commits/{commit}")).await.assert_status(StatusCode::OK);
    app.get(&format!("/api/v1/environments/{prod}")).await.assert_status(StatusCode::OK);
}

#[sqlx::test]
async fn one_commit_can_be_deployed_many_times_and_to_many_environments(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let staging = app.create_environment(&project, "Staging").await;
    let prod = app.create_environment(&project, "Production").await;
    let commit = app.create_commit(&project, 'a', "feat: checkout").await;

    let first = app.create_deployment(&project, &staging, &commit, "failed", "2026-03-02T10:00:00Z").await;
    let second = app.create_deployment(&project, &staging, &commit, "successful", "2026-03-02T11:00:00Z").await;
    let third = app.create_deployment(&project, &prod, &commit, "successful", "2026-03-03T09:00:00Z").await;

    let history = app.get(&format!("/api/v1/deployments?commit_id={commit}")).await.assert_status(StatusCode::OK);
    assert_eq!(ids(&history), [third.clone(), second.clone(), first.clone()], "newest first");

    let staging_only = app.get(&format!("/api/v1/deployments?commit_id={commit}&environment_id={staging}")).await;
    assert_eq!(ids(&staging_only), [second, first]);
}

#[sqlx::test]
async fn deployment_must_reference_an_environment_and_commit_of_its_own_project(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let shop = app.create_project("Shop").await;
    let blog = app.create_project("Blog").await;
    let shop_env = app.create_environment(&shop, "Production").await;
    let shop_commit = app.create_commit(&shop, 'a', "shop commit").await;
    let blog_env = app.create_environment(&blog, "Production").await;
    let blog_commit = app.create_commit(&blog, 'b', "blog commit").await;
    let path = format!("/api/v1/projects/{shop}/deployments");

    let foreign_env = app
        .post(&path, json!({ "environment_id": blog_env, "commit_id": shop_commit, "deployment_title": "x" }))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(foreign_env.body["error"]["details"]["environment_id"].is_array());
    assert!(foreign_env.body["error"]["details"].get("commit_id").is_none());

    let foreign_commit = app
        .post(&path, json!({ "environment_id": shop_env, "commit_id": blog_commit, "deployment_title": "x" }))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(foreign_commit.body["error"]["details"]["commit_id"].is_array());

    let unknown = app
        .post(&path, json!({ "environment_id": uuid::Uuid::now_v7(), "commit_id": uuid::Uuid::now_v7(), "deployment_title": "x" }))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(unknown.body["error"]["details"]["environment_id"].is_array());
    assert!(unknown.body["error"]["details"]["commit_id"].is_array());

    // Moving an existing record to another project's commit is refused too.
    let id = app.create_deployment(&shop, &shop_env, &shop_commit, "successful", "2026-03-02T10:00:00Z").await;
    app.patch(&format!("/api/v1/deployments/{id}"), json!({ "commit_id": blog_commit }))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM deployments").fetch_one(&app.pool).await.unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test]
async fn deployment_history_filters(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let other = app.create_project("Blog").await;
    let staging = app.create_environment(&project, "Staging").await;
    let prod = app.create_environment(&project, "Production").await;
    let other_env = app.create_environment(&other, "Production").await;
    let a = app.create_commit(&project, 'a', "feat: login").await;
    let b = app.create_commit(&project, 'b', "feat: cart").await;
    let c = app.create_commit(&other, 'c', "feat: post").await;

    let d1 = app.create_deployment(&project, &staging, &a, "successful", "2026-01-10T10:00:00Z").await;
    let d2 = app.create_deployment(&project, &prod, &a, "failed", "2026-02-10T10:00:00Z").await;
    let d3 = app.create_deployment(&project, &prod, &b, "successful", "2026-03-10T10:00:00Z").await;
    let d4 = app.create_deployment(&other, &other_env, &c, "pending", "2026-03-11T10:00:00Z").await;

    assert_eq!(ids(&app.get("/api/v1/deployments").await), [d4.clone(), d3.clone(), d2.clone(), d1.clone()]);
    assert_eq!(
        ids(&app.get(&format!("/api/v1/projects/{project}/deployments")).await),
        [d3.clone(), d2.clone(), d1.clone()]
    );
    assert_eq!(ids(&app.get(&format!("/api/v1/deployments?environment_id={prod}")).await), [d3.clone(), d2.clone()]);
    assert_eq!(ids(&app.get("/api/v1/deployments?status=failed").await), [d2.as_str()]);
    assert_eq!(ids(&app.get("/api/v1/deployments?status=pending").await), [d4.as_str()]);
    let ranged = app.get("/api/v1/deployments?date_from=2026-02-01T00:00:00Z&date_to=2026-03-10T10:00:00Z").await;
    assert_eq!(ids(&ranged), [d3.clone(), d2.clone()]);
    // Search reaches through to the commit message.
    assert_eq!(ids(&app.get("/api/v1/deployments?search=cart").await), [d3.as_str()]);
    let oldest = app.get("/api/v1/deployments?order=asc&per_page=1").await;
    assert_eq!(ids(&oldest), [d1]);
    assert_eq!(oldest.body["meta"]["total"], 4);
}

#[sqlx::test]
async fn environments_report_their_latest_recorded_deployment(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let prod = app.create_environment(&project, "Production").await;
    let idle = app.create_environment(&project, "Staging").await;
    let old = app.create_commit(&project, 'a', "old").await;
    let new = app.create_commit(&project, 'b', "new").await;

    app.create_deployment(&project, &prod, &old, "successful", "2026-03-01T10:00:00Z").await;
    let latest_ok = app.create_deployment(&project, &prod, &new, "successful", "2026-03-02T10:00:00Z").await;
    let latest_any = app.create_deployment(&project, &prod, &old, "failed", "2026-03-03T10:00:00Z").await;

    let env = app.get(&format!("/api/v1/environments/{prod}")).await.assert_status(StatusCode::OK);
    assert_eq!(env.data()["deployment_count"], 3);
    assert_eq!(env.data()["latest_deployment"]["id"], latest_any.as_str());
    assert_eq!(env.data()["latest_deployment"]["status"], "failed");
    assert_eq!(env.data()["latest_successful_deployment"]["id"], latest_ok.as_str());
    assert_eq!(env.data()["latest_successful_deployment"]["commit"]["commit_message"], "new");

    // No records → no claim about what is deployed.
    let empty = app.get(&format!("/api/v1/environments/{idle}")).await;
    assert_eq!(empty.data()["latest_deployment"], json!(null));
    assert_eq!(empty.data()["latest_successful_deployment"], json!(null));

    let overview = app.get(&format!("/api/v1/environments?project_id={project}")).await.assert_status(StatusCode::OK);
    assert_eq!(overview.body["meta"]["total"], 2);
}

#[sqlx::test]
async fn compare_lists_recorded_commits_per_environment(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let staging = app.create_environment(&project, "Staging").await;
    let prod = app.create_environment(&project, "Production").await;
    let shared = app.create_commit(&project, 'a', "in both").await;
    let staging_only = app.create_commit(&project, 'b', "staging only").await;
    let prod_failed = app.create_commit(&project, 'c', "failed in production").await;
    app.create_commit(&project, 'd', "never recorded").await;

    app.create_deployment(&project, &staging, &shared, "successful", "2026-03-01T10:00:00Z").await;
    app.create_deployment(&project, &prod, &shared, "successful", "2026-03-02T10:00:00Z").await;
    app.create_deployment(&project, &staging, &staging_only, "successful", "2026-03-03T10:00:00Z").await;
    app.create_deployment(&project, &prod, &prod_failed, "failed", "2026-03-04T10:00:00Z").await;

    let path = format!("/api/v1/projects/{project}/deployments/compare?base={staging}&target={prod}");
    let comparison = app.get(&path).await.assert_status(StatusCode::OK);
    let data = comparison.data();
    let commit_ids = |key: &str| -> Vec<String> {
        data[key].as_array().unwrap().iter().map(|c| c["commit"]["id"].as_str().unwrap().to_owned()).collect()
    };

    assert_eq!(data["statuses_considered"], json!(["successful"]));
    assert_eq!(commit_ids("only_in_base"), [staging_only.as_str()]);
    assert!(commit_ids("only_in_target").is_empty(), "failed deployments are not counted by default");
    assert_eq!(commit_ids("in_both"), [shared.as_str()]);
    assert_eq!(data["counts"], json!({ "only_in_base": 1, "only_in_target": 0, "in_both": 1 }));
    assert_eq!(data["base"]["environment"]["name"], "Staging");
    assert_eq!(data["base"]["recorded_commit_count"], 2);
    assert_eq!(data["base"]["latest_deployment"]["commit"]["id"], staging_only.as_str());
    assert_eq!(data["target"]["latest_deployment"]["commit"]["id"], shared.as_str());
    assert_eq!(data["truncated"], false);

    // The API never claims ancestry or absence.
    let notice = data["notice"].as_str().unwrap().to_lowercase();
    assert!(notice.contains("recorded") && notice.contains("may still be deployed"));
    let text = comparison.body.to_string().to_lowercase();
    for claim in ["\"ahead\"", "\"behind\"", "missing_from", "not_deployed"] {
        assert!(!text.contains(claim), "comparison must not contain {claim}");
    }

    let all = app.get(&format!("{path}&include_unsuccessful=true")).await.assert_status(StatusCode::OK);
    assert_eq!(all.data()["counts"], json!({ "only_in_base": 1, "only_in_target": 1, "in_both": 1 }));
    assert_eq!(all.data()["statuses_considered"].as_array().unwrap().len(), 4);
    assert_eq!(all.data()["target"]["latest_deployment"]["status"], "failed");
}

#[sqlx::test]
async fn compare_validates_its_environments(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let other = app.create_project("Blog").await;
    let env = app.create_environment(&project, "Staging").await;
    let foreign = app.create_environment(&other, "Production").await;
    let base = format!("/api/v1/projects/{project}/deployments/compare");

    app.get(&format!("{base}?base={env}&target={env}")).await.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    let cross =
        app.get(&format!("{base}?base={env}&target={foreign}")).await.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(cross.body["error"]["details"]["target"].is_array());
    app.get(&format!("{base}?base={env}")).await.assert_status(StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn deleting_an_environment_with_history_requires_cascade(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let env = app.create_environment(&project, "Production").await;
    let commit = app.create_commit(&project, 'a', "feat: thing").await;
    let deployment = app.create_deployment(&project, &env, &commit, "successful", "2026-03-02T10:00:00Z").await;

    app.delete(&format!("/api/v1/environments/{env}")).await.assert_status(StatusCode::CONFLICT);
    app.get(&format!("/api/v1/deployments/{deployment}")).await.assert_status(StatusCode::OK);

    app.delete(&format!("/api/v1/environments/{env}?cascade=true")).await.assert_status(StatusCode::NO_CONTENT);
    app.get(&format!("/api/v1/deployments/{deployment}")).await.assert_status(StatusCode::NOT_FOUND);
    app.get(&format!("/api/v1/commits/{commit}")).await.assert_status(StatusCode::OK);

    // An environment without history deletes without the flag.
    let empty = app.create_environment(&project, "Staging").await;
    app.delete(&format!("/api/v1/environments/{empty}")).await.assert_status(StatusCode::NO_CONTENT);
}
