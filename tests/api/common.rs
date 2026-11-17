//! Shared harness for the API integration tests.
//!
//! Every test gets its own freshly migrated database from `#[sqlx::test]` and
//! drives the real router in-process (no network), so the whole stack —
//! middleware, sessions, CSRF, validation, SQL — is exercised.

use std::collections::HashMap;

use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use commitvault::{AppState, auth, build_app, config::Config};
use http_body_util::BodyExt;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

pub const ADMIN_EMAIL: &str = "admin@example.com";
pub const ADMIN_PASSWORD: &str = "correct horse battery staple";
pub const ORIGIN: &str = "http://localhost:5173";

pub fn test_config(overrides: &[(&str, &str)]) -> Config {
    let mut vars: HashMap<String, String> = HashMap::from([
        ("DATABASE_URL".into(), "postgres://unused".into()),
        ("SESSION_SECRET".into(), "t".repeat(64)),
        ("BOOTSTRAP_ADMIN_EMAIL".into(), ADMIN_EMAIL.into()),
        ("BOOTSTRAP_ADMIN_PASSWORD".into(), ADMIN_PASSWORD.into()),
        ("FRONTEND_ORIGIN".into(), ORIGIN.into()),
    ]);
    for (key, value) in overrides {
        vars.insert((*key).into(), (*value).into());
    }
    Config::from_lookup(|key| vars.get(key).cloned()).expect("valid test configuration")
}

pub struct TestResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

impl TestResponse {
    /// The `data` member of a success envelope.
    pub fn data(&self) -> &Value {
        &self.body["data"]
    }

    pub fn error_code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or_default()
    }

    pub fn id(&self) -> String {
        self.body["data"]["id"].as_str().expect("response has data.id").to_owned()
    }

    #[track_caller]
    pub fn assert_status(self, expected: StatusCode) -> Self {
        assert_eq!(self.status, expected, "unexpected status; body: {}", self.body);
        self
    }
}

/// A browser-like client: keeps the session cookie and CSRF token.
pub struct TestApp {
    pub pool: PgPool,
    router: Router,
    cookie: Option<String>,
    pub csrf_token: Option<String>,
}

impl TestApp {
    /// App with the bootstrap administrator created, not signed in.
    pub async fn new(pool: PgPool) -> Self {
        Self::with_config(pool, test_config(&[])).await
    }

    pub async fn with_config(pool: PgPool, config: Config) -> Self {
        auth::bootstrap::ensure_admin(&pool, &config).await.expect("bootstrap admin");
        let router = build_app(AppState::new(pool.clone(), config));
        Self { pool, router, cookie: None, csrf_token: None }
    }

    /// App signed in as the administrator.
    pub async fn signed_in(pool: PgPool) -> Self {
        let mut app = Self::new(pool).await;
        app.login().await;
        app
    }

    /// A second, independent browser against the same server.
    pub fn new_client(&self) -> Self {
        Self { pool: self.pool.clone(), router: self.router.clone(), cookie: None, csrf_token: None }
    }

    pub async fn fetch_csrf(&mut self) -> String {
        let response = self.get("/api/v1/auth/csrf").await.assert_status(StatusCode::OK);
        let token = response.data()["csrf_token"].as_str().expect("csrf token").to_owned();
        self.csrf_token = Some(token.clone());
        token
    }

    pub async fn login(&mut self) {
        self.login_as(ADMIN_EMAIL, ADMIN_PASSWORD).await.assert_status(StatusCode::OK);
    }

    pub async fn login_as(&mut self, email: &str, password: &str) -> TestResponse {
        if self.csrf_token.is_none() {
            self.fetch_csrf().await;
        }
        let response =
            self.post("/api/v1/auth/login", serde_json::json!({ "email": email, "password": password })).await;
        if let Some(token) = response.body["data"]["csrf_token"].as_str() {
            self.csrf_token = Some(token.to_owned());
        }
        response
    }

    pub fn session_cookie(&self) -> Option<&str> {
        self.cookie.as_deref()
    }

    pub fn set_session_cookie(&mut self, cookie: Option<String>) {
        self.cookie = cookie;
    }

    pub async fn get(&mut self, path: &str) -> TestResponse {
        self.send(Method::GET, path, None, &[]).await
    }

    pub async fn post(&mut self, path: &str, body: Value) -> TestResponse {
        self.send(Method::POST, path, Some(body), &[]).await
    }

    pub async fn patch(&mut self, path: &str, body: Value) -> TestResponse {
        self.send(Method::PATCH, path, Some(body), &[]).await
    }

