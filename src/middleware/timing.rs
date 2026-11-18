//! Request timing: every response carries `Server-Timing: app;dur=<ms>` (the
//! time spent inside the backend, session load and save included), and slow
//! requests are logged. Comparing that number with what the browser measures
//! separates backend time from proxy and network time.

use std::time::{Duration, Instant};

use axum::{
    extract::Request,
    http::{HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};

/// Requests slower than this are logged at `warn`.
const SLOW_REQUEST: Duration = Duration::from_millis(500);

const SERVER_TIMING: HeaderName = HeaderName::from_static("server-timing");

pub async fn server_timing(request: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = request.method().clone();
    // The path only: query strings can carry search terms.
    let path = request.uri().path().to_owned();

    let mut response = next.run(request).await;

    let elapsed = started.elapsed();
    let millis = elapsed.as_secs_f64() * 1000.0;
    if let Ok(value) = HeaderValue::from_str(&format!("app;dur={millis:.1}")) {
        response.headers_mut().insert(SERVER_TIMING, value);
    }
    if elapsed >= SLOW_REQUEST {
        tracing::warn!(%method, path, status = response.status().as_u16(), ms = millis.round() as u64, "slow request");
    }
    response
}
