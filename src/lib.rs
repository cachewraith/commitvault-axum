//! CommitVault backend: a modular monolith exposing a REST API under `/api/v1`.
//!
//! Layering inside each feature module: `routes` → `handlers` (HTTP only) →
//! `service` (business rules, transactions) → `repository` (SQL).

pub mod auth;
pub mod config;
pub mod database;
pub mod errors;
pub mod mail;
pub mod middleware;
pub mod modules;
pub mod openapi;
pub mod shared;

use std::{sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    http::{HeaderName, HeaderValue, Method, StatusCode, header},
    middleware::from_fn_with_state,
};
use sha2::{Digest, Sha512};
use sqlx::PgPool;
use tower_http::{
    compression::CompressionLayer,
    cors::CorsLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    set_header::SetResponseHeaderLayer,
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
use tower_sessions::{
    Expiry, SessionManagerLayer,
    cookie::{Key, SameSite},
};

use crate::{
    auth::{rate_limit::LoginRateLimiter, session_store::PgSessionStore},
    config::Config,
    errors::AppError,
    mail::Mailer,
};

/// Largest accepted request body. Rich-text documents are the biggest payloads.
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Shared dependencies, injected into handlers through axum state.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub config: Arc<Config>,
    pub login_limiter: Arc<LoginRateLimiter>,
    pub mailer: Mailer,
}

impl AppState {
    pub fn new(db: PgPool, config: Config, mailer: Mailer) -> Self {
        Self { db, config: Arc::new(config), login_limiter: Arc::new(LoginRateLimiter::default()), mailer }
    }
}

/// Builds the complete HTTP application.
pub fn build_app(state: AppState) -> Router {
    let config = Arc::clone(&state.config);
    let (api_routes, spec) = openapi::routes().split_for_parts();

    let sessions = SessionManagerLayer::new(PgSessionStore::new(state.db.clone()))
        .with_name(config.session_cookie_name())
        .with_http_only(true)
        .with_secure(config.cookie_secure)
        .with_same_site(SameSite::Strict)
        .with_path("/")
        // Sliding expiry: every authenticated request extends the session.
        .with_expiry(Expiry::OnInactivity(time::Duration::seconds(config.session_ttl.as_secs() as i64)))
        .with_always_save(true)
        .with_signed(session_signing_key(&config));

    // `openapi::routes()` already mounts everything under `/api/v1`.
    // Order matters (outermost last): sessions wrap everything, CSRF wraps the handlers.
    let mut app = api_routes
        .layer(from_fn_with_state(state.clone(), middleware::csrf::csrf_protect))
        .layer(axum::middleware::from_fn(middleware::auth::limit_anonymous_sessions))
        .layer(sessions);
    if config.enable_api_docs {
        app = app.merge(docs_router(spec));
    }

    let request_id = HeaderName::from_static("x-request-id");
    middleware::security_headers::apply(app, &config)
        .fallback(|| async { AppError::NotFound("Endpoint") })
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(cors_layer(&config))
        .layer(TimeoutLayer::with_status_code(StatusCode::GATEWAY_TIMEOUT, REQUEST_TIMEOUT))
        .layer(CompressionLayer::new())
        // Wraps sessions, CSRF and handlers: `Server-Timing` is the backend's whole share.
        .layer(axum::middleware::from_fn(middleware::timing::server_timing))
        .layer(PropagateRequestIdLayer::new(request_id.clone()))
        // The default span records method, path and version — never headers or
        // bodies, so cookies and credentials stay out of the logs.
        .layer(TraceLayer::new_for_http())
        .layer(SetRequestIdLayer::new(request_id, MakeRequestUuid))
        .with_state(state)
}

/// Signing key for the session cookie, derived from `SESSION_SECRET`.
/// The cookie only carries the opaque session id; signing it means forged or
/// tampered ids are rejected before the database is consulted.
fn session_signing_key(config: &Config) -> Key {
    Key::from(&Sha512::digest(config.session_secret.expose().as_bytes()))
}

/// CORS for the configured frontend origins only. In the recommended
/// same-origin deployment the browser never needs it; it exists for setups
/// that host the frontend on a separate origin.
fn cors_layer(config: &Config) -> CorsLayer {
    let origins: Vec<HeaderValue> =
        config.frontend_origins.iter().filter_map(|origin| HeaderValue::from_str(origin).ok()).collect();
    CorsLayer::new()
        .allow_origin(origins)
        .allow_credentials(true)
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::PUT, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE, HeaderName::from_static(auth::CSRF_HEADER)])
        .max_age(Duration::from_secs(600))
}

/// Swagger UI and the raw OpenAPI document (disabled in production by default).
fn docs_router(spec: utoipa::openapi::OpenApi) -> Router<AppState> {
    let raw = spec.clone();
    let ui: Router<AppState> =
        utoipa_swagger_ui::SwaggerUi::new("/api/docs").url("/api/docs/openapi.json", spec).into();
    ui.route("/api/v1/openapi.json", axum::routing::get(move || std::future::ready(Json(raw.clone())))).layer(
        SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(middleware::security_headers::DOCS_CSP),
        ),
    )
}
