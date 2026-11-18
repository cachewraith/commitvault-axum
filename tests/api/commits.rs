use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use crate::common::{TestApp, commit_hash, ids};

#[sqlx::test]
async fn commit_crud_lifecycle(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let checkout = app.create_branch(&project, "feature/checkout").await;
    let hash = "A1B2C3D4E5F60718293A4B5C6D7E8F9012345678";

    let created = app
        .post(
            &format!("/api/v1/projects/{project}/commits"),
            json!({
                "branch_id": checkout,
                "commit_hash": hash,
                "commit_message": "feat: add checkout flow",
                "description": "Adds the three-step checkout.",
                "commit_date": "2026-03-01T10:00:00+07:00",
                "author_name": "Dara",
                "external_url": "https://github.com/acme/shop/commit/a1b2c3d",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);
    let commit = created.data();
    assert_eq!(commit["commit_hash"], hash.to_lowercase(), "hashes are stored lower-case");
    assert_eq!(commit["short_hash"], "a1b2c3d");
    assert_eq!(commit["commit_date"], "2026-03-01T03:00:00Z", "timestamps are normalised to UTC");
    assert_eq!(commit["project"]["name"], "Shop");
    assert_eq!(commit["branch_id"], checkout.as_str());
    assert_eq!(commit["branch"]["name"], "feature/checkout");
    assert_eq!(commit["deployment_count"], 0);
    assert_eq!(commit["environments"], json!([]));
    assert_eq!(commit["releases"], json!([]));
    let id = created.id();

    let updated = app
        .patch(&format!("/api/v1/commits/{id}"), json!({ "commit_message": "feat: checkout", "author_name": null }))
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(updated.data()["commit_message"], "feat: checkout");
    assert_eq!(updated.data()["author_name"], json!(null));
    assert_eq!(updated.data()["branch"]["name"], "feature/checkout");

    // Moving to another branch of the same project; never to another project's branch.
    let main = app.branch_id(&project, "main").await;
    let moved = app.patch(&format!("/api/v1/commits/{id}"), json!({ "branch_id": main })).await;
    assert_eq!(moved.assert_status(StatusCode::OK).data()["branch"]["name"], "main");
    let other_project = app.create_project("Blog").await;
    let foreign = app.branch_id(&other_project, "main").await;
    let rejected = app
        .patch(&format!("/api/v1/commits/{id}"), json!({ "branch_id": foreign }))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(rejected.body["error"]["details"]["branch_id"].is_array());

    // Changing the hash re-derives the abbreviation.
    let rehashed = app.patch(&format!("/api/v1/commits/{id}"), json!({ "commit_hash": commit_hash('f') })).await;
    assert_eq!(rehashed.data()["short_hash"], &commit_hash('f')[..7]);

    app.delete(&format!("/api/v1/commits/{id}")).await.assert_status(StatusCode::NO_CONTENT);
    app.get(&format!("/api/v1/commits/{id}")).await.assert_status(StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn duplicate_hash_is_rejected_within_a_project_but_allowed_across_projects(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let first = app.create_project("One").await;
    let second = app.create_project("Two").await;
    let original = app.create_commit(&first, 'a', "original").await;
    let other = app.create_commit(&first, 'b', "other").await;

    let main = app.branch_id(&first, "main").await;
    let duplicate = app
        .post(
            &format!("/api/v1/projects/{first}/commits"),
            json!({ "branch_id": main, "commit_hash": commit_hash('a').to_uppercase(), "commit_message": "dup", "commit_date": "2026-03-01T10:00:00Z" }),
        )
        .await
        .assert_status(StatusCode::CONFLICT);
    assert!(duplicate.body["error"]["details"]["commit_hash"].is_array());

    // Editing another commit onto an existing hash is also a conflict.
    app.patch(&format!("/api/v1/commits/{other}"), json!({ "commit_hash": commit_hash('a') }))
        .await
        .assert_status(StatusCode::CONFLICT);

    // The same hash in a different project is fine (forks, mirrors).
    app.create_commit(&second, 'a', "same hash elsewhere").await;
    app.get(&format!("/api/v1/commits/{original}")).await.assert_status(StatusCode::OK);
}

#[sqlx::test]
async fn commit_validation(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let path = format!("/api/v1/projects/{project}/commits");
    let main = app.branch_id(&project, "main").await;

    let invalid = app
        .post(
            &path,
            json!({
                "branch_id": main,
                "commit_hash": "xyz",
                "commit_message": "",
                "commit_date": "2026-03-01T10:00:00Z",
                "external_url": "javascript:alert(document.cookie)",
            }),
        )
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    let details = &invalid.body["error"]["details"];
    for field in ["commit_hash", "commit_message", "external_url"] {
        assert!(details[field].is_array(), "missing error for {field}: {details}");
    }

    let wrong_short = app
        .post(
            &path,
            json!({ "branch_id": main, "commit_hash": commit_hash('a'), "short_hash": "bbbbbbb", "commit_message": "x", "commit_date": "2026-03-01T10:00:00Z" }),
        )
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(wrong_short.body["error"]["details"]["short_hash"].is_array());

    let missing_date = app
        .post(&path, json!({ "branch_id": main, "commit_hash": commit_hash('a'), "commit_message": "x" }))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(missing_date.error_code(), "validation_failed");

    app.post(&format!("/api/v1/projects/{}/commits", uuid::Uuid::now_v7()), json!({ "branch_id": main, "commit_hash": commit_hash('a'), "commit_message": "x", "commit_date": "2026-03-01T10:00:00Z" }))
        .await
        .assert_status(StatusCode::NOT_FOUND);

    // A branch that does not exist (or belongs elsewhere) is a field error.
    let unknown = app
        .post(&path, json!({ "branch_id": uuid::Uuid::now_v7(), "commit_hash": commit_hash('a'), "commit_message": "x", "commit_date": "2026-03-01T10:00:00Z" }))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(unknown.body["error"]["details"]["branch_id"].is_array());
}

#[sqlx::test]
async fn commit_list_search_and_filters(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let shop = app.create_project("Shop").await;
    let blog = app.create_project("Blog").await;
    let make = |seed: char, branch: &str, message: &str, date: &str, author: &str| json!({ "branch_name": branch, "commit_hash": commit_hash(seed), "commit_message": message, "commit_date": date, "author_name": author });
    let login = app
        .create_commit_with(&shop, make('a', "feature/login", "feat: login form", "2026-01-10T00:00:00Z", "Dara"))
        .await;
    let cart = app
        .create_commit_with(&shop, make('b', "feature/cart", "feat: shopping cart", "2026-02-10T00:00:00Z", "Sok"))
        .await;
    let fix =
        app.create_commit_with(&shop, make('c', "main", "fix: login redirect", "2026-03-10T00:00:00Z", "Dara")).await;
    let post =
        app.create_commit_with(&blog, make('d', "main", "feat: first post", "2026-03-11T00:00:00Z", "Dara")).await;

    // Default order: newest commit first, across projects.
    assert_eq!(ids(&app.get("/api/v1/commits").await), [post.clone(), fix.clone(), cart.clone(), login.clone()]);

    // Project scoping, both ways of asking.
    assert_eq!(ids(&app.get(&format!("/api/v1/projects/{shop}/commits")).await).len(), 3);
    assert_eq!(ids(&app.get(&format!("/api/v1/commits?project_id={blog}")).await), [post.as_str()]);

    // Message search is case-insensitive.
    assert_eq!(ids(&app.get("/api/v1/commits?search=LOGIN").await), [fix.clone(), login.clone()]);
    // Branch search and exact branch filter.
    assert_eq!(ids(&app.get("/api/v1/commits?search=feature/").await), [cart.clone(), login.clone()]);
    let shop_main = app.branch_id(&shop, "main").await;
    assert_eq!(ids(&app.get(&format!("/api/v1/commits?branch_id={shop_main}")).await), [fix.as_str()]);
    // Hash prefix, via the dedicated filter and via the search box.
    let prefix = &commit_hash('b')[..10];
    assert_eq!(ids(&app.get(&format!("/api/v1/commits?hash={}", prefix.to_uppercase())).await), [cart.as_str()]);
    assert_eq!(ids(&app.get(&format!("/api/v1/commits?search={prefix}")).await), [cart.as_str()]);
    // Author search.
    assert_eq!(ids(&app.get("/api/v1/commits?search=sok").await), [cart.as_str()]);
    // Date range (inclusive).
    let ranged = app.get("/api/v1/commits?date_from=2026-02-01T00:00:00Z&date_to=2026-03-10T00:00:00Z").await;
    assert_eq!(ids(&ranged), [fix.clone(), cart.clone()]);
    // Sorting.
    let oldest_first = app.get("/api/v1/commits?sort=commit_date&order=asc&per_page=1").await;
    assert_eq!(ids(&oldest_first), [login.as_str()]);
    assert_eq!(oldest_first.body["meta"]["total"], 4);

    // Branch order groups by project, then branch, newest commit first.
    let grouped = app.get("/api/v1/commits?sort=branch_name&order=asc").await;
    assert_eq!(ids(&grouped), [post.clone(), cart.clone(), login.clone(), fix.clone()]);
}

#[sqlx::test]
async fn commits_can_be_filtered_by_recorded_deployment_status(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let staging = app.create_environment(&project, "Staging").await;
    let prod = app.create_environment(&project, "Production").await;
    let both = app.create_commit(&project, 'a', "in staging and production").await;
    let staging_only = app.create_commit(&project, 'b', "only staging").await;
    let failed = app.create_commit(&project, 'c', "failed in production").await;
    let none = app.create_commit(&project, 'd', "never recorded").await;

    app.create_deployment(&project, &staging, &both, "successful", "2026-03-02T10:00:00Z").await;
    app.create_deployment(&project, &prod, &both, "successful", "2026-03-03T10:00:00Z").await;
    app.create_deployment(&project, &staging, &staging_only, "successful", "2026-03-04T10:00:00Z").await;
    app.create_deployment(&project, &prod, &failed, "failed", "2026-03-05T10:00:00Z").await;

    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    let mut expectations: Vec<(String, Vec<String>)> = vec![
        ("deployment_status=none".into(), vec![none.clone()]),
        ("deployment_status=any".into(), vec![both.clone(), staging_only.clone(), failed.clone()]),
        ("deployment_status=failed".into(), vec![failed.clone()]),
        ("deployment_status=successful".into(), vec![both.clone(), staging_only.clone()]),
        // Scoped to one environment.
        (format!("environment_id={prod}"), vec![both.clone(), failed.clone()]),
        (format!("environment_id={prod}&deployment_status=successful"), vec![both.clone()]),
        // "No record in production" — which is not the same as "not deployed there".
        (format!("environment_id={prod}&deployment_status=none"), vec![staging_only.clone(), none.clone()]),
    ];
    for (query, expected) in expectations.drain(..) {
        let response =
            app.get(&format!("/api/v1/projects/{project}/commits?{query}")).await.assert_status(StatusCode::OK);
        assert_eq!(sorted(ids(&response)), sorted(expected), "{query}");
    }

    app.get("/api/v1/commits?deployment_status=deployed").await.assert_status(StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn commit_shows_where_it_has_recorded_deployments(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let staging = app.create_environment(&project, "Staging").await;
    let prod = app.create_environment(&project, "Production").await;
    let commit = app.create_commit(&project, 'a', "feat: thing").await;

    // Same commit, deployed three times across two environments.
    app.create_deployment(&project, &staging, &commit, "failed", "2026-03-02T10:00:00Z").await;
    app.create_deployment(&project, &staging, &commit, "successful", "2026-03-02T11:00:00Z").await;
    app.create_deployment(&project, &prod, &commit, "pending", "2026-03-03T10:00:00Z").await;

    let fetched = app.get(&format!("/api/v1/commits/{commit}")).await.assert_status(StatusCode::OK);
    assert_eq!(fetched.data()["deployment_count"], 3);
    let environments = fetched.data()["environments"].as_array().unwrap();
    assert_eq!(environments.len(), 2);
    assert_eq!(environments[0]["name"], "Staging");
    assert_eq!(environments[0]["deployment_count"], 2);
    assert_eq!(environments[0]["latest_status"], "successful", "the most recent record wins");
    assert_eq!(environments[0]["latest_deployed_at"], "2026-03-02T11:00:00Z");
    assert_eq!(environments[1]["name"], "Production");
    assert_eq!(environments[1]["latest_status"], "pending");
}

#[sqlx::test]
async fn deleting_a_commit_with_deployment_history_requires_cascade(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let env = app.create_environment(&project, "Production").await;
    let commit = app.create_commit(&project, 'a', "feat: thing").await;
    let deployment = app.create_deployment(&project, &env, &commit, "successful", "2026-03-02T10:00:00Z").await;

    let blocked = app.delete(&format!("/api/v1/commits/{commit}")).await.assert_status(StatusCode::CONFLICT);
    assert!(blocked.body["error"]["message"].as_str().unwrap().contains("recorded deployments"));
    app.get(&format!("/api/v1/commits/{commit}")).await.assert_status(StatusCode::OK);
    app.get(&format!("/api/v1/deployments/{deployment}")).await.assert_status(StatusCode::OK);

    app.delete(&format!("/api/v1/commits/{commit}?cascade=true")).await.assert_status(StatusCode::NO_CONTENT);
    app.get(&format!("/api/v1/commits/{commit}")).await.assert_status(StatusCode::NOT_FOUND);
    app.get(&format!("/api/v1/deployments/{deployment}")).await.assert_status(StatusCode::NOT_FOUND);
    // The environment itself is untouched.
    app.get(&format!("/api/v1/environments/{env}")).await.assert_status(StatusCode::OK);
}

#[sqlx::test]
async fn search_terms_cannot_inject_sql(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    app.create_commit(&project, 'a', "feat: thing").await;

    for term in ["'; DROP TABLE commits; --", "' OR '1'='1", "%' OR 1=1 --", "\\"] {
        let encoded: String = term.bytes().map(|b| format!("%{b:02X}")).collect();
        let response = app.get(&format!("/api/v1/commits?search={encoded}")).await.assert_status(StatusCode::OK);
        assert_eq!(response.body["meta"]["total"], 0, "{term}");
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM commits").fetch_one(&app.pool).await.unwrap();
    assert_eq!(count, 1);
}
