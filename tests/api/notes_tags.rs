use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use crate::common::{TestApp, ids};

fn doc(content: serde_json::Value) -> serde_json::Value {
    json!({ "type": "doc", "content": content })
}

fn paragraph(text: &str) -> serde_json::Value {
    json!({ "type": "paragraph", "content": [{ "type": "text", "text": text }] })
}

#[sqlx::test]
async fn note_crud_and_autosave(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;

    let created = app
        .post("/api/v1/notes", json!({ "title": "  Deploy runbook ", "project_id": project, "category": "Runbooks" }))
        .await
        .assert_status(StatusCode::CREATED);
    let note = created.data();
    assert_eq!(note["title"], "Deploy runbook");
    assert_eq!(note["project"]["name"], "Shop");
    assert_eq!(note["content_json"], doc(json!([{ "type": "paragraph" }])), "defaults to an empty document");
    assert_eq!(note["excerpt"], "");
    assert_eq!(note["is_pinned"], false);
    let id = created.id();

    // Autosave: only the document is sent.
    let content = doc(json!([
        { "type": "heading", "attrs": { "level": 2 }, "content": [{ "type": "text", "text": "Steps" }] },
        paragraph("Run the migration, then restart the worker."),
        { "type": "codeBlock", "attrs": { "language": "bash" }, "content": [{ "type": "text", "text": "sqlx migrate run" }] },
    ]));
    let saved = app
        .patch(&format!("/api/v1/notes/{id}"), json!({ "content_json": content }))
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(saved.data()["content_json"], content, "the structured document round-trips unchanged");
    assert_eq!(saved.data()["title"], "Deploy runbook");
    assert!(saved.data()["excerpt"].as_str().unwrap().starts_with("Steps Run the migration"));

    // Stored as JSONB, never as HTML.
    let (kind, text): (String, String) = sqlx::query_as("SELECT jsonb_typeof(content_json), content_text FROM notes")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(kind, "object");
    assert!(text.contains("sqlx migrate run") && !text.contains('<'));

    let pinned = app
        .patch(&format!("/api/v1/notes/{id}"), json!({ "is_pinned": true, "project_id": null, "category": null }))
        .await;
    assert_eq!(pinned.data()["is_pinned"], true);
    assert_eq!(pinned.data()["project"], json!(null), "null detaches the note from its project");
    assert_eq!(pinned.data()["category"], json!(null));

    app.delete(&format!("/api/v1/notes/{id}")).await.assert_status(StatusCode::NO_CONTENT);
    app.get(&format!("/api/v1/notes/{id}")).await.assert_status(StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn note_documents_are_sanitised_before_storage(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;

    let unsupported = app
        .post("/api/v1/notes", json!({ "title": "x", "content_json": doc(json!([{ "type": "iframe", "attrs": { "src": "https://evil.example" } }])) }))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(unsupported.body["error"]["details"]["content_json"].is_array());

    app.post("/api/v1/notes", json!({ "title": "x", "content_json": "<script>alert(1)</script>" }))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    let hostile = doc(json!([
        { "type": "paragraph", "attrs": { "onclick": "steal()" }, "content": [
            { "type": "text", "text": "click me", "marks": [{ "type": "link", "attrs": { "href": "javascript:alert(document.cookie)" } }] },
            { "type": "text", "text": " safe", "marks": [{ "type": "link", "attrs": { "href": "https://example.com", "target": "_self", "onmouseover": "x()" } }] },
        ]},
        { "type": "image", "attrs": { "src": "data:image/svg+xml;base64,PHN2Zz4=", "onerror": "x()" } },
        { "type": "image", "attrs": { "src": "https://example.com/diagram.png", "onerror": "x()" } },
    ]));
    let created = app
        .post("/api/v1/notes", json!({ "title": "x", "content_json": hostile }))
        .await
        .assert_status(StatusCode::CREATED);
    let stored = created.data()["content_json"].to_string();
    for forbidden in ["javascript:", "onclick", "onmouseover", "onerror", "data:image", "_self"] {
        assert!(!stored.contains(forbidden), "{forbidden} survived sanitisation: {stored}");
    }
    assert!(stored.contains("click me"), "text of a stripped link is kept");
    assert!(stored.contains("https://example.com/diagram.png"));
    assert!(stored.contains("noopener noreferrer nofollow"));
}

#[sqlx::test]
async fn note_list_filters_search_and_pinning(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let project = app.create_project("Shop").await;
    let make = |title: &str, body: &str, extra: serde_json::Value| {
        let mut note = json!({ "title": title, "content_json": doc(json!([paragraph(body)])) });
        note.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        note
    };
    let runbook = app
        .post(
            "/api/v1/notes",
            make("Deploy runbook", "restart the worker", json!({ "project_id": project, "category": "Runbooks" })),
        )
        .await
        .id();
    let decision = app
        .post("/api/v1/notes", make("Why Postgres", "we chose jsonb for documents", json!({ "category": "Decisions" })))
        .await
        .id();
    let pinned = app.post("/api/v1/notes", make("Onboarding", "start here", json!({ "is_pinned": true }))).await.id();

    // Pinned first, then most recently updated.
    assert_eq!(ids(&app.get("/api/v1/notes").await), [pinned.clone(), decision.clone(), runbook.clone()]);
    assert_eq!(ids(&app.get(&format!("/api/v1/projects/{project}/notes")).await), [runbook.as_str()]);
    assert_eq!(ids(&app.get(&format!("/api/v1/notes?project_id={project}")).await), [runbook.as_str()]);
    assert_eq!(ids(&app.get("/api/v1/notes?global=true").await), [pinned.clone(), decision.clone()]);
    assert_eq!(ids(&app.get("/api/v1/notes?category=Decisions").await), [decision.as_str()]);
    assert_eq!(ids(&app.get("/api/v1/notes?pinned=true").await), [pinned.as_str()]);
    // Search reaches into the document text.
    assert_eq!(ids(&app.get("/api/v1/notes?search=JSONB").await), [decision.as_str()]);
    assert_eq!(ids(&app.get("/api/v1/notes?search=runbook").await), [runbook.as_str()]);

    // Lists never ship document bodies.
    let list = app.get("/api/v1/notes").await;
    assert!(list.data()[0].get("content_json").is_none());
    assert_eq!(list.data()[1]["excerpt"], "we chose jsonb for documents");

    let categories = app.get("/api/v1/notes/categories").await.assert_status(StatusCode::OK);
    assert_eq!(categories.data().as_array().unwrap().len(), 2);
    let scoped = app.get(&format!("/api/v1/notes/categories?project_id={project}")).await;
    assert_eq!(scoped.data(), &json!([{ "category": "Runbooks", "note_count": 1 }]));

    app.post("/api/v1/notes", json!({ "title": "x", "project_id": uuid::Uuid::now_v7() }))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test]
async fn tag_crud_and_usage_counts(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;

    let created = app
        .post("/api/v1/tags", json!({ "name": "Hot Fix", "color": "#EF4444" }))
        .await
        .assert_status(StatusCode::CREATED);
    assert_eq!(created.data()["slug"], "hot-fix");
    assert_eq!(created.data()["color"], "#ef4444");
    assert_eq!(created.data()["usage"]["total"], 0);
    let id = created.id();

    let duplicate = app.post("/api/v1/tags", json!({ "name": "hot fix" })).await.assert_status(StatusCode::CONFLICT);
    assert!(duplicate.body["error"]["details"]["name"].is_array());
    app.post("/api/v1/tags", json!({ "name": "", "color": "blue" }))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    let renamed =
        app.patch(&format!("/api/v1/tags/{id}"), json!({ "name": "Hotfix" })).await.assert_status(StatusCode::OK);
    assert_eq!(renamed.data()["slug"], "hotfix");
    assert_eq!(renamed.data()["color"], "#ef4444");

    app.create_tag("backend").await;
    let listed = app.get("/api/v1/tags?search=hot").await.assert_status(StatusCode::OK);
    assert_eq!(ids(&listed), [id.as_str()]);
    assert_eq!(app.get("/api/v1/tags").await.body["meta"]["total"], 2);

    app.delete(&format!("/api/v1/tags/{id}")).await.assert_status(StatusCode::NO_CONTENT);
    app.get(&format!("/api/v1/tags/{id}")).await.assert_status(StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn tags_attach_to_every_record_type(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let urgent = app.create_tag("urgent").await;
    let backend = app.create_tag("backend").await;

    let project = app
        .post("/api/v1/projects", json!({ "name": "Shop", "tag_ids": [backend] }))
        .await
        .assert_status(StatusCode::CREATED)
        .id();
    let env = app.create_environment(&project, "Production").await;
    let commit = app
        .create_commit_with(&project, json!({ "branch_name": "main", "commit_hash": crate::common::commit_hash('a'), "commit_message": "fix: outage", "commit_date": "2026-03-01T10:00:00Z", "tag_ids": [urgent, backend] }))
        .await;
    let plain_commit = app.create_commit(&project, 'b', "chore: deps").await;
    let deployment = app
        .post(
            &format!("/api/v1/projects/{project}/deployments"),
            json!({ "environment_id": env, "commit_id": commit, "deployment_title": "Hotfix", "tag_ids": [urgent] }),
        )
        .await
        .assert_status(StatusCode::CREATED);
    let release = app
        .post(
            &format!("/api/v1/projects/{project}/releases"),
            json!({ "version": "1.0.1", "title": "Patch", "tag_ids": [urgent] }),
        )
        .await
        .assert_status(StatusCode::CREATED);
    let note = app
        .post("/api/v1/notes", json!({ "title": "Postmortem", "tag_ids": [urgent] }))
        .await
        .assert_status(StatusCode::CREATED);

    // Tags come back embedded, sorted by name.
    let fetched = app.get(&format!("/api/v1/commits/{commit}")).await;
    let names: Vec<&str> =
        fetched.data()["tags"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["backend", "urgent"]);
    assert_eq!(deployment.data()["tags"][0]["name"], "urgent");
    assert_eq!(release.data()["tags"][0]["name"], "urgent");
    assert_eq!(note.data()["tags"][0]["name"], "urgent");

    // Filtering by tag on each list.
    assert_eq!(ids(&app.get(&format!("/api/v1/commits?tag_id={urgent}")).await), [commit.as_str()]);
    assert_eq!(ids(&app.get(&format!("/api/v1/projects?tag_id={backend}")).await), [project.as_str()]);
    assert_eq!(ids(&app.get(&format!("/api/v1/deployments?tag_id={urgent}")).await), [deployment.id()]);
    assert_eq!(ids(&app.get(&format!("/api/v1/releases?tag_id={urgent}")).await), [release.id()]);
    assert_eq!(ids(&app.get(&format!("/api/v1/notes?tag_id={urgent}")).await), [note.id()]);

    let usage = app.get(&format!("/api/v1/tags/{urgent}")).await;
    assert_eq!(
        usage.data()["usage"],
        json!({ "project_count": 0, "commit_count": 1, "deployment_count": 1, "release_count": 1, "note_count": 1, "total": 4 })
    );
    let by_usage = app.get("/api/v1/tags?sort=usage").await;
    assert_eq!(ids(&by_usage), [urgent.clone(), backend.clone()]);

    // tag_ids replaces the set; an empty list clears it.
    let retagged = app.patch(&format!("/api/v1/commits/{commit}"), json!({ "tag_ids": [backend] })).await;
    assert_eq!(retagged.data()["tags"].as_array().unwrap().len(), 1);
    let cleared = app.patch(&format!("/api/v1/commits/{commit}"), json!({ "tag_ids": [] })).await;
    assert_eq!(cleared.data()["tags"], json!([]));

    // Unknown tags are rejected and nothing is half-applied.
    let unknown = app
        .patch(
            &format!("/api/v1/commits/{plain_commit}"),
            json!({ "commit_message": "changed", "tag_ids": [urgent, uuid::Uuid::now_v7()] }),
        )
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(unknown.body["error"]["details"]["tag_ids"].is_array());
    let untouched = app.get(&format!("/api/v1/commits/{plain_commit}")).await;
    assert_eq!(untouched.data()["commit_message"], "chore: deps");
    assert_eq!(untouched.data()["tags"], json!([]));

    // Deleting a tag only detaches it.
    app.delete(&format!("/api/v1/tags/{urgent}")).await.assert_status(StatusCode::NO_CONTENT);
    let after = app.get(&format!("/api/v1/notes/{}", note.id())).await.assert_status(StatusCode::OK);
    assert_eq!(after.data()["tags"], json!([]));
}
