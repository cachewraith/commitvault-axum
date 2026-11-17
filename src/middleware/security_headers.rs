//! Response headers applied to every API response.

use axum::{
    Router,
    http::{
        HeaderName, HeaderValue,
        header::{
            CACHE_CONTROL, CONTENT_SECURITY_POLICY, REFERRER_POLICY, STRICT_TRANSPORT_SECURITY, X_CONTENT_TYPE_OPTIONS,
            X_FRAME_OPTIONS,
        },
    },
};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::config::Config;

/// The API only ever returns JSON, so nothing may be loaded, framed or submitted.
pub const API_CSP: &str = "default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

/// Swagger UI (development only) needs its own bundled scripts and styles.
pub const DOCS_CSP: &str = "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; \
     img-src 'self' data:; frame-ancestors 'none'; base-uri 'self'";

/// Adds security headers without overriding ones a route set deliberately.
pub fn apply<S: Clone + Send + Sync + 'static>(router: Router<S>, config: &Config) -> Router<S> {
    let header = |name: HeaderName, value: &'static str| {
        SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(value))
    };

    let router = router
        .layer(header(X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(header(X_FRAME_OPTIONS, "DENY"))
        .layer(header(REFERRER_POLICY, "no-referrer"))
        .layer(header(CONTENT_SECURITY_POLICY, API_CSP))
        .layer(header(
            HeaderName::from_static("permissions-policy"),
            "camera=(), microphone=(), geolocation=(), payment=()",
        ))
        .layer(header(HeaderName::from_static("cross-origin-resource-policy"), "same-origin"))
        .layer(header(HeaderName::from_static("cross-origin-opener-policy"), "same-origin"))
        // Authenticated JSON must never be stored by shared caches or the browser.
        .layer(header(CACHE_CONTROL, "no-store"));

    if config.cookie_secure {
        router.layer(header(STRICT_TRANSPORT_SECURITY, "max-age=63072000; includeSubDomains"))
    } else {
        router
    }
}
