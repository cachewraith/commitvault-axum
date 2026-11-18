use chrono::{DateTime, Utc};
use tower_sessions::Session;
use uuid::Uuid;

use super::{
    SESSION_CSRF_TOKEN, SESSION_PENDING_USER_ID, SESSION_USER_ID,
    codes::{self, CodePurpose},
    device,
    dto::{
        AuthOptionsResponse, ChangePasswordRequest, CodeChallenge, ForgotPasswordRequest, LoginRequest, LoginResponse,
        ResetPasswordRequest, SessionResponse, VerifyLoginCodeRequest,
    },
    models::{CurrentUser, User},
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

pub fn options(state: &AppState) -> AuthOptionsResponse {
    AuthOptionsResponse { email_enabled: state.mailer.is_enabled() }
}

/// Checks the password. Browsers that are not trusted yet then get a pending
/// (unauthenticated) session and an emailed code instead of a signed-in one.
pub async fn login(
    state: &AppState,
    session: &Session,
    client_ip: &str,
    device_token: Option<String>,
    request: LoginRequest,
) -> AppResult<LoginResponse> {
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

    let trusted = match &device_token {
        Some(token) => repository::touch_trusted_device(&state.db, user.id, &device::hash_token(token)).await?,
        None => false,
    };
    if trusted || !state.mailer.is_enabled() {
        return Ok(LoginResponse::Authenticated(complete_login(session, user).await?));
    }

    // Second step. A fresh session id here too: the pending state must not be
    // attached to an id the client had before the password was checked.
    session.cycle_id().await?;
    session.remove::<Uuid>(SESSION_USER_ID).await?;
    session.insert(SESSION_PENDING_USER_ID, user.id).await?;
    let csrf_token = new_csrf_token();
    session.insert(SESSION_CSRF_TOKEN, &csrf_token).await?;

    let (expires_at, resend_available_at) = send_code(state, &user, CodePurpose::Login).await?;
    tracing::info!(user_id = %user.id, "password accepted; sign-in code sent");
    Ok(LoginResponse::CodeRequired(CodeChallenge {
        csrf_token,
        email_hint: codes::email_hint(&user.email),
        expires_at,
        resend_available_at,
    }))
}

/// Completes a pending sign-in with the emailed code. Returns the session and
/// a new trusted-device token for the browser.
pub async fn verify_login_code(
    state: &AppState,
    session: &Session,
    client_ip: &str,
    request: VerifyLoginCodeRequest,
) -> AppResult<(SessionResponse, String)> {
    let user = pending_user(state, session).await?;
    let ip_key = format!("code-ip:{client_ip}");
    let user_key = format!("code-user:{}", user.id);
    check_code_limits(state, &ip_key, &user_key)?;

    if !check_code(state, &user, CodePurpose::Login, &request.code).await? {
        state.login_limiter.record_failure(&ip_key);
        state.login_limiter.record_failure(&user_key);
        tracing::warn!(client_ip, "wrong sign-in code");
        return Err(AppError::invalid("code", "The code is incorrect or has expired"));
    }
    state.login_limiter.reset(&ip_key);
    state.login_limiter.reset(&user_key);

    let token = device::new_token();
    let expires_at = Utc::now() + device::TRUSTED_DEVICE_TTL;
    repository::insert_trusted_device(&state.db, user.id, &device::hash_token(&token), expires_at).await?;
    Ok((complete_login(session, user).await?, token))
}

/// Sends a new code for the pending sign-in (the previous one stops working).
pub async fn resend_login_code(state: &AppState, session: &Session) -> AppResult<CodeChallenge> {
    let user = pending_user(state, session).await?;
    if let Some(retry_after_secs) = resend_wait(state, user.id, CodePurpose::Login).await? {
        return Err(AppError::RateLimited { retry_after_secs });
    }
    let (expires_at, resend_available_at) = send_code(state, &user, CodePurpose::Login).await?;
    Ok(CodeChallenge {
        csrf_token: csrf_token(session).await?,
        email_hint: codes::email_hint(&user.email),
        expires_at,
        resend_available_at,
    })
}

/// Starts a password reset. The outcome is identical whether or not the
/// address belongs to the account, and the lookup and delivery run in the
/// background so response time does not reveal it either.
pub fn forgot_password(state: &AppState, client_ip: &str, request: ForgotPasswordRequest) -> AppResult<()> {
    if !state.mailer.is_enabled() {
        return Err(AppError::EmailUnavailable);
    }
    // Every request counts, successful or not: this endpoint sends email.
    let ip_key = format!("forgot-ip:{client_ip}");
    if let Err(retry_after_secs) = state.login_limiter.check(&ip_key, MAX_FAILURES_PER_IP) {
        tracing::warn!(client_ip, "password reset request rate limit hit");
        return Err(AppError::RateLimited { retry_after_secs });
    }
    state.login_limiter.record_failure(&ip_key);

    let state = state.clone();
    let email = request.email.to_lowercase();
    tokio::spawn(async move {
        let result: AppResult<()> = async {
            let Some(user) = repository::find_by_email(&state.db, &email).await? else {
                return Ok(());
            };
            // Throttled silently: the response must not differ.
            if resend_wait(&state, user.id, CodePurpose::PasswordReset).await?.is_some() {
                return Ok(());
            }
            send_code(&state, &user, CodePurpose::PasswordReset).await?;
            tracing::info!(user_id = %user.id, "password reset code sent");
            Ok(())
        }
        .await;
        if let Err(err) = result {
            tracing::error!(error = %err, "could not send a password reset code");
        }
    });
    Ok(())
}

/// Sets a new password with an emailed reset code, then signs out every
/// session and forgets every trusted browser.
pub async fn reset_password(state: &AppState, client_ip: &str, request: ResetPasswordRequest) -> AppResult<()> {
    let email = request.email.to_lowercase();
    let ip_key = format!("reset-ip:{client_ip}");
    let email_key = format!("reset-email:{email}");
    let limiter = &state.login_limiter;
    let blocked = limiter.check(&ip_key, MAX_FAILURES_PER_IP).and(limiter.check(&email_key, MAX_FAILURES_PER_EMAIL));
    if let Err(retry_after_secs) = blocked {
        tracing::warn!(client_ip, "password reset rate limit hit");
        return Err(AppError::RateLimited { retry_after_secs });
    }

    let user = repository::find_by_email(&state.db, &email).await?;
    let valid = match &user {
        Some(user) => check_code(state, user, CodePurpose::PasswordReset, &request.code).await?,
        None => false,
    };
    let Some(user) = user.filter(|_| valid) else {
        limiter.record_failure(&ip_key);
        limiter.record_failure(&email_key);
        tracing::warn!(client_ip, "wrong password reset code");
        return Err(AppError::invalid("code", "The code is incorrect or has expired"));
    };
    limiter.reset(&ip_key);
    limiter.reset(&email_key);

    let hash = password::hash(request.new_password).await?;
    repository::update_password(&state.db, user.id, &hash).await?;
    let sessions = repository::delete_all_sessions(&state.db, user.id).await?;
    let devices = repository::delete_trusted_devices(&state.db, user.id).await?;
    tracing::info!(user_id = %user.id, sessions, devices, "password reset; sessions and trusted browsers revoked");
    Ok(())
}

/// Turns the session into a signed-in one.
async fn complete_login(session: &Session, user: User) -> AppResult<SessionResponse> {
    // New session id on privilege change (prevents session fixation) and a
    // fresh CSRF token bound to the authenticated session.
    session.cycle_id().await?;
    session.remove::<Uuid>(SESSION_PENDING_USER_ID).await?;
    let csrf_token = new_csrf_token();
    session.insert(SESSION_USER_ID, user.id).await?;
    session.insert(SESSION_CSRF_TOKEN, &csrf_token).await?;

    tracing::info!(user_id = %user.id, "user signed in");
    Ok(SessionResponse { user: user.into(), csrf_token })
}

async fn pending_user(state: &AppState, session: &Session) -> AppResult<User> {
    let user_id = session.get::<Uuid>(SESSION_PENDING_USER_ID).await?.ok_or(AppError::SignInExpired)?;
    repository::find_by_id(&state.db, user_id).await?.ok_or(AppError::SignInExpired)
}

fn check_code_limits(state: &AppState, ip_key: &str, user_key: &str) -> AppResult<()> {
    let limiter = &state.login_limiter;
    limiter
        .check(ip_key, MAX_FAILURES_PER_IP)
        .and(limiter.check(user_key, MAX_FAILURES_PER_EMAIL))
        .map_err(|retry_after_secs| AppError::RateLimited { retry_after_secs })
}

/// Issues, stores and emails a new code. Returns (expires_at, resend_available_at).
async fn send_code(state: &AppState, user: &User, purpose: CodePurpose) -> AppResult<(DateTime<Utc>, DateTime<Utc>)> {
    let code = codes::generate();
    let now = Utc::now();
    let expires_at = now + codes::CODE_TTL;
    let hash = codes::hash(state.config.session_secret.expose(), purpose, user.id, &code);
    repository::replace_code(&state.db, user.id, purpose.as_str(), &hash, expires_at).await?;
    state.mailer.send(codes::email(purpose, &user.email, &code)).await?;
    Ok((expires_at, now + codes::RESEND_INTERVAL))
}

/// Seconds until another code of this purpose may be sent, if it is too soon.
async fn resend_wait(state: &AppState, user_id: Uuid, purpose: CodePurpose) -> AppResult<Option<u64>> {
    let Some(sent_at) = repository::last_code_sent_at(&state.db, user_id, purpose.as_str()).await? else {
        return Ok(None);
    };
    let next = sent_at + codes::RESEND_INTERVAL;
    let wait = (next - Utc::now()).num_seconds();
    Ok((wait > 0).then_some(wait as u64))
}

/// Compares `code` with the active code; a wrong guess uses up an attempt,
/// a right one consumes the code.
async fn check_code(state: &AppState, user: &User, purpose: CodePurpose, code: &str) -> AppResult<bool> {
    let Some(active) =
        repository::find_active_code(&state.db, user.id, purpose.as_str(), codes::MAX_CODE_ATTEMPTS).await?
    else {
        return Ok(false);
    };
    let candidate = codes::hash(state.config.session_secret.expose(), purpose, user.id, code);
    if !codes::matches(&active.code_hash, &candidate) {
        repository::record_code_attempt(&state.db, active.id).await?;
        return Ok(false);
    }
    Ok(repository::consume_code(&state.db, active.id).await?)
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
