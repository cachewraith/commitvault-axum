use tower_sessions::Session;

use super::{
    SESSION_CSRF_TOKEN, SESSION_USER_ID,
    dto::{ChangePasswordRequest, LoginRequest, SessionResponse},
    models::CurrentUser,
    password,
    rate_limit::{MAX_FAILURES_PER_EMAIL, MAX_FAILURES_PER_IP},
    repository,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
};

/// 256 bits from the thread-local CSPRNG, hex encoded.
fn new_csrf_token() -> String {
    rand::random::<[u8; 32]>().iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Returns the session's CSRF token, creating one (and thereby the session)
/// on first use.
pub async fn csrf_token(session: &Session) -> AppResult<String> {
    if let Some(token) = session.get::<String>(SESSION_CSRF_TOKEN).await? {
        return Ok(token);
    }
    let token = new_csrf_token();
    session.insert(SESSION_CSRF_TOKEN, &token).await?;
    Ok(token)
}

pub async fn login(
    state: &AppState,
    session: &Session,
    client_ip: &str,
    request: LoginRequest,
) -> AppResult<SessionResponse> {
    let email = request.email.to_lowercase();
    let ip_key = format!("ip:{client_ip}");
    let email_key = format!("email:{email}");

    let limiter = &state.login_limiter;
    let blocked = limiter.check(&ip_key, MAX_FAILURES_PER_IP).and(limiter.check(&email_key, MAX_FAILURES_PER_EMAIL));
    if let Err(retry_after_secs) = blocked {
        tracing::warn!(client_ip, "login rate limit hit");
        return Err(AppError::RateLimited { retry_after_secs });
    }

    let user = repository::find_by_email(&state.db, &email).await?;
    // Always run one Argon2 verification so response time does not reveal
    // whether the account exists.
    let stored_hash = user.as_ref().map_or_else(|| password::dummy_hash().to_owned(), |u| u.password_hash.clone());
    let password_matches = password::verify(request.password, stored_hash).await?;

    let Some(user) = user.filter(|_| password_matches) else {
        limiter.record_failure(&ip_key);
        limiter.record_failure(&email_key);
        tracing::warn!(client_ip, "failed login attempt");
        return Err(AppError::InvalidCredentials);
    };
    limiter.reset(&ip_key);
    limiter.reset(&email_key);

    // New session id on privilege change (prevents session fixation) and a
    // fresh CSRF token bound to the authenticated session.
    session.cycle_id().await?;
    let csrf_token = new_csrf_token();
    session.insert(SESSION_USER_ID, user.id).await?;
    session.insert(SESSION_CSRF_TOKEN, &csrf_token).await?;

    tracing::info!(user_id = %user.id, "user signed in");
    Ok(SessionResponse { user: user.into(), csrf_token })
}

pub async fn current_session(state: &AppState, session: &Session, current: CurrentUser) -> AppResult<SessionResponse> {
    // The account may have been removed while the session was still alive.
    let Some(user) = repository::find_by_id(&state.db, current.id).await? else {
        session.flush().await?;
        return Err(AppError::Unauthorized);
    };
    Ok(SessionResponse { user: user.into(), csrf_token: csrf_token(session).await? })
}

/// Deletes the session server-side and clears the cookie.
pub async fn logout(session: &Session) -> AppResult<()> {
    session.flush().await?;
    Ok(())
}

pub async fn change_password(
    state: &AppState,
    session: &Session,
    current: CurrentUser,
    request: ChangePasswordRequest,
) -> AppResult<()> {
    let user = repository::find_by_id(&state.db, current.id).await?.ok_or(AppError::Unauthorized)?;
    if !password::verify(request.current_password.clone(), user.password_hash).await? {
        return Err(AppError::invalid("current_password", "Current password is incorrect"));
    }
    if request.new_password == request.current_password {
        return Err(AppError::invalid("new_password", "Choose a password different from the current one"));
    }

    let hash = password::hash(request.new_password).await?;
    repository::update_password(&state.db, user.id, &hash).await?;

    // Sign out every other device, and move this one to a fresh session id.
    session.cycle_id().await?;
    session.save().await?;
    let keep = session.id().map(|id| id.to_string());
    let removed = repository::delete_other_sessions(&state.db, user.id, keep.as_deref()).await?;
    tracing::info!(user_id = %user.id, removed, "password changed; other sessions revoked");
    Ok(())
}
