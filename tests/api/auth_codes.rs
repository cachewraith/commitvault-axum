//! Emailed one-time codes: the second sign-in step for new browsers, trusted
//! browsers, and password reset.

use std::time::Duration;

use axum::http::{StatusCode, header};
use serde_json::json;
use sqlx::PgPool;

use crate::common::{ADMIN_EMAIL, ADMIN_PASSWORD, TestApp};

const NEW_PASSWORD: &str = "a brand new long passphrase";

/// Password reset emails are sent by a background task.
async fn wait_for_emails(app: &TestApp, count: usize) {
    for _ in 0..100 {
        if app.sent().len() >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("expected {count} email(s), got {}", app.sent().len());
}

/// A well-formed code that is not `code`.
fn wrong_code(code: &str) -> &'static str {
    if code == "000000" { "111111" } else { "000000" }
}

/// Pretends the latest code of `purpose` was sent long enough ago to send another.
async fn age_codes(app: &TestApp) {
    sqlx::query("UPDATE auth_codes SET created_at = created_at - interval '2 minutes'")
        .execute(&app.pool)
        .await
        .unwrap();
}

#[sqlx::test]
async fn a_new_browser_must_enter_the_emailed_code(pool: PgPool) {
    let mut app = TestApp::new(pool).await;

    let login = app.login_as(ADMIN_EMAIL, ADMIN_PASSWORD).await.assert_status(StatusCode::OK);
    assert_eq!(login.data()["status"], "code_required");
    assert_eq!(login.data()["email_hint"], "a•••@example.com");
    assert!(login.data().get("user").is_none(), "no account data before the second step");
    assert!(login.data()["expires_at"].is_string());

    let sent = app.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to, ADMIN_EMAIL);
    assert!(sent[0].subject.contains("sign-in code"));

    // Password alone is not a session.
    for path in ["/api/v1/auth/me", "/api/v1/projects"] {
        app.get(path).await.assert_status(StatusCode::UNAUTHORIZED);
    }

    let code = app.last_code();
    let wrong = app.verify_code(wrong_code(&code)).await.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(wrong.body["error"]["details"]["code"][0], "The code is incorrect or has expired");

    let malformed = app.verify_code("12ab").await.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    assert!(malformed.body["error"]["details"]["code"].is_array());

    let verified = app.verify_code(&code).await.assert_status(StatusCode::OK);
    assert_eq!(verified.data()["user"]["email"], ADMIN_EMAIL);
    let device_cookie = verified
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .find(|v| v.starts_with("cv_device="))
        .expect("device cookie is set");
    for attribute in ["HttpOnly", "SameSite=Strict", "Path=/", "Max-Age=7776000"] {
        assert!(device_cookie.contains(attribute), "{device_cookie}");
    }
    app.get("/api/v1/auth/me").await.assert_status(StatusCode::OK);

    // A code works once.
    let mut other = app.new_client();
    other.login_as(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    other.verify_code(&code).await.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test]
