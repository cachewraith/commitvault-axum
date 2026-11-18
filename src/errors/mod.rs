//! Centralised error type. Every handler returns `AppResult<T>`; the
//! `IntoResponse` impl is the single place where errors become HTTP responses,
//! so internal details (SQL, stack traces) can never leak to clients.

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::rejection::{JsonRejection, PathRejection, QueryRejection},
    http::{HeaderValue, StatusCode, header::RETRY_AFTER},
    response::{IntoResponse, Response},
};
use serde::Serialize;
use utoipa::ToSchema;

pub type AppResult<T> = Result<T, AppError>;

/// Field name → human-readable problems.
pub type FieldErrors = BTreeMap<String, Vec<String>>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("Authentication required")]
    Unauthorized,
    #[error("Invalid email or password")]
    InvalidCredentials,
    #[error("{0}")]
    Forbidden(String),
    #[error("CSRF token missing or invalid")]
    CsrfInvalid,
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{message}")]
    Conflict { message: String, details: FieldErrors },
    #[error("Validation failed")]
    Validation(FieldErrors),
    #[error("This sign-in attempt has expired. Sign in again.")]
    SignInExpired,
    #[error("Outgoing email is not configured, so codes cannot be sent.")]
    EmailUnavailable,
    #[error("Too many attempts. Try again later.")]
    RateLimited { retry_after_secs: u64 },
    #[error("database error")]
    Database(#[source] sqlx::Error),
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict { message: message.into(), details: FieldErrors::new() }
    }

    /// A conflict that can be attributed to one form field.
    pub fn field_conflict(field: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        Self::Conflict { details: FieldErrors::from([(field.to_owned(), vec![message.clone()])]), message }
    }

    /// A validation failure on a single field.
    pub fn invalid(field: &str, message: impl Into<String>) -> Self {
        Self::Validation(FieldErrors::from([(field.to_owned(), vec![message.into()])]))
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized | Self::InvalidCredentials | Self::SignInExpired => StatusCode::UNAUTHORIZED,
            Self::EmailUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::Forbidden(_) | Self::CsrfInvalid => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict { .. } => StatusCode::CONFLICT,
            Self::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::Database(_) | Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::Unauthorized => "unauthorized",
            Self::InvalidCredentials => "invalid_credentials",
            Self::SignInExpired => "sign_in_expired",
            Self::EmailUnavailable => "email_unavailable",
            Self::Forbidden(_) => "forbidden",
            Self::CsrfInvalid => "csrf_invalid",
            Self::NotFound(_) => "not_found",
            Self::Conflict { .. } => "conflict",
            Self::Validation(_) => "validation_failed",
            Self::RateLimited { .. } => "rate_limited",
            Self::Database(_) | Self::Internal(_) => "internal_error",
        }
    }
}

/// Error envelope returned for every non-2xx response.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: ErrorBody,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    /// Stable machine-readable code, e.g. `validation_failed`.
    pub code: String,
    /// Human-readable summary, safe to display.
    pub message: String,
    /// Per-field problems (validation and attributable conflicts).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    #[schema(value_type = Object, additional_properties = true)]
    pub details: FieldErrors,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let code = self.code();

        // Server-side faults are logged with full detail and replaced by a
        // generic message; client faults carry their own safe message.
        let message = match &self {
            Self::Database(err) => {
                tracing::error!(error = %err, "database error");
                "An unexpected error occurred".to_owned()
            }
            Self::Internal(err) => {
                tracing::error!(error = ?err, "internal error");
                "An unexpected error occurred".to_owned()
            }
            other => other.to_string(),
        };

        let retry_after = match &self {
            Self::RateLimited { retry_after_secs } => Some(*retry_after_secs),
            _ => None,
        };
        let details = match self {
            Self::Validation(details) | Self::Conflict { details, .. } => details,
            _ => FieldErrors::new(),
        };

        let body = ErrorResponse { error: ErrorBody { code: code.to_owned(), message, details } };
        let mut response = (status, Json(body)).into_response();
        if let Some(secs) = retry_after {
            response.headers_mut().insert(RETRY_AFTER, HeaderValue::from(secs));
        }
        response
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        if matches!(err, sqlx::Error::RowNotFound) {
            return Self::NotFound("Resource");
        }
        if let Some(db) = err.as_database_error() {
            let constraint = db.constraint().unwrap_or_default();
            if db.is_unique_violation() {
                return unique_violation(constraint);
            }
            if db.is_foreign_key_violation() {
                return foreign_key_violation(constraint);
            }
            if db.is_check_violation() {
                // Request validation should catch these first; the constraint
                // is the backstop, so report it as a validation problem.
                tracing::warn!(constraint, "check constraint rejected a write");
                return Self::BadRequest("The submitted data violates a data integrity rule".into());
            }
        }
        Self::Database(err)
    }
}

