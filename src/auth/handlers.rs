use axum::{Extension, Json, extract::State, http::StatusCode};
use tower_sessions::Session;

use super::{
    dto::{ChangePasswordRequest, CsrfTokenResponse, LoginRequest, SessionResponse},
    models::CurrentUser,
    service,
};
use crate::{
    AppState,
    errors::{AppResult, ErrorResponse},
    middleware::client_ip::ClientIp,
    shared::{extractors::ValidJson, response::Data},
};

/// Get a CSRF token
///
/// Returns the CSRF token bound to the current session, creating an anonymous
/// session if there is none. Required before the first state-changing request
/// (including login).
#[utoipa::path(get, path = "/auth/csrf", tag = "auth",
    responses((status = 200, body = Data<CsrfTokenResponse>)))]
pub async fn csrf(session: Session) -> AppResult<Json<Data<CsrfTokenResponse>>> {
    let csrf_token = service::csrf_token(&session).await?;
    Ok(Data::json(CsrfTokenResponse { csrf_token }))
}

/// Sign in
#[utoipa::path(post, path = "/auth/login", tag = "auth", request_body = LoginRequest,
    responses(
        (status = 200, body = Data<SessionResponse>),
        (status = 401, body = ErrorResponse, description = "Invalid credentials"),
        (status = 422, body = ErrorResponse),
        (status = 429, body = ErrorResponse, description = "Too many failed attempts; see `Retry-After`"),
    ))]
pub async fn login(
    State(state): State<AppState>,
    session: Session,
    ClientIp(client_ip): ClientIp,
    ValidJson(request): ValidJson<LoginRequest>,
) -> AppResult<Json<Data<SessionResponse>>> {
    Ok(Data::json(service::login(&state, &session, &client_ip, request).await?))
}

/// Sign out
///
/// Invalidates the server-side session and clears the cookie.
#[utoipa::path(post, path = "/auth/logout", tag = "auth", responses((status = 204)))]
pub async fn logout(session: Session) -> AppResult<StatusCode> {
    service::logout(&session).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Current session
#[utoipa::path(get, path = "/auth/me", tag = "auth",
    responses((status = 200, body = Data<SessionResponse>), (status = 401, body = ErrorResponse)))]
pub async fn me(
    State(state): State<AppState>,
    session: Session,
    Extension(current): Extension<CurrentUser>,
) -> AppResult<Json<Data<SessionResponse>>> {
    Ok(Data::json(service::current_session(&state, &session, current).await?))
}

/// Change password
///
/// Revokes every other session of the account.
#[utoipa::path(post, path = "/auth/password", tag = "auth", request_body = ChangePasswordRequest,
    responses((status = 204), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn change_password(
    State(state): State<AppState>,
    session: Session,
    Extension(current): Extension<CurrentUser>,
    ValidJson(request): ValidJson<ChangePasswordRequest>,
) -> AppResult<StatusCode> {
    service::change_password(&state, &session, current, request).await?;
    Ok(StatusCode::NO_CONTENT)
}
