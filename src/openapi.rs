//! Route table and OpenAPI document. Handlers are registered through
//! `utoipa_axum::routes!`, so the served routes and the documented contract
//! cannot drift apart.

use axum::{Json, extract::State, http::StatusCode, middleware::from_fn};
use serde::Serialize;
use utoipa::{
    Modify, OpenApi, ToSchema,
    openapi::security::{ApiKey, ApiKeyValue, SecurityScheme},
};
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{AppState, auth, errors::ErrorResponse, middleware::auth::require_auth, modules, shared::response::Data};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "CommitVault API",
        description = "REST API of CommitVault, a personal workspace for recording commits, deployments, \
                       environments, releases and technical notes.\n\n\
                       All endpoints except `/health`, `/auth/csrf`, `/auth/login` and `/auth/logout` require the \
                       session cookie. Every POST, PUT, PATCH and DELETE request must also send the session's CSRF \
                       token in the `X-CSRF-Token` header."
    ),
    tags(
        (name = "auth", description = "Session and account"),
        (name = "dashboard", description = "Workspace overview"),
        (name = "projects", description = "Projects"),
        (name = "environments", description = "Deployment targets of a project"),
        (name = "commits", description = "Manually recorded commits"),
        (name = "deployments", description = "Recorded deployment events"),
        (name = "releases", description = "Releases and their commits"),
        (name = "notes", description = "Technical notes (rich text)"),
        (name = "tags", description = "Reusable tags"),
        (name = "search", description = "Global search"),
        (name = "system", description = "Operational endpoints"),
    ),
    // Types that only appear in query parameters are not discovered through
    // request/response bodies, so they are registered explicitly.
    components(schemas(
        ErrorResponse,
        crate::shared::pagination::SortOrder,
        modules::projects::dto::ProjectSort,
        modules::commits::dto::CommitSort,
        modules::commits::dto::CommitDeploymentFilter,
        modules::deployments::dto::DeploymentSort,
        modules::releases::dto::ReleaseSort,
        modules::notes::dto::NoteSort,
        modules::tags::dto::TagSort,
    )),
    modifiers(&SecuritySchemes)
)]
struct ApiDoc;

struct SecuritySchemes;

impl Modify for SecuritySchemes {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "session_cookie",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                "cv_session",
                "HttpOnly session cookie set by `POST /auth/login` (named `__Host-cv_session` over HTTPS).",
            ))),
        );
        components.add_security_scheme(
            "csrf_token",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::with_description(
                "X-CSRF-Token",
                "Per-session token from `GET /auth/csrf`; required on state-changing requests.",
            ))),
        );
    }
}

/// All API routes, relative to `/api/v1`, together with their OpenAPI
/// description. Pure: no state or I/O is needed to build it, which lets the
/// `openapi` CLI command export the spec without a database.
pub fn routes() -> OpenApiRouter<AppState> {
    let public = OpenApiRouter::new().routes(routes!(health)).merge(auth::routes::public_router());

    let protected = OpenApiRouter::new()
        .merge(auth::routes::protected_router())
        .merge(modules::router())
        .route_layer(from_fn(require_auth));

    OpenApiRouter::with_openapi(ApiDoc::openapi()).nest("/api/v1", public.merge(protected))
}

/// The OpenAPI document as served at `/api/v1/openapi.json`.
pub fn spec() -> utoipa::openapi::OpenApi {
    routes().into_openapi()
}

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    status: &'static str,
}

/// Liveness and database connectivity
#[utoipa::path(get, path = "/health", tag = "system",
    responses((status = 200, body = Data<HealthResponse>), (status = 503, body = Data<HealthResponse>)))]
async fn health(State(state): State<AppState>) -> (StatusCode, Json<Data<HealthResponse>>) {
    match sqlx::query_scalar!("SELECT 1").fetch_one(&state.db).await {
        Ok(_) => (StatusCode::OK, Data::json(HealthResponse { status: "ok" })),
        Err(err) => {
            tracing::error!(error = %err, "health check could not reach the database");
            (StatusCode::SERVICE_UNAVAILABLE, Data::json(HealthResponse { status: "database_unavailable" }))
        }
    }
}