fn unique_violation(constraint: &str) -> AppError {
    match constraint {
        "projects_slug_key" => AppError::field_conflict("slug", "A project with this slug already exists"),
        "environments_project_slug_key" => {
            AppError::field_conflict("name", "This project already has an environment with this name")
        }
        "branches_project_name_key" => {
            AppError::field_conflict("name", "This project already has a branch with this name")
        }
        "commits_project_hash_key" => {
            AppError::field_conflict("commit_hash", "This commit hash is already recorded for this project")
        }
        "releases_project_version_key" => {
            AppError::field_conflict("version", "This project already has a release with this version")
        }
        "tags_slug_key" => AppError::field_conflict("name", "A tag with this name already exists"),
        "users_email_key" => AppError::field_conflict("email", "This email address is already in use"),
        _ => AppError::conflict("A record with the same unique value already exists"),
    }
}

fn foreign_key_violation(constraint: &str) -> AppError {
    match constraint {
        "deployments_commit_fkey" => {
            AppError::conflict("This commit has recorded deployments. Delete them first or confirm cascading deletion.")
        }
        "deployments_environment_fkey" => AppError::conflict(
            "This environment has recorded deployments. Delete them first or confirm cascading deletion.",
        ),
        "commits_branch_fkey" => AppError::conflict(
            "This branch has recorded commits. Move or delete them first, or confirm cascading deletion.",
        ),
        _ => AppError::conflict("The operation conflicts with related records"),
    }
}

impl From<validator::ValidationErrors> for AppError {
    fn from(errors: validator::ValidationErrors) -> Self {
        let mut details = FieldErrors::new();
        collect_validation_errors(&errors, "", &mut details);
        Self::Validation(details)
    }
}

fn collect_validation_errors(errors: &validator::ValidationErrors, prefix: &str, out: &mut FieldErrors) {
    use validator::ValidationErrorsKind as Kind;

    for (field, kind) in errors.errors() {
        let path = if prefix.is_empty() { field.to_string() } else { format!("{prefix}.{field}") };
        match kind {
            Kind::Field(list) => {
                let messages = list
                    .iter()
                    .map(|e| e.message.as_ref().map(ToString::to_string).unwrap_or_else(|| default_message(&e.code)));
                out.entry(path).or_default().extend(messages);
            }
            Kind::Struct(nested) => collect_validation_errors(nested, &path, out),
            Kind::List(items) => {
                for (index, nested) in items {
                    collect_validation_errors(nested, &format!("{path}[{index}]"), out);
                }
            }
        }
    }
}

fn default_message(code: &str) -> String {
    match code {
        "length" => "Has an invalid length".into(),
        "email" => "Must be a valid email address".into(),
        "url" => "Must be a valid URL".into(),
        "range" => "Is out of range".into(),
        "required" => "Is required".into(),
        other => format!("Is invalid ({other})"),
    }
}

impl From<JsonRejection> for AppError {
    fn from(rejection: JsonRejection) -> Self {
        match rejection {
            // Well-formed JSON whose shape is wrong (missing field, wrong type…).
            JsonRejection::JsonDataError(err) => {
                Self::Validation(FieldErrors::from([("body".to_owned(), vec![clean_serde_message(&err.body_text())])]))
            }
            JsonRejection::JsonSyntaxError(_) => Self::BadRequest("Request body is not valid JSON".into()),
            JsonRejection::MissingJsonContentType(_) => {
                Self::BadRequest("Expected a request with `Content-Type: application/json`".into())
            }
            other => Self::BadRequest(other.body_text()),
        }
    }
}

impl From<QueryRejection> for AppError {
    fn from(rejection: QueryRejection) -> Self {
        Self::BadRequest(format!("Invalid query parameters: {}", clean_serde_message(&rejection.body_text())))
    }
}

impl From<PathRejection> for AppError {
    fn from(_: PathRejection) -> Self {
        // Malformed ids can never match a record.
        Self::NotFound("Resource")
    }
}

/// Strips axum's fixed prefixes so only the serde explanation remains.
fn clean_serde_message(text: &str) -> String {
    text.split_once(": ").map_or(text, |(_, tail)| tail).trim().to_owned()
}

impl From<tower_sessions::session::Error> for AppError {
    fn from(err: tower_sessions::session::Error) -> Self {
        Self::Internal(anyhow::Error::new(err).context("session store failure"))
    }
}
