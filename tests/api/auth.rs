//! Authentication, session, CSRF, rate limiting and security-header behaviour.

use crate::common::{ADMIN_EMAIL, ADMIN_PASSWORD, TestApp, test_config};
use axum::http::{Method, StatusCode, header};
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test]
async fn protected_endpoints_require_a_session(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    for path in ["/api/v1/auth/me", "/api/v1/projects", "/api/v1/dashboard", "/api/v1/tags", "/api/v1/search?q=abc"] {
        let response = app.get(path).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(response.error_code(), "unauthorized", "{path}");
    }
}

#[sqlx::test]
async fn login_me_logout_flow(pool: PgPool) {
    // Without outgoing email the password alone signs in.
    let mut app = TestApp::without_email(pool).await;

    let login = app.login_as(ADMIN_EMAIL, ADMIN_PASSWORD).await.assert_status(StatusCode::OK);
    assert_eq!(login.data()["status"], "authenticated");
    assert_eq!(login.data()["user"]["email"], ADMIN_EMAIL);
    assert!(login.data()["user"].get("password_hash").is_none(), "hash must never be serialised");
    assert_eq!(login.data()["csrf_token"].as_str().unwrap().len(), 64);

    let me = app.get("/api/v1/auth/me").await.assert_status(StatusCode::OK);
    assert_eq!(me.data()["user"]["email"], ADMIN_EMAIL);

    let stolen_cookie = app.session_cookie();
    app.post("/api/v1/auth/logout", json!({})).await.assert_status(StatusCode::NO_CONTENT);
    app.get("/api/v1/auth/me").await.assert_status(StatusCode::UNAUTHORIZED);

    // Logout invalidates the session server-side: replaying the old cookie fails.
    app.set_session_cookie(stolen_cookie);
    app.get("/api/v1/auth/me").await.assert_status(StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn email_is_case_insensitive_and_trimmed(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    app.login_as("  Admin@Example.COM ", ADMIN_PASSWORD).await.assert_status(StatusCode::OK);
}

#[sqlx::test]
async fn wrong_credentials_are_rejected_without_revealing_which_part(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    let wrong_password = app.login_as(ADMIN_EMAIL, "not the password").await;
    let unknown_user = app.login_as("nobody@example.com", ADMIN_PASSWORD).await;
    for response in [&wrong_password, &unknown_user] {
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
        assert_eq!(response.error_code(), "invalid_credentials");
    }
    assert_eq!(wrong_password.body, unknown_user.body, "responses must be indistinguishable");
    app.get("/api/v1/auth/me").await.assert_status(StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn login_validates_input(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    let response = app.login_as("", "").await.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.error_code(), "validation_failed");
    assert!(response.body["error"]["details"]["email"].is_array());
    assert!(response.body["error"]["details"]["password"].is_array());

    app.send(Method::POST, "/api/v1/auth/login", Some(json!({ "email": "a@b.c" })), &[])
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test]
async fn session_cookie_is_http_only_and_same_site_strict(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    app.fetch_csrf().await;
    let response = app.login_as(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    let cookie = response.headers.get(header::SET_COOKIE).unwrap().to_str().unwrap().to_owned();
    assert!(cookie.starts_with("cv_session="), "{cookie}");
    assert!(cookie.contains("HttpOnly"), "{cookie}");
    assert!(cookie.contains("SameSite=Strict"), "{cookie}");
    assert!(cookie.contains("Path=/"), "{cookie}");
    assert!(!cookie.contains("Secure"), "plain-HTTP development config");
}

#[sqlx::test]
async fn secure_cookie_and_hsts_when_cookie_secure_is_enabled(pool: PgPool) {
    let config = test_config(&[("COOKIE_SECURE", "true")]);
    let mut app = TestApp::with_config(pool, config).await;
    let response = app.get("/api/v1/auth/csrf").await;
    let cookie = response.headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    assert!(cookie.starts_with("__Host-cv_session="), "{cookie}");
    assert!(cookie.contains("Secure"), "{cookie}");
    assert!(response.headers.contains_key(header::STRICT_TRANSPORT_SECURITY));
}

#[sqlx::test]
async fn session_id_changes_on_login(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    app.fetch_csrf().await;
    let anonymous_cookie = app.session_cookie().unwrap().to_owned();
    let anonymous_token = app.csrf_token.clone().unwrap();

    app.login().await;
    assert_ne!(app.session_cookie().unwrap(), anonymous_cookie, "session fixation: id must rotate");
    assert_ne!(app.csrf_token.clone().unwrap(), anonymous_token, "CSRF token must rotate");

    // The pre-login session id is no longer valid for anything.
    let mut attacker = app.new_client();
    attacker.set_session_cookie(Some(anonymous_cookie));
    attacker.get("/api/v1/auth/me").await.assert_status(StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn tampered_session_cookie_is_ignored(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let cookie = app.session_cookie().unwrap().to_owned();
    let mut forged = cookie.clone();
    let last = forged.pop().unwrap();
    forged.push(if last == 'A' { 'B' } else { 'A' });
    app.set_session_cookie(Some(forged));
    app.get("/api/v1/auth/me").await.assert_status(StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn state_changing_requests_require_the_csrf_token(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let body = json!({ "name": "urgent" });

    let missing = app.send(Method::POST, "/api/v1/tags", Some(body.clone()), &[("x-csrf-token", "")]).await;
    assert_eq!(missing.status, StatusCode::FORBIDDEN);
    assert_eq!(missing.error_code(), "csrf_invalid");

    let wrong = app.send(Method::POST, "/api/v1/tags", Some(body.clone()), &[("x-csrf-token", &"0".repeat(64))]).await;
    assert_eq!(wrong.error_code(), "csrf_invalid");

    // A token from a different session is useless.
    let mut other = app.new_client();
    let foreign_token = other.fetch_csrf().await;
    let foreign = app.send(Method::POST, "/api/v1/tags", Some(body.clone()), &[("x-csrf-token", &foreign_token)]).await;
    assert_eq!(foreign.error_code(), "csrf_invalid");

    app.post("/api/v1/tags", body).await.assert_status(StatusCode::CREATED);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM tags").fetch_one(&app.pool).await.unwrap();
    assert_eq!(count, 1, "only the request with the right token wrote anything");
}

#[sqlx::test]
async fn login_itself_requires_a_csrf_token(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    let body = json!({ "email": ADMIN_EMAIL, "password": ADMIN_PASSWORD });
    let response = app.send(Method::POST, "/api/v1/auth/login", Some(body), &[]).await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert_eq!(response.error_code(), "csrf_invalid");
}

#[sqlx::test]
async fn cross_origin_state_changes_are_rejected(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let body = json!({ "name": "urgent" });

    let evil = app.send(Method::POST, "/api/v1/tags", Some(body.clone()), &[("origin", "https://evil.example")]).await;
    assert_eq!(evil.status, StatusCode::FORBIDDEN);
    assert_eq!(evil.error_code(), "forbidden");

    let cross_site = app
        .send(Method::POST, "/api/v1/tags", Some(body.clone()), &[("origin", ""), ("sec-fetch-site", "cross-site")])
        .await;
    assert_eq!(cross_site.status, StatusCode::FORBIDDEN);

    // Non-browser clients send neither header and are judged on the token alone.
    app.send(Method::POST, "/api/v1/tags", Some(body), &[("origin", "")]).await.assert_status(StatusCode::CREATED);
}

#[sqlx::test]
async fn cors_allows_only_the_configured_origin(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    let preflight = |origin: &'static str| {
        [
            ("origin", origin),
            ("access-control-request-method", "POST"),
            ("access-control-request-headers", "x-csrf-token"),
        ]
    };

    let allowed = app.send(Method::OPTIONS, "/api/v1/tags", None, &preflight(crate::common::ORIGIN)).await;
    assert_eq!(allowed.headers.get("access-control-allow-origin").unwrap(), crate::common::ORIGIN);
    assert_eq!(allowed.headers.get("access-control-allow-credentials").unwrap(), "true");

    let denied = app.send(Method::OPTIONS, "/api/v1/tags", None, &preflight("https://evil.example")).await;
    assert!(denied.headers.get("access-control-allow-origin").is_none());
}

#[sqlx::test]
async fn repeated_failed_logins_are_rate_limited(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    for _ in 0..5 {
        app.login_as(ADMIN_EMAIL, "wrong password").await.assert_status(StatusCode::UNAUTHORIZED);
    }
    let blocked = app.login_as(ADMIN_EMAIL, "wrong password").await;
    assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(blocked.error_code(), "rate_limited");
    let retry_after: u64 = blocked.headers.get(header::RETRY_AFTER).unwrap().to_str().unwrap().parse().unwrap();
    assert!((1..=900).contains(&retry_after));

    // Even the correct password is refused while the limit is active.
    app.login_as(ADMIN_EMAIL, ADMIN_PASSWORD).await.assert_status(StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test]
async fn change_password_rotates_credentials_and_revokes_other_sessions(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    let mut other_device = app.new_client();
    other_device.login().await;

    let wrong = app
        .post("/api/v1/auth/password", json!({ "current_password": "nope", "new_password": "a brand new passphrase" }))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(wrong.body["error"]["details"]["current_password"].is_array());

    let short = app
        .post("/api/v1/auth/password", json!({ "current_password": ADMIN_PASSWORD, "new_password": "short" }))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(short.body["error"]["details"]["new_password"].is_array());

    app.post(
        "/api/v1/auth/password",
        json!({ "current_password": ADMIN_PASSWORD, "new_password": "a brand new passphrase" }),
    )
    .await
    .assert_status(StatusCode::NO_CONTENT);

    app.get("/api/v1/auth/me").await.assert_status(StatusCode::OK);
    other_device.get("/api/v1/auth/me").await.assert_status(StatusCode::UNAUTHORIZED);

    let mut fresh = app.new_client();
    fresh.login_as(ADMIN_EMAIL, ADMIN_PASSWORD).await.assert_status(StatusCode::UNAUTHORIZED);
    fresh.login_as(ADMIN_EMAIL, "a brand new passphrase").await.assert_status(StatusCode::OK);
}

#[sqlx::test]
async fn passwords_are_stored_as_argon2id_hashes(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users").fetch_one(&app.pool).await.unwrap();
    assert!(hash.starts_with("$argon2id$"));
    assert!(!hash.contains(ADMIN_PASSWORD));
}

#[sqlx::test]
async fn bootstrap_creates_exactly_one_admin_and_never_overwrites(pool: PgPool) {
    let app = TestApp::new(pool).await;
    let changed = test_config(&[
        ("BOOTSTRAP_ADMIN_EMAIL", "other@example.com"),
        ("BOOTSTRAP_ADMIN_PASSWORD", "another long password"),
    ]);
    commitvault::auth::bootstrap::ensure_admin(&app.pool, &changed).await.unwrap();

    let emails: Vec<String> = sqlx::query_scalar("SELECT email FROM users").fetch_all(&app.pool).await.unwrap();
    assert_eq!(emails, vec![ADMIN_EMAIL.to_owned()]);
}

#[sqlx::test]
async fn there_is_no_registration_endpoint(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    app.fetch_csrf().await;
    for path in ["/api/v1/auth/register", "/api/v1/auth/signup", "/api/v1/users"] {
        let response = app.post(path, json!({ "email": "x@example.com", "password": "a long enough password" })).await;
        assert!(
            matches!(response.status, StatusCode::NOT_FOUND | StatusCode::UNAUTHORIZED),
            "{path}: {}",
            response.status
        );
    }
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users").fetch_one(&app.pool).await.unwrap();
    assert_eq!(users, 1);
}

#[sqlx::test]
async fn responses_carry_security_headers(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    let response = app.get("/api/v1/health").await.assert_status(StatusCode::OK);
    assert_eq!(response.data()["status"], "ok");
    let header = |name: &str| response.headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
    assert_eq!(header("x-content-type-options"), "nosniff");
    assert_eq!(header("x-frame-options"), "DENY");
    assert_eq!(header("referrer-policy"), "no-referrer");
    assert!(header("content-security-policy").contains("default-src 'none'"));
    assert_eq!(header("cache-control"), "no-store");
    assert!(!header("x-request-id").is_empty());
}

#[sqlx::test]
async fn errors_use_the_standard_envelope_and_hide_internals(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;

    let not_found = app.get("/api/v1/does-not-exist").await.assert_status(StatusCode::NOT_FOUND);
    assert_eq!(not_found.error_code(), "not_found");

    let bad_id = app.get("/api/v1/tags/not-a-uuid").await.assert_status(StatusCode::NOT_FOUND);
    assert_eq!(bad_id.error_code(), "not_found");

    let malformed = app
        .send(Method::POST, "/api/v1/tags", Some(json!("just a string")), &[])
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    let text = malformed.body.to_string();
    assert!(!text.contains("sqlx") && !text.contains("src/"), "no internals in: {text}");

    let bad_query = app.get("/api/v1/tags?page=abc").await.assert_status(StatusCode::BAD_REQUEST);
    assert_eq!(bad_query.error_code(), "bad_request");
}

#[sqlx::test]
async fn anonymous_sessions_are_short_lived_and_authenticated_ones_use_the_idle_timeout(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    let lifetime_minutes = |pool: PgPool| async move {
        let seconds: f64 =
            sqlx::query_scalar("SELECT extract(epoch FROM (max(expires_at) - now()))::float8 FROM sessions")
                .fetch_one(&pool)
                .await
                .unwrap();
        (seconds / 60.0).round() as i64
    };

    app.get("/api/v1/health").await;
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions").fetch_one(&app.pool).await.unwrap();
    assert_eq!(rows, 0, "requests that never touch the session do not create one");

    app.fetch_csrf().await;
    assert_eq!(lifetime_minutes(app.pool.clone()).await, 60, "anonymous session: one hour");

    app.login().await;
    assert_eq!(lifetime_minutes(app.pool.clone()).await, 7 * 24 * 60, "signed in: SESSION_TTL_HOURS");
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions").fetch_one(&app.pool).await.unwrap();
    assert_eq!(rows, 1, "the anonymous session was replaced, not kept alongside");
}

#[sqlx::test]
async fn expired_sessions_stop_working_and_are_purged(pool: PgPool) {
    let mut app = TestApp::signed_in(pool).await;
    sqlx::query("UPDATE sessions SET expires_at = now() - interval '1 second'").execute(&app.pool).await.unwrap();
    app.get("/api/v1/auth/me").await.assert_status(StatusCode::UNAUTHORIZED);

    let store = commitvault::auth::session_store::PgSessionStore::new(app.pool.clone());
    assert_eq!(store.delete_expired().await.unwrap(), 1);
}

#[sqlx::test]
async fn unchanged_sessions_are_not_rewritten_on_every_request(pool: PgPool) {
    let mut app = TestApp::signed_in(pool.clone()).await;
    let expiry = || async {
        sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>("SELECT expires_at FROM sessions WHERE data ? 'user_id'")
            .fetch_one(&pool)
            .await
            .unwrap()
    };

    // Stored expiry slightly behind the sliding expiry: a read leaves the row alone.
    sqlx::query("UPDATE sessions SET expires_at = now() + interval '168 hours' - interval '1 minute'")
        .execute(&app.pool)
        .await
        .unwrap();
    let before = expiry().await;
    app.get("/api/v1/auth/me").await.assert_status(StatusCode::OK);
    assert_eq!(expiry().await, before, "an unchanged session must not be written");

    // Far enough behind: the expiry is extended.
    sqlx::query("UPDATE sessions SET expires_at = now() + interval '1 hour'").execute(&app.pool).await.unwrap();
    let before = expiry().await;
    app.get("/api/v1/auth/me").await.assert_status(StatusCode::OK);
    assert!(expiry().await > before + chrono::Duration::hours(100), "sliding expiry still slides");
}
