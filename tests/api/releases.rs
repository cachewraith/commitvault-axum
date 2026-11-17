use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use crate::common::{TestApp, ids};

fn notes_doc(text: &str) -> serde_json::Value {
    json!({ "type": "doc", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": text }] }] })
}

#[sqlx::test]
async fn release_crud_with_commits_and_notes(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let first = app.create_commit(&project, 'a', "feat: login").await;
    let second = app.create_commit(&project, 'b', "feat: cart").await;

    let created = app
        .post(
            &format!("/api/v1/projects/{project}/releases"),
            json!({
                "version": "1.4.0",
                "title": "Spring release",
                "description": "Checkout improvements",
                "release_notes": notes_doc("Adds the new checkout."),
                "commit_ids": [first, second, first],
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);
    let release = created.data();
    assert_eq!(release["status"], "draft");
    assert_eq!(release["released_at"], json!(null));
    assert_eq!(release["commit_count"], 2, "duplicate ids are collapsed");
    assert_eq!(release["commits"].as_array().unwrap().len(), 2);
    assert_eq!(release["release_notes"]["content"][0]["content"][0]["text"], "Adds the new checkout.");
    let id = created.id();

    // Marking it released stamps the release date.
    let released = app
        .patch(&format!("/api/v1/releases/{id}"), json!({ "status": "released" }))
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(released.data()["status"], "released");
    assert!(released.data()["released_at"].is_string());

    // Replacing the commit set.
    let trimmed = app.patch(&format!("/api/v1/releases/{id}"), json!({ "commit_ids": [second] })).await;
    assert_eq!(trimmed.data()["commit_count"], 1);
    assert_eq!(trimmed.data()["commits"][0]["id"], second.as_str());

    // The commit knows which releases contain it.
    let commit = app.get(&format!("/api/v1/commits/{second}")).await;
    assert_eq!(commit.data()["releases"][0]["version"], "1.4.0");
    assert_eq!(commit.data()["releases"][0]["status"], "released");
    assert_eq!(ids(&app.get(&format!("/api/v1/commits?release_id={id}")).await), [second.as_str()]);
    assert_eq!(ids(&app.get(&format!("/api/v1/releases?commit_id={second}")).await), [id.as_str()]);

    let cleared =
        app.patch(&format!("/api/v1/releases/{id}"), json!({ "release_notes": null, "description": null })).await;
    assert_eq!(cleared.data()["release_notes"], json!(null));
    assert_eq!(cleared.data()["description"], json!(null));

    app.delete(&format!("/api/v1/releases/{id}")).await.assert_status(StatusCode::NO_CONTENT);
    app.get(&format!("/api/v1/releases/{id}")).await.assert_status(StatusCode::NOT_FOUND);
    // Commits outlive the release.
    app.get(&format!("/api/v1/commits/{second}")).await.assert_status(StatusCode::OK);
}

#[sqlx::test]
async fn versions_are_free_form_but_unique_per_project(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let shop = app.create_project("Shop").await;
    let blog = app.create_project("Blog").await;
    let path = format!("/api/v1/projects/{shop}/releases");

    for version in ["1.0.0", "2026.03", "v2-beta", "spring-drop"] {
        app.post(&path, json!({ "version": version, "title": "Release" })).await.assert_status(StatusCode::CREATED);
    }
    let duplicate =
        app.post(&path, json!({ "version": "1.0.0", "title": "Again" })).await.assert_status(StatusCode::CONFLICT);
    assert!(duplicate.body["error"]["details"]["version"].is_array());

    app.post(&format!("/api/v1/projects/{blog}/releases"), json!({ "version": "1.0.0", "title": "Blog 1.0" }))
        .await
        .assert_status(StatusCode::CREATED);

    let invalid =
        app.post(&path, json!({ "version": "", "title": "" })).await.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(invalid.body["error"]["details"]["version"].is_array());
    assert!(invalid.body["error"]["details"]["title"].is_array());
}

#[sqlx::test]
async fn release_commits_must_belong_to_the_same_project(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let shop = app.create_project("Shop").await;
    let blog = app.create_project("Blog").await;
    let own = app.create_commit(&shop, 'a', "shop commit").await;
    let foreign = app.create_commit(&blog, 'b', "blog commit").await;

    let rejected = app
        .post(
            &format!("/api/v1/projects/{shop}/releases"),
            json!({ "version": "1.0.0", "title": "x", "commit_ids": [own, foreign] }),
        )
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(rejected.body["error"]["details"]["commit_ids"].is_array());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM releases").fetch_one(&app.pool).await.unwrap();
    assert_eq!(count, 0, "the failed request left nothing behind (transaction rolled back)");

    let id = app
        .post(
            &format!("/api/v1/projects/{shop}/releases"),
            json!({ "version": "1.0.0", "title": "x", "commit_ids": [own] }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .id();
    app.patch(&format!("/api/v1/releases/{id}"), json!({ "title": "renamed", "commit_ids": [foreign] }))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    let unchanged = app.get(&format!("/api/v1/releases/{id}")).await;
    assert_eq!(unchanged.data()["title"], "x", "a rejected update changes nothing");
    assert_eq!(unchanged.data()["commit_count"], 1);
}

#[sqlx::test]
async fn release_notes_are_validated_as_rich_text(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let rejected = app
        .post(
            &format!("/api/v1/projects/{project}/releases"),
            json!({ "version": "1.0.0", "title": "x", "release_notes": { "type": "doc", "content": [{ "type": "script" }] } }),
        )
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(rejected.body["error"]["details"]["release_notes"].is_array());
}

#[sqlx::test]
async fn release_list_filters_and_ordering(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let path = format!("/api/v1/projects/{project}/releases");
    let old = app.post(&path, json!({ "version": "1.0.0", "title": "First", "status": "released", "released_at": "2026-01-01T00:00:00Z" })).await.id();
    let new = app.post(&path, json!({ "version": "1.1.0", "title": "Second", "status": "released", "released_at": "2026-02-01T00:00:00Z", "release_notes": notes_doc("Fixes the flaky importer") })).await.id();
    let draft = app.post(&path, json!({ "version": "2.0.0", "title": "Next" })).await.id();

    assert_eq!(
        ids(&app.get(&path).await),
        [draft.clone(), new.clone(), old.clone()],
        "drafts first, then newest release"
    );
    assert_eq!(ids(&app.get("/api/v1/releases?status=released").await), [new.clone(), old.clone()]);
    assert_eq!(ids(&app.get("/api/v1/releases?search=importer").await), [new.as_str()], "search covers note text");
    assert_eq!(ids(&app.get("/api/v1/releases?sort=version&order=asc").await), [old, new, draft]);
}