async fn a_trusted_browser_skips_the_code(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    app.login().await;
    assert!(app.has_cookie("cv_device"));
    app.post("/api/v1/auth/logout", json!({})).await.assert_status(StatusCode::NO_CONTENT);
    assert!(app.has_cookie("cv_device"), "signing out keeps the browser trusted");
    app.fetch_csrf().await;

    let again = app.login_as(ADMIN_EMAIL, ADMIN_PASSWORD).await.assert_status(StatusCode::OK);
    assert_eq!(again.data()["status"], "authenticated");
    assert_eq!(app.sent().len(), 1, "no second email");

    // Another browser is not trusted.
    let mut other = app.new_client();
    let login = other.login_as(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    assert_eq!(login.data()["status"], "code_required");

    // An expired trust is not trust.
    sqlx::query("UPDATE trusted_devices SET expires_at = now() - interval '1 second'")
        .execute(&app.pool)
        .await
        .unwrap();
    app.post("/api/v1/auth/logout", json!({})).await;
    app.fetch_csrf().await;
    let expired = app.login_as(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    assert_eq!(expired.data()["status"], "code_required");
}

#[sqlx::test]
async fn verifying_needs_a_pending_sign_in(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    app.fetch_csrf().await;
    let response = app.verify_code("123456").await.assert_status(StatusCode::UNAUTHORIZED);
    assert_eq!(response.error_code(), "sign_in_expired");
    app.post("/api/v1/auth/login/resend", json!({})).await.assert_status(StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn expired_codes_are_rejected(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    app.login_as(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    sqlx::query("UPDATE auth_codes SET expires_at = now() - interval '1 second'").execute(&app.pool).await.unwrap();
    let code = app.last_code();
    app.verify_code(&code).await.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    app.get("/api/v1/auth/me").await.assert_status(StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn resending_is_throttled_and_replaces_the_code(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    app.login_as(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    let first = app.last_code();

    let too_soon = app.post("/api/v1/auth/login/resend", json!({})).await.assert_status(StatusCode::TOO_MANY_REQUESTS);
    assert!(too_soon.headers.contains_key(header::RETRY_AFTER));
    assert_eq!(app.sent().len(), 1);

    age_codes(&app).await;
    let resent = app.post("/api/v1/auth/login/resend", json!({})).await.assert_status(StatusCode::OK);
    assert_eq!(resent.data()["email_hint"], "a•••@example.com");
    assert_eq!(app.sent().len(), 2);
    let second = app.last_code();

    if first != second {
        app.verify_code(&first).await.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }
    app.verify_code(&second).await.assert_status(StatusCode::OK);
}

#[sqlx::test]
async fn wrong_codes_are_rate_limited(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    app.login_as(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    let code = app.last_code();
    for _ in 0..5 {
        app.verify_code(wrong_code(&code)).await.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let blocked = app.verify_code(&code).await.assert_status(StatusCode::TOO_MANY_REQUESTS);
    assert!(blocked.headers.contains_key(header::RETRY_AFTER));
    let attempts: i32 = sqlx::query_scalar("SELECT attempts FROM auth_codes").fetch_one(&app.pool).await.unwrap();
    assert_eq!(attempts, 5, "the code itself is used up as well");
}

#[sqlx::test]
async fn password_reset_with_an_emailed_code(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    // A signed-in browser that should be signed out by the reset.
    let mut laptop = app.new_client();
    laptop.login().await;
    let emails_before = app.sent().len();

    // Unknown address: same answer, nothing sent.
    app.fetch_csrf().await;
    app.post("/api/v1/auth/password/forgot", json!({ "email": "nobody@example.com" }))
        .await
        .assert_status(StatusCode::ACCEPTED);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(app.sent().len(), emails_before);

    app.post("/api/v1/auth/password/forgot", json!({ "email": " Admin@Example.com " }))
        .await
        .assert_status(StatusCode::ACCEPTED);
    wait_for_emails(&app, emails_before + 1).await;
    let email = app.sent().last().unwrap().clone();
    assert_eq!(email.to, ADMIN_EMAIL);
    assert!(email.subject.contains("password reset"));
    let code = app.last_code();

    let reset = |code: &str, password: &str| json!({ "email": ADMIN_EMAIL, "code": code, "new_password": password });
    app.post("/api/v1/auth/password/reset", reset(wrong_code(&code), NEW_PASSWORD))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    let short = app.post("/api/v1/auth/password/reset", reset(&code, "short")).await;
    assert!(short.assert_status(StatusCode::UNPROCESSABLE_ENTITY).body["error"]["details"]["new_password"].is_array());

    app.post("/api/v1/auth/password/reset", reset(&code, NEW_PASSWORD)).await.assert_status(StatusCode::NO_CONTENT);
    // Single use.
    app.post("/api/v1/auth/password/reset", reset(&code, "yet another long passphrase"))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    // Every session is gone and the laptop is no longer trusted.
    laptop.get("/api/v1/auth/me").await.assert_status(StatusCode::UNAUTHORIZED);
    laptop.fetch_csrf().await;
    laptop.login_as(ADMIN_EMAIL, ADMIN_PASSWORD).await.assert_status(StatusCode::UNAUTHORIZED);
    let again = laptop.login_as(ADMIN_EMAIL, NEW_PASSWORD).await.assert_status(StatusCode::OK);
    assert_eq!(again.data()["status"], "code_required");

    // The browser that reset the password proved it reads the mailbox: its
    // next sign-in goes straight through, without another code.
    assert!(app.has_cookie("cv_device"));
    let emails = app.sent().len();
    app.fetch_csrf().await;
    let signed_in = app.login_as(ADMIN_EMAIL, NEW_PASSWORD).await.assert_status(StatusCode::OK);
    assert_eq!(signed_in.data()["status"], "authenticated");
    assert_eq!(app.sent().len(), emails, "no sign-in code was sent");
}

#[sqlx::test]
async fn reset_codes_are_not_login_codes(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    app.fetch_csrf().await;
    app.post("/api/v1/auth/password/forgot", json!({ "email": ADMIN_EMAIL })).await;
    wait_for_emails(&app, 1).await;
    let reset_code = app.last_code();

    app.login_as(ADMIN_EMAIL, ADMIN_PASSWORD).await;
    let login_code = app.last_code();
    if reset_code != login_code {
        app.verify_code(&reset_code).await.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }
    app.post(
        "/api/v1/auth/password/reset",
        json!({ "email": ADMIN_EMAIL, "code": login_code, "new_password": NEW_PASSWORD }),
    )
    .await
    .assert_status(if reset_code == login_code {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::UNPROCESSABLE_ENTITY
    });
}

#[sqlx::test]
async fn reset_requests_are_rate_limited_per_client(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    app.fetch_csrf().await;
    for _ in 0..5 {
        app.post("/api/v1/auth/password/forgot", json!({ "email": "nobody@example.com" }))
            .await
            .assert_status(StatusCode::ACCEPTED);
    }
    app.post("/api/v1/auth/password/forgot", json!({ "email": "nobody@example.com" }))
        .await
        .assert_status(StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test]
async fn without_email_there_is_no_code_step_and_no_reset(pool: PgPool) {
    let mut app = TestApp::without_email(pool).await;
    let options = app.get("/api/v1/auth/options").await.assert_status(StatusCode::OK);
    assert_eq!(options.data()["email_enabled"], false);

    app.fetch_csrf().await;
    let forgot = app
        .post("/api/v1/auth/password/forgot", json!({ "email": ADMIN_EMAIL }))
        .await
        .assert_status(StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(forgot.error_code(), "email_unavailable");

    let login = app.login_as(ADMIN_EMAIL, ADMIN_PASSWORD).await.assert_status(StatusCode::OK);
    assert_eq!(login.data()["status"], "authenticated");
    assert!(!app.has_cookie("cv_device"));
}

#[sqlx::test]
async fn options_report_email_when_configured(pool: PgPool) {
    let mut app = TestApp::new(pool).await;
    let options = app.get("/api/v1/auth/options").await.assert_status(StatusCode::OK);
    assert_eq!(options.data()["email_enabled"], true);
}
