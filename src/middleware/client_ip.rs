use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::request::Parts,
};

use crate::AppState;

/// Best-effort client address, used as a rate-limiting key.
///
/// `X-Forwarded-For` is only honoured when `TRUST_PROXY=true`; otherwise any
/// client could spoof it and dodge the login limiter. With a trusted proxy the
/// right-most entry is the address the proxy itself saw.
#[derive(Debug, Clone)]
pub struct ClientIp(pub String);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        if state.config.trust_proxy
            && let Some(forwarded) = parts.headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
            && let Some(ip) = forwarded.rsplit(',').map(str::trim).find(|ip| !ip.is_empty())
        {
            return Ok(Self(ip.to_owned()));
        }
        let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|info| info.0.ip().to_string());
        Ok(Self(peer.unwrap_or_else(|| "unknown".to_owned())))
    }
}
