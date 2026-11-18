//! "Trusted browser" cookie: after a browser completes the emailed-code step
//! it receives a random token, and later sign-ins from that browser skip the
//! code until the token expires. Only the token's SHA-256 is stored.

use std::time::Duration;

use axum::http::{HeaderMap, HeaderValue, header};
use sha2::{Digest, Sha256};
use tower_sessions::cookie::{Cookie, SameSite};

use super::codes::hex;
use crate::config::Config;

/// How long a browser stays trusted after completing the code step.
pub const TRUSTED_DEVICE_TTL: Duration = Duration::from_secs(90 * 24 * 3600);

/// 256 random bits, hex encoded.
pub fn new_token() -> String {
    hex(&rand::random::<[u8; 32]>())
}

pub fn hash_token(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// The device token sent by the browser, if any.
pub fn token_from_headers(headers: &HeaderMap, cookie_name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == cookie_name)
        .map(|(_, value)| value.to_owned())
        .filter(|value| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// `Set-Cookie` value for a freshly trusted browser. Same attributes as the
/// session cookie: HttpOnly, SameSite=Strict, Secure (and `__Host-`) over HTTPS.
pub fn set_cookie(config: &Config, token: &str) -> HeaderValue {
    let cookie = Cookie::build((config.device_cookie_name(), token.to_owned()))
        .http_only(true)
        .secure(config.cookie_secure)
        .same_site(SameSite::Strict)
        .path("/")
        .max_age(time::Duration::seconds(TRUSTED_DEVICE_TTL.as_secs() as i64))
        .build();
    HeaderValue::from_str(&cookie.to_string()).expect("cookie built from hex token is a valid header value")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_a_well_formed_token() {
        let token = new_token();
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, HeaderValue::from_str(&format!("cv_session=abc; cv_device={token}")).unwrap());
        assert_eq!(token_from_headers(&headers, "cv_device"), Some(token));
        assert_eq!(token_from_headers(&headers, "__Host-cv_device"), None);

        headers.insert(header::COOKIE, HeaderValue::from_static("cv_device=not-hex"));
        assert_eq!(token_from_headers(&headers, "cv_device"), None);
    }
}
