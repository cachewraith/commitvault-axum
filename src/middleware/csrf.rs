//! CSRF protection for cookie-authenticated requests.
//!
//! Three independent layers:
//! 1. the session cookie is `SameSite=Strict`, so browsers do not attach it to
//!    cross-site requests at all;
//! 2. state-changing requests carrying an `Origin` (or `Sec-Fetch-Site`)
//!    header must come from a configured frontend origin;
//! 3. state-changing requests must echo the per-session token in
//!    `X-CSRF-Token` (synchroniser token pattern) — a value an attacker's page
//!    cannot read.

use axum::{
    extract::{Request, State},
    http::{Method, header::ORIGIN},
    middleware::Next,
    response::Response,
};
use subtle::ConstantTimeEq;
use tower_sessions::Session;

use crate::{
    AppState,
    auth::{CSRF_HEADER, SESSION_CSRF_TOKEN},
    errors::AppError,
};

pub async fn csrf_protect(
    State(state): State<AppState>,
    session: Session,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    if matches!(*request.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(next.run(request).await);
    }

    let headers = request.headers();
    if let Some(origin) = headers.get(ORIGIN) {
        let allowed = origin.to_str().is_ok_and(|o| state.config.frontend_origins.iter().any(|allowed| allowed == o));
        if !allowed {
            return Err(AppError::Forbidden("Cross-origin request rejected".into()));
        }
    } else if let Some(site) = headers.get("sec-fetch-site")
        && !matches!(site.as_bytes(), b"same-origin" | b"none")
    {
        return Err(AppError::Forbidden("Cross-site request rejected".into()));
    }

    let expected = session.get::<String>(SESSION_CSRF_TOKEN).await?;
    let provided = headers.get(CSRF_HEADER).map(|value| value.as_bytes());
    match (expected, provided) {
        (Some(expected), Some(provided)) if bool::from(expected.as_bytes().ct_eq(provided)) => {
            Ok(next.run(request).await)
        }
        _ => Err(AppError::CsrfInvalid),
    }
}
