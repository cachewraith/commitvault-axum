use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use crate::common::{TestApp, ids};

#[sqlx::test]
async fn project_crud_lifecycle(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;

    let created = app
        .post(
            "/api/v1/projects",
            json!({
                "name": "  Billing API  ",
                "description": "Invoices and payments",
                "repository_url": "https://github.com/acme/billing",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);
    let project = created.data();
    assert_eq!(project["name"], "Billing API", "input is trimmed");
    assert_eq!(project["slug"], "billing-api");
    assert_eq!(project["default_branch"], "main");
    assert_eq!(project["status"], "active");
    assert_eq!(project["archived_at"], json!(null));
    assert_eq!(project["stats"]["commit_count"], 0);
    let id = created.id();

    let fetched = app.get(&format!("/api/v1/projects/{id}")).await.assert_status(StatusCode::OK);
    assert_eq!(fetched.data()["repository_url"], "https://github.com/acme/billing");

    let updated = app
        .patch(
            &format!("/api/v1/projects/{id}"),
            json!({ "name": "Billing", "description": null, "default_branch": "develop" }),
        )
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(updated.data()["name"], "Billing");
    assert_eq!(updated.data()["description"], json!(null), "null clears a nullable field");
    assert_eq!(updated.data()["repository_url"], "https://github.com/acme/billing", "omitted fields are untouched");
    assert_eq!(updated.data()["default_branch"], "develop");

    app.delete(&format!("/api/v1/projects/{id}")).await.assert_status(StatusCode::NO_CONTENT);
    app.get(&format!("/api/v1/projects/{id}")).await.assert_status(StatusCode::NOT_FOUND);
    app.delete(&format!("/api/v1/projects/{id}")).await.assert_status(StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn generated_slugs_are_unique_and_explicit_slug_conflicts(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let slug_of = |r: &crate::common::TestResponse| r.data()["slug"].as_str().unwrap().to_owned();

    let first = app.post("/api/v1/projects", json!({ "name": "Web App" })).await.assert_status(StatusCode::CREATED);
    let second = app.post("/api/v1/projects", json!({ "name": "Web App" })).await.assert_status(StatusCode::CREATED);
    let third = app.post("/api/v1/projects", json!({ "name": "web  app!" })).await.assert_status(StatusCode::CREATED);
    assert_eq!([slug_of(&first), slug_of(&second), slug_of(&third)], ["web-app", "web-app-2", "web-app-3"]);

    let non_latin = app.post("/api/v1/projects", json!({ "name": "ប្រព័ន្ធ" })).await.assert_status(StatusCode::CREATED);
    assert!(slug_of(&non_latin).starts_with("project-"));

    let conflict = app
        .post("/api/v1/projects", json!({ "name": "Other", "slug": "web-app" }))
        .await
        .assert_status(StatusCode::CONFLICT);
    assert_eq!(conflict.error_code(), "conflict");
    assert!(conflict.body["error"]["details"]["slug"].is_array(), "conflict is attributed to the field");
}

#[sqlx::test]
async fn project_validation_errors_are_reported_per_field(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let response = app
        .post(
            "/api/v1/projects",
            json!({
                "name": "   ",
                "slug": "Not A Slug",
                "repository_url": "javascript:alert(1)",
                "default_branch": "bad branch",
            }),
        )
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    let details = &response.body["error"]["details"];
    for field in ["name", "slug", "repository_url", "default_branch"] {
        assert!(details[field].is_array(), "missing error for {field}: {details}");
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM projects").fetch_one(&app.pool).await.unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test]
async fn archive_and_restore_keep_status_and_timestamp_in_sync(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let id = app.create_project("Legacy").await;

    let archived = app.patch(&format!("/api/v1/projects/{id}"), json!({ "status": "archived" })).await;
    assert_eq!(archived.data()["status"], "archived");
    assert!(archived.data()["archived_at"].is_string());

    let active_only = app.get("/api/v1/projects?status=active").await;
    assert!(ids(&active_only).is_empty());
    let archived_only = app.get("/api/v1/projects?status=archived").await;
    assert_eq!(ids(&archived_only), vec![id.as_str()]);

    let restored = app.patch(&format!("/api/v1/projects/{id}"), json!({ "status": "active" })).await;
    assert_eq!(restored.data()["status"], "active");
    assert_eq!(restored.data()["archived_at"], json!(null));
}

#[sqlx::test]
async fn project_list_supports_search_sort_and_pagination(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    for name in ["Alpha", "Beta", "Gamma", "Delta", "Alphabet"] {
        app.create_project(name).await;
    }

    let page = app.get("/api/v1/projects?sort=name&order=asc&per_page=2&page=2").await.assert_status(StatusCode::OK);
    let names: Vec<&str> = page.body["data"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Beta", "Delta"]);
    assert_eq!(page.body["meta"], json!({ "page": 2, "per_page": 2, "total": 5, "total_pages": 3 }));

    let search = app.get("/api/v1/projects?search=ALPHA&sort=name&order=asc").await;
    assert_eq!(search.body["meta"]["total"], 2);

    // LIKE metacharacters in the term are literals, not wildcards.
    let wildcard = app.get("/api/v1/projects?search=%25").await;
    assert_eq!(wildcard.body["meta"]["total"], 0);

    let clamped = app.get("/api/v1/projects?per_page=100000").await;
    assert_eq!(clamped.body["meta"]["per_page"], 100);

    app.get("/api/v1/projects?sort=password_hash").await.assert_status(StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn environments_are_ordered_and_reorderable(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let dev = app.create_environment(&project, "Development").await;
    let staging = app.create_environment(&project, "Staging").await;
    let prod = app.create_environment(&project, "Production").await;
    let path = format!("/api/v1/projects/{project}/environments");

    let listed = app.get(&path).await.assert_status(StatusCode::OK);
    assert_eq!(ids(&listed), [dev.clone(), staging.clone(), prod.clone()]);
    assert_eq!(listed.data()[0]["slug"], "development");
    assert_eq!(listed.data()[0]["deployment_count"], 0);
    assert_eq!(listed.data()[0]["latest_deployment"], json!(null));

    let reordered = app
        .put(&format!("{path}/order"), json!({ "environment_ids": [prod, dev, staging] }))
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(ids(&reordered), [prod.clone(), dev.clone(), staging.clone()]);
    assert_eq!(ids(&app.get(&path).await), [prod.clone(), dev.clone(), staging.clone()]);

    // The list must name every environment exactly once.
    for bad in [json!([prod, dev]), json!([prod, dev, dev]), json!([prod, dev, staging, uuid::Uuid::now_v7()])] {
        let response = app.put(&format!("{path}/order"), json!({ "environment_ids": bad })).await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    }
}

#[sqlx::test]
async fn environment_names_are_unique_per_project_only(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let first = app.create_project("One").await;
    let second = app.create_project("Two").await;
    app.create_environment(&first, "Production").await;
    app.create_environment(&second, "Production").await;

    let duplicate = app
        .post(&format!("/api/v1/projects/{first}/environments"), json!({ "name": "production" }))
        .await
        .assert_status(StatusCode::CONFLICT);
    assert!(duplicate.body["error"]["details"]["name"].is_array());

    app.post(&format!("/api/v1/projects/{}/environments", uuid::Uuid::now_v7()), json!({ "name": "X" }))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn environment_update_and_validation(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let env = app.create_environment(&project, "Dev Server").await;

    let updated = app
        .patch(
            &format!("/api/v1/environments/{env}"),
            json!({ "name": "Dev Box", "color": "#22C55E", "server_label": "vps-eu-1", "description": "shared" }),
        )
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(updated.data()["slug"], "dev-box");
    assert_eq!(updated.data()["color"], "#22c55e", "colour is normalised to lower case");
    assert_eq!(updated.data()["server_label"], "vps-eu-1");
    assert_eq!(updated.data()["project"]["name"], "Shop");

    let cleared = app.patch(&format!("/api/v1/environments/{env}"), json!({ "server_label": null })).await;
    assert_eq!(cleared.data()["server_label"], json!(null));

    let invalid = app
        .patch(&format!("/api/v1/environments/{env}"), json!({ "color": "red" }))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(invalid.body["error"]["details"]["color"].is_array());
}

#[sqlx::test]
async fn deleting_a_project_cascades_to_everything_it_owns(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Doomed").await;
    let keep = app.create_project("Survivor").await;
    let env = app.create_environment(&project, "Production").await;
    let commit = app.create_commit(&project, 'a', "feat: thing").await;
    app.create_deployment(&project, &env, &commit, "successful", "2026-03-02T10:00:00Z").await;
    app.post(
        &format!("/api/v1/projects/{project}/releases"),
        json!({ "version": "1.0.0", "title": "First", "commit_ids": [commit] }),
    )
    .await
    .assert_status(StatusCode::CREATED);
    app.post(&format!("/api/v1/projects/{project}/notes"), json!({ "title": "Runbook" }))
        .await
        .assert_status(StatusCode::CREATED);
    app.create_commit(&keep, 'a', "unrelated").await;

    app.delete(&format!("/api/v1/projects/{project}")).await.assert_status(StatusCode::NO_CONTENT);

    for (table, expected) in [
        ("environments", 0),
        ("deployments", 0),
        ("releases", 0),
        ("release_commits", 0),
        ("notes", 0),
        ("commits", 1),
        ("projects", 1),
    ] {
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&app.pool)
            .await
            .unwrap();
        assert_eq!(count, expected, "{table}");
    }
}
