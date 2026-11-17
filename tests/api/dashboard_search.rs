use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use crate::common::TestApp;

#[sqlx::test]
async fn dashboard_is_empty_but_well_formed_on_a_new_install(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let dashboard = app.get("/api/v1/dashboard").await.assert_status(StatusCode::OK);
    let data = dashboard.data();
    assert_eq!(
        data["totals"],
        json!({ "projects": 0, "active_projects": 0, "commits": 0, "environments": 0, "deployments": 0, "releases": 0, "notes": 0, "tags": 0 })
    );
    for list in ["recent_deployments", "recent_commits", "recent_notes", "project_activity", "environment_overview"] {
        assert_eq!(data[list], json!([]), "{list}");
    }
    let activity = data["deployment_activity"].as_array().unwrap();
    assert_eq!(activity.len(), 30, "one bucket per day, including empty days");
    assert!(activity.iter().all(|day| day["successful"] == 0 && day["failed"] == 0));
}

#[sqlx::test]
async fn dashboard_reflects_real_records(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let shop = app.create_project("Shop").await;
    let archived = app.create_project("Legacy").await;
    app.patch(&format!("/api/v1/projects/{archived}"), json!({ "status": "archived" })).await;
    let staging = app.create_environment(&shop, "Staging").await;
    let prod = app.create_environment(&shop, "Production").await;
    app.create_environment(&archived, "Production").await;
    let old = app.create_commit(&shop, 'a', "feat: login").await;
    let new = app.create_commit_with(&shop, json!({ "branch_name": "main", "commit_hash": crate::common::commit_hash('b'), "commit_message": "feat: cart", "commit_date": "2026-04-01T10:00:00Z" })).await;

    let now = chrono::Utc::now();
    let today = now.to_rfc3339();
    let yesterday = (now - chrono::Duration::days(1)).to_rfc3339();
    app.create_deployment(&shop, &staging, &old, "successful", &yesterday).await;
    app.create_deployment(&shop, &staging, &new, "failed", &today).await;
    let latest = app.create_deployment(&shop, &prod, &new, "successful", &today).await;
    app.post("/api/v1/notes", json!({ "title": "Runbook" })).await.assert_status(StatusCode::CREATED);
    app.post(&format!("/api/v1/projects/{shop}/releases"), json!({ "version": "1.0.0", "title": "First" }))
        .await
        .assert_status(StatusCode::CREATED);
    app.create_tag("urgent").await;

    let dashboard = app.get("/api/v1/dashboard").await.assert_status(StatusCode::OK);
    let data = dashboard.data();
    assert_eq!(
        data["totals"],
        json!({ "projects": 2, "active_projects": 1, "commits": 2, "environments": 3, "deployments": 3, "releases": 1, "notes": 1, "tags": 1 })
    );
    assert_eq!(data["recent_deployments"].as_array().unwrap().len(), 3);
    assert_eq!(data["recent_deployments"][0]["id"], latest.as_str());
    assert_eq!(data["recent_commits"][0]["commit_message"], "feat: cart", "newest commit first");
    assert_eq!(data["recent_notes"][0]["title"], "Runbook");

    let activity = data["project_activity"].as_array().unwrap();
    assert_eq!(activity.len(), 1, "archived projects are left out");
    assert_eq!(activity[0]["project"]["name"], "Shop");
    assert_eq!(activity[0]["commit_count"], 2);
    assert_eq!(activity[0]["deployment_count"], 3);

    let environments = data["environment_overview"].as_array().unwrap();
    assert_eq!(environments.len(), 2, "environments of archived projects are left out");
    assert!(environments.iter().all(|env| env["latest_deployment"].is_object()));

    let days = data["deployment_activity"].as_array().unwrap();
    let total: i64 = days.iter().map(|d| d["successful"].as_i64().unwrap() + d["failed"].as_i64().unwrap()).sum();
    assert_eq!(total, 3);
    assert_eq!(days.last().unwrap()["date"], now.date_naive().to_string());
}

#[sqlx::test]
async fn search_finds_every_record_type(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let shop = app.create_project("Checkout Service").await;
    let blog = app.create_project("Blog").await;
    let env = app.create_environment(&shop, "Production").await;
    let commit = app.create_commit(&shop, 'a', "feat: checkout flow").await;
    app.create_commit(&blog, 'b', "feat: checkout widget for blog").await;
    app.post(
        &format!("/api/v1/projects/{shop}/deployments"),
        json!({ "environment_id": env, "commit_id": commit, "deployment_title": "Ship checkout" }),
    )
    .await;
    app.post(&format!("/api/v1/projects/{shop}/releases"), json!({ "version": "2.0.0", "title": "Checkout rewrite" }))
        .await;
    app.post(
        "/api/v1/notes",
        json!({ "title": "Payments", "content_json": { "type": "doc", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "The checkout calls the gateway twice." }] }] } }),
    )
    .await;
    app.create_tag("checkout").await;

    let results = app.get("/api/v1/search?q=CHECKOUT").await.assert_status(StatusCode::OK);
    let data = results.data();
    assert_eq!(data["query"], "CHECKOUT");
    assert_eq!(data["projects"][0]["title"], "Checkout Service");
    assert_eq!(data["projects"][0]["kind"], "project");
    assert_eq!(data["commits"].as_array().unwrap().len(), 2);
    assert!(data["commits"][0]["project"]["name"].is_string());
    assert_eq!(data["deployments"][0]["title"], "Ship checkout");
    assert_eq!(data["deployments"][0]["subtitle"], "Production · successful");
    assert_eq!(data["releases"][0]["title"], "2.0.0 — Checkout rewrite");
    assert_eq!(data["notes"][0]["title"], "Payments", "matched through the note body");
    assert_eq!(data["notes"][0]["project"], json!(null));
    assert_eq!(data["tags"][0]["title"], "checkout");
    assert_eq!(data["total"], 7);

    // Scoped to a project.
    let scoped = app.get(&format!("/api/v1/search?q=checkout&project_id={blog}")).await;
    assert_eq!(scoped.data()["commits"].as_array().unwrap().len(), 1);
    assert_eq!(scoped.data()["projects"], json!([]));
    assert_eq!(scoped.data()["deployments"], json!([]));

    // Per-group limit.
    let limited = app.get("/api/v1/search?q=checkout&limit=1").await;
    assert_eq!(limited.data()["commits"].as_array().unwrap().len(), 1);

    // Hash prefixes find commits.
    let by_hash = app.get(&format!("/api/v1/search?q={}", &crate::common::commit_hash('a')[..8])).await;
    assert_eq!(by_hash.data()["commits"].as_array().unwrap().len(), 1);
    assert_eq!(by_hash.data()["commits"][0]["id"], commit.as_str());
}

#[sqlx::test]
async fn search_validates_the_term_and_treats_wildcards_literally(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    app.create_project("Shop").await;

    app.get("/api/v1/search?q=a").await.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    app.get("/api/v1/search?q=%20%20").await.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    app.get("/api/v1/search").await.assert_status(StatusCode::BAD_REQUEST);
    app.get(&format!("/api/v1/search?q={}", "x".repeat(101))).await.assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    let wildcard = app.get("/api/v1/search?q=%25%25").await.assert_status(StatusCode::OK);
    assert_eq!(wildcard.data()["total"], 0, "`%%` is not a match-everything pattern");
    let underscore = app.get("/api/v1/search?q=s__p").await.assert_status(StatusCode::OK);
    assert_eq!(underscore.data()["total"], 0);
}
