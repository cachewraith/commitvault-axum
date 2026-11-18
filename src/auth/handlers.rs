use axum::{
    Extension, Json,
    extract::State,
    http::{HeaderMap, StatusCode, header::SET_COOKIE},
    response::AppendHeaders,
};
use tower_sessions::Session;

use super::{
    device,
    dto::{
        AuthOptionsResponse, ChangePasswordRequest, CodeChallenge, CsrfTokenResponse, ForgotPasswordRequest,
        LoginRequest, LoginResponse, ResetPasswordRequest, SessionResponse, VerifyLoginCodeRequest,
    },
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

/// Sign-in options
///
/// Tells the login screen whether email-based features are available.
#[utoipa::path(get, path = "/auth/options", tag = "auth", responses((status = 200, body = Data<AuthOptionsResponse>)))]
pub async fn options(State(state): State<AppState>) -> Json<Data<AuthOptionsResponse>> {
    Data::json(service::options(&state))
}

/// Sign in
///
/// With outgoing email configured, a browser that has not completed the code
/// step before gets `status: "code_required"`: a six-digit code was emailed
/// and must be sent to `POST /auth/login/verify`. Browsers that did complete
/// it within the last 90 days (device cookie) are signed in directly.
#[utoipa::path(post, path = "/auth/login", tag = "auth", request_body = LoginRequest,
    responses(
        (status = 200, body = Data<LoginResponse>),
        (status = 401, body = ErrorResponse, description = "Invalid credentials"),
        (status = 422, body = ErrorResponse),
        (status = 429, body = ErrorResponse, description = "Too many failed attempts; see `Retry-After`"),
    ))]
pub async fn login(
    State(state): State<AppState>,
    session: Session,
    ClientIp(client_ip): ClientIp,
    headers: HeaderMap,
    ValidJson(request): ValidJson<LoginRequest>,
) -> AppResult<Json<Data<LoginResponse>>> {
    let device_token = device::token_from_headers(&headers, state.config.device_cookie_name());
    Ok(Data::json(service::login(&state, &session, &client_ip, device_token, request).await?))
}

/// Enter the emailed sign-in code
///
/// Completes a sign-in that answered `code_required`. On success the browser
/// is remembered (device cookie) and will not be asked again for 90 days.
#[utoipa::path(post, path = "/auth/login/verify", tag = "auth", request_body = VerifyLoginCodeRequest,
    responses(
        (status = 200, body = Data<SessionResponse>),
        (status = 401, body = ErrorResponse, description = "`sign_in_expired`: no pending sign-in; start again"),
        (status = 422, body = ErrorResponse, description = "Wrong or expired code"),
        (status = 429, body = ErrorResponse, description = "Too many wrong codes; see `Retry-After`"),
    ))]
pub async fn verify_login_code(
    State(state): State<AppState>,
    session: Session,
    ClientIp(client_ip): ClientIp,
    ValidJson(request): ValidJson<VerifyLoginCodeRequest>,
) -> AppResult<(AppendHeaders<[(axum::http::HeaderName, axum::http::HeaderValue); 1]>, Json<Data<SessionResponse>>)> {
    let (response, device_token) = service::verify_login_code(&state, &session, &client_ip, request).await?;
    let cookie = device::set_cookie(&state.config, &device_token);
    Ok((AppendHeaders([(SET_COOKIE, cookie)]), Data::json(response)))
}

/// Send a new sign-in code
///
/// The previous code stops working. At most one code per minute.
#[utoipa::path(post, path = "/auth/login/resend", tag = "auth",
    responses(
        (status = 200, body = Data<CodeChallenge>),
        (status = 401, body = ErrorResponse, description = "`sign_in_expired`: no pending sign-in; start again"),
        (status = 429, body = ErrorResponse, description = "Too soon; see `Retry-After`"),
    ))]
pub async fn resend_login_code(
    State(state): State<AppState>,
    session: Session,
) -> AppResult<Json<Data<CodeChallenge>>> {
    Ok(Data::json(service::resend_login_code(&state, &session).await?))
}

/// Request a password reset code
///
/// Always answers 202, whether or not the address belongs to the account; if
/// it does, a code is emailed (at most one per minute).
#[utoipa::path(post, path = "/auth/password/forgot", tag = "auth", request_body = ForgotPasswordRequest,
    responses(
        (status = 202),
        (status = 422, body = ErrorResponse),
        (status = 429, body = ErrorResponse, description = "Too many requests; see `Retry-After`"),
        (status = 503, body = ErrorResponse, description = "`email_unavailable`: outgoing email is not configured"),
    ))]
pub async fn forgot_password(
    State(state): State<AppState>,
    ClientIp(client_ip): ClientIp,
    ValidJson(request): ValidJson<ForgotPasswordRequest>,
) -> AppResult<StatusCode> {
    service::forgot_password(&state, &client_ip, request)?;
    Ok(StatusCode::ACCEPTED)
}

/// Reset the password with an emailed code
///
/// Signs out every session and forgets every other trusted browser. The
/// browser that made the request is trusted (device cookie): having the
/// emailed code is the same proof as the sign-in code, so its next sign-in
/// does not ask for one.
#[utoipa::path(post, path = "/auth/password/reset", tag = "auth", request_body = ResetPasswordRequest,
    responses(
        (status = 204),
        (status = 422, body = ErrorResponse, description = "Invalid fields, or wrong or expired code"),
        (status = 429, body = ErrorResponse, description = "Too many wrong codes; see `Retry-After`"),
    ))]
pub async fn reset_password(
    State(state): State<AppState>,
    ClientIp(client_ip): ClientIp,
    ValidJson(request): ValidJson<ResetPasswordRequest>,
) -> AppResult<(StatusCode, AppendHeaders<[(axum::http::HeaderName, axum::http::HeaderValue); 1]>)> {
    let device_token = service::reset_password(&state, &client_ip, request).await?;
    let cookie = device::set_cookie(&state.config, &device_token);
    Ok((StatusCode::NO_CONTENT, AppendHeaders([(SET_COOKIE, cookie)])))
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