    pub async fn put(&mut self, path: &str, body: Value) -> TestResponse {
        self.send(Method::PUT, path, Some(body), &[]).await
    }

    pub async fn delete(&mut self, path: &str) -> TestResponse {
        self.send(Method::DELETE, path, None, &[]).await
    }

    /// Sends a request with the stored cookie and CSRF token. `extra_headers`
    /// override the defaults (pass an empty value to omit a default header).
    pub async fn send(
        &mut self,
        method: Method,
        path: &str,
        body: Option<Value>,
        extra_headers: &[(&str, &str)],
    ) -> TestResponse {
        let mut headers: Vec<(String, String)> = Vec::new();
        if let Some(cookie) = &self.cookie {
            headers.push((header::COOKIE.to_string(), cookie.clone()));
        }
        if !matches!(method, Method::GET | Method::HEAD) {
            headers.push((header::ORIGIN.to_string(), ORIGIN.to_owned()));
            if let Some(token) = &self.csrf_token {
                headers.push(("x-csrf-token".to_owned(), token.clone()));
            }
        }
        for (name, value) in extra_headers {
            headers.retain(|(existing, _)| !existing.eq_ignore_ascii_case(name));
            if !value.is_empty() {
                headers.push(((*name).to_owned(), (*value).to_owned()));
            }
        }

        let mut builder = Request::builder().method(method).uri(path);
        for (name, value) in &headers {
            builder = builder.header(name, value);
        }
        let request = match body {
            Some(json) => builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&json).unwrap())),
            None => builder.body(Body::empty()),
        }
        .expect("valid request");

        let response = self.router.clone().oneshot(request).await.expect("router is infallible");
        let status = response.status();
        let headers = response.headers().clone();

        for set_cookie in headers.get_all(header::SET_COOKIE) {
            let raw = set_cookie.to_str().unwrap_or_default();
            let pair = raw.split(';').next().unwrap_or_default().trim();
            let cleared = pair.ends_with('=') || raw.to_ascii_lowercase().contains("max-age=0");
            self.cookie = if cleared { None } else { Some(pair.to_owned()) };
        }

        let bytes = response.into_body().collect().await.expect("response body").to_bytes();
        let body = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::Null) };
        TestResponse { status, headers, body }
    }
}

/// Fixture helpers: each creates one record through the public API and
/// returns its id.
impl TestApp {
    pub async fn create_project(&mut self, name: &str) -> String {
        self.post("/api/v1/projects", serde_json::json!({ "name": name })).await.assert_status(StatusCode::CREATED).id()
    }

    pub async fn create_environment(&mut self, project_id: &str, name: &str) -> String {
        self.post(&format!("/api/v1/projects/{project_id}/environments"), serde_json::json!({ "name": name }))
            .await
            .assert_status(StatusCode::CREATED)
            .id()
    }

    /// `seed` becomes the first hex digit of a 40-character hash.
    pub async fn create_commit(&mut self, project_id: &str, seed: char, message: &str) -> String {
        self.create_commit_with(
            project_id,
            serde_json::json!({
                "branch_name": "main",
                "commit_hash": commit_hash(seed),
                "commit_message": message,
                "commit_date": "2026-03-01T10:00:00Z",
            }),
        )
        .await
    }

    pub async fn create_commit_with(&mut self, project_id: &str, body: Value) -> String {
        self.post(&format!("/api/v1/projects/{project_id}/commits"), body).await.assert_status(StatusCode::CREATED).id()
    }

    pub async fn create_deployment(
        &mut self,
        project_id: &str,
        environment_id: &str,
        commit_id: &str,
        status: &str,
        deployed_at: &str,
    ) -> String {
        self.post(
            &format!("/api/v1/projects/{project_id}/deployments"),
            serde_json::json!({
                "environment_id": environment_id,
                "commit_id": commit_id,
                "deployment_title": format!("Deploy to {environment_id}"),
                "status": status,
                "deployed_at": deployed_at,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .id()
    }

    pub async fn create_tag(&mut self, name: &str) -> String {
        self.post("/api/v1/tags", serde_json::json!({ "name": name })).await.assert_status(StatusCode::CREATED).id()
    }
}

/// A valid 40-character hash starting with `seed`.
pub fn commit_hash(seed: char) -> String {
    format!("{seed}{}", "0123456789abcdef0123456789abcdef0123456".chars().take(39).collect::<String>())
}

/// Ids of the items in a list response, in order.
pub fn ids(response: &TestResponse) -> Vec<String> {
    response.body["data"]
        .as_array()
        .expect("list response")
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect()
}
