//! Shared harness for the API integration tests.
//!
//! Every test gets its own freshly migrated database from `#[sqlx::test]` and
//! drives the real router in-process (no network), so the whole stack —
//! middleware, sessions, CSRF, validation, SQL — is exercised.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use commitvault::{
    AppState, auth, build_app,
    config::Config,
    mail::{Mailer, OutgoingEmail},
};
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

/// A browser-like client: keeps its cookies (session, trusted device) and
/// CSRF token. Outgoing email lands in `outbox`.
pub struct TestApp {
    pub pool: PgPool,
    router: Router,
    cookies: BTreeMap<String, String>,
    pub csrf_token: Option<String>,
    pub outbox: Arc<Mutex<Vec<OutgoingEmail>>>,
}

const SESSION_COOKIE: &str = "cv_session";

impl TestApp {
    /// App with the bootstrap administrator created, not signed in. Email is
    /// "configured" (kept in memory), so new browsers get the code step.
    pub async fn new(pool: PgPool) -> Self {
        Self::with_config(pool, test_config(&[])).await
    }

    pub async fn with_config(pool: PgPool, config: Config) -> Self {
        let (mailer, outbox) = Mailer::memory();
        Self::with_mailer(pool, config, mailer, outbox).await
    }

    /// App without outgoing email: password-only sign-in, no password reset.
    pub async fn without_email(pool: PgPool) -> Self {
        Self::with_mailer(pool, test_config(&[]), Mailer::Disabled, Arc::default()).await
    }

    async fn with_mailer(pool: PgPool, config: Config, mailer: Mailer, outbox: Arc<Mutex<Vec<OutgoingEmail>>>) -> Self {
        auth::bootstrap::ensure_admin(&pool, &config).await.expect("bootstrap admin");
        let router = build_app(AppState::new(pool.clone(), config, mailer));
        Self { pool, router, cookies: BTreeMap::new(), csrf_token: None, outbox }
    }

    /// App signed in as the administrator.
    pub async fn signed_in(pool: PgPool) -> Self {
        let mut app = Self::new(pool).await;
        app.login().await;
        app
    }

    /// A second, independent browser against the same server.
    pub fn new_client(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            router: self.router.clone(),
            cookies: BTreeMap::new(),
            csrf_token: None,
            outbox: Arc::clone(&self.outbox),
        }
    }

    /// The emails sent so far.
    pub fn sent(&self) -> Vec<OutgoingEmail> {
        self.outbox.lock().unwrap().clone()
    }

    /// The six-digit code in the most recent email.
    pub fn last_code(&self) -> String {
        let sent = self.sent();
        let body = &sent.last().expect("an email was sent").body;
        body.split_whitespace()
            .find(|word| word.len() == 6 && word.bytes().all(|b| b.is_ascii_digit()))
            .expect("email contains a code")
            .to_owned()
    }

    pub fn has_cookie(&self, name: &str) -> bool {
        self.cookies.contains_key(name)
    }

    pub async fn fetch_csrf(&mut self) -> String {
        let response = self.get("/api/v1/auth/csrf").await.assert_status(StatusCode::OK);
        let token = response.data()["csrf_token"].as_str().expect("csrf token").to_owned();
        self.csrf_token = Some(token.clone());
        token
    }

    /// Signs in, completing the emailed-code step when it is asked for.
    pub async fn login(&mut self) {
        let response = self.login_as(ADMIN_EMAIL, ADMIN_PASSWORD).await.assert_status(StatusCode::OK);
        if response.data()["status"] == "code_required" {
            let code = self.last_code();
            self.verify_code(&code).await.assert_status(StatusCode::OK);
        }
    }

    pub async fn verify_code(&mut self, code: &str) -> TestResponse {
        let response = self.post("/api/v1/auth/login/verify", serde_json::json!({ "code": code })).await;
        if let Some(token) = response.body["data"]["csrf_token"].as_str() {
            self.csrf_token = Some(token.to_owned());
        }
        response
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

    /// The session cookie as `name=value`.
    pub fn session_cookie(&self) -> Option<String> {
        self.cookies.get(SESSION_COOKIE).map(|value| format!("{SESSION_COOKIE}={value}"))
    }

    pub fn set_session_cookie(&mut self, cookie: Option<String>) {
        match cookie.as_deref().and_then(|pair| pair.split_once('=')) {
            Some((_, value)) => self.cookies.insert(SESSION_COOKIE.to_owned(), value.to_owned()),
            None => self.cookies.remove(SESSION_COOKIE),
        };
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
        if !self.cookies.is_empty() {
            let cookie = self.cookies.iter().map(|(name, value)| format!("{name}={value}")).collect::<Vec<_>>();
            headers.push((header::COOKIE.to_string(), cookie.join("; ")));
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
            let Some((name, value)) = pair.split_once('=') else { continue };
            let cleared = value.is_empty() || raw.to_ascii_lowercase().contains("max-age=0");
            if cleared {
                self.cookies.remove(name);
            } else {
                self.cookies.insert(name.to_owned(), value.to_owned());
            }
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

    pub async fn create_branch(&mut self, project_id: &str, name: &str) -> String {
        self.post(&format!("/api/v1/projects/{project_id}/branches"), serde_json::json!({ "name": name }))
            .await
            .assert_status(StatusCode::CREATED)
            .id()
    }

    /// Id of the project's branch called `name`, created if missing.
    pub async fn branch_id(&mut self, project_id: &str, name: &str) -> String {
        let list = self.get(&format!("/api/v1/projects/{project_id}/branches?per_page=100")).await;
        let existing = list.data().as_array().and_then(|branches| {
            branches.iter().find(|b| b["name"] == name).map(|b| b["id"].as_str().unwrap().to_owned())
        });
        match existing {
            Some(id) => id,
            None => self.create_branch(project_id, name).await,
        }
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

    /// A `branch_name` in `body` is a test convenience: it is replaced by the
    /// id of that branch (created on first use).
    pub async fn create_commit_with(&mut self, project_id: &str, mut body: Value) -> String {
        if let Some(name) = body.as_object_mut().and_then(|o| o.remove("branch_name")) {
            let id = self.branch_id(project_id, name.as_str().unwrap()).await;
            body["branch_id"] = Value::String(id);
        }
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
