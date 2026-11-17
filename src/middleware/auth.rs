use axum::{extract::Request, middleware::Next, response::Response};
use tower_sessions::Session;
use uuid::Uuid;

use crate::{
    auth::{SESSION_USER_ID, models::CurrentUser},
    errors::AppError,
};

/// Rejects requests without an authenticated session and exposes the user to
/// handlers as `Extension<CurrentUser>`.
///
/// CommitVault is single-user: every record belongs to the one administrator,
/// so authentication is the authorisation boundary.
pub async fn require_auth(session: Session, mut request: Request, next: Next) -> Result<Response, AppError> {
    let user_id = session.get::<Uuid>(SESSION_USER_ID).await?.ok_or(AppError::Unauthorized)?;
    request.extensions_mut().insert(CurrentUser { id: user_id });
    Ok(next.run(request).await)
}

/// How long a session that never signed in may live.
const ANONYMOUS_SESSION_TTL: time::Duration = time::Duration::hours(1);

/// Gives anonymous sessions (created by `GET /auth/csrf` for the login form)
/// a short lifetime, so unauthenticated traffic cannot pile up week-long
/// session rows. Authenticated sessions keep the configured idle timeout.
///
/// Runs after the handler so that a successful login is already reflected.
pub async fn limit_anonymous_sessions(session: Session, request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    if !session.is_empty().await && matches!(session.get::<Uuid>(SESSION_USER_ID).await, Ok(None)) {
        session.set_expiry(Some(tower_sessions::Expiry::OnInactivity(ANONYMOUS_SESSION_TTL)));
    }
    response
}
