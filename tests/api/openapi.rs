use axum::http::StatusCode;
use sqlx::PgPool;

use crate::common::{TestApp, test_config};

#[test]
fn spec_documents_every_endpoint_group() {
    let spec = serde_json::to_value(commitvault::openapi::spec()).unwrap();
    let paths = spec["paths"].as_object().unwrap();

    for path in [
        "/api/v1/health",
        "/api/v1/auth/csrf",
        "/api/v1/auth/login",
        "/api/v1/auth/logout",
        "/api/v1/auth/me",
        "/api/v1/auth/password",
        "/api/v1/auth/options",
        "/api/v1/auth/login/verify",
        "/api/v1/auth/login/resend",
        "/api/v1/auth/password/forgot",
        "/api/v1/auth/password/reset",
        "/api/v1/dashboard",
        "/api/v1/projects",
        "/api/v1/projects/{id}",
        "/api/v1/projects/{project_id}/environments",
        "/api/v1/projects/{project_id}/environments/order",
        "/api/v1/environments",
        "/api/v1/environments/{id}",
        "/api/v1/projects/{project_id}/commits",
        "/api/v1/projects/{project_id}/branches",
        "/api/v1/branches/{id}",
        "/api/v1/commits",
        "/api/v1/commits/{id}",
        "/api/v1/projects/{project_id}/deployments",
        "/api/v1/projects/{project_id}/deployments/compare",
        "/api/v1/deployments",
        "/api/v1/deployments/{id}",
        "/api/v1/projects/{project_id}/releases",
        "/api/v1/releases",
        "/api/v1/releases/{id}",
        "/api/v1/projects/{project_id}/notes",
        "/api/v1/notes",
        "/api/v1/notes/categories",
        "/api/v1/notes/{id}",
        "/api/v1/tags",
        "/api/v1/tags/{id}",
        "/api/v1/search",
    ] {
        assert!(paths.contains_key(path), "OpenAPI spec is missing {path}");
    }

    let operations: usize = paths.values().map(|item| item.as_object().unwrap().len()).sum();
    assert_eq!(operations, 62, "update this count (and docs/API.md) when endpoints are added or removed");

    let schemas = spec["components"]["schemas"].as_object().unwrap();
    for schema in [
        "ErrorResponse",
        "ProjectResponse",
        "CommitResponse",
        "BranchResponse",
        "BranchDetailResponse",
        "LoginResponse",
        "DeploymentResponse",
        "EnvironmentComparison",
        "NoteDetailResponse",
        "DashboardResponse",
    ] {
        assert!(schemas.contains_key(schema), "OpenAPI spec is missing schema {schema}");
    }
    assert!(spec["components"]["securitySchemes"]["session_cookie"].is_object());

    // Every `$ref` must resolve, otherwise client type generation breaks.
    let text = spec.to_string();
    for reference in text.split("#/components/schemas/").skip(1) {
        let name: String = reference.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        assert!(schemas.contains_key(&name), "dangling schema reference: {name}");
    }
}

#[sqlx::test]
async fn api_docs_are_served_in_development_only(pool: PgPool) {
    let mut dev = TestApp::new(pool.clone()).await;
    let spec = dev.get("/api/v1/openapi.json").await.assert_status(StatusCode::OK);
    assert_eq!(spec.body["info"]["title"], "CommitVault API");

    let production = test_config(&[
        ("APP_ENV", "production"),
        ("FRONTEND_ORIGIN", "https://vault.example.com"),
        ("BOOTSTRAP_ADMIN_PASSWORD", "a sufficiently long passphrase"),
    ]);
    let mut prod = TestApp::with_config(pool, production).await;
    prod.get("/api/v1/openapi.json").await.assert_status(StatusCode::NOT_FOUND);
    prod.get("/api/docs").await.assert_status(StatusCode::NOT_FOUND);
}
