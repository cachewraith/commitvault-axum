//! Custom validation rules shared across modules (used with `validator`).

use std::borrow::Cow;

use validator::ValidationError;

fn error(code: &'static str, message: &'static str) -> ValidationError {
    ValidationError::new(code).with_message(Cow::Borrowed(message))
}

/// Only absolute `http`/`https` URLs with a host, at most 2048 characters.
/// Blocks `javascript:`, `data:` and friends from ever being stored as links.
pub fn http_url(value: &str) -> Result<(), ValidationError> {
    if is_safe_http_url(value) { Ok(()) } else { Err(error("url", "Must be an http(s) URL")) }
}

pub fn is_safe_http_url(value: &str) -> bool {
    if value.len() > 2048 || value.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return false;
    }
    match url::Url::parse(value) {
        Ok(url) => matches!(url.scheme(), "http" | "https") && url.host_str().is_some_and(|h| !h.is_empty()),
        Err(_) => false,
    }
}

/// `#rrggbb`, lower-case (callers normalise case before validating).
pub fn hex_color(value: &str) -> Result<(), ValidationError> {
    let valid = value.len() == 7
        && value.starts_with('#')
        && value[1..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if valid { Ok(()) } else { Err(error("color", "Must be a hex colour like #22c55e")) }
}

/// Lower-case kebab-case slug.
pub fn slug(value: &str) -> Result<(), ValidationError> {
    let valid = !value.is_empty()
        && value.len() <= 140
        && value
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()));
    if valid { Ok(()) } else { Err(error("slug", "Use lower-case letters, digits and single hyphens")) }
}

/// Full or abbreviated Git object name: 7–64 lower-case hex characters.
pub fn commit_hash(value: &str) -> Result<(), ValidationError> {
    if is_hex(value, 7, 64) { Ok(()) } else { Err(error("commit_hash", "Must be 7–64 hexadecimal characters")) }
}

/// Abbreviated hash shown in the UI: 7–12 lower-case hex characters.
pub fn short_hash(value: &str) -> Result<(), ValidationError> {
    if is_hex(value, 7, 12) { Ok(()) } else { Err(error("short_hash", "Must be 7–12 hexadecimal characters")) }
}

fn is_hex(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Git ref rules, simplified: no whitespace/control characters and none of the
/// characters Git forbids in ref names.
pub fn branch_name(value: &str) -> Result<(), ValidationError> {
    let forbidden =
        |c: char| c.is_whitespace() || c.is_control() || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\');
    let valid = !value.is_empty()
        && value.len() <= 255
        && !value.chars().any(forbidden)
        && !value.contains("..")
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.ends_with('.');
    if valid { Ok(()) } else { Err(error("branch_name", "Is not a valid Git branch name")) }
}

/// No more than `MAX_IDS` ids in a relation list (tags, release commits).
pub const MAX_IDS: usize = 500;

pub fn id_list(ids: &[uuid::Uuid]) -> Result<(), ValidationError> {
    if ids.len() <= MAX_IDS { Ok(()) } else { Err(error("length", "Too many ids")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_url_accepts_only_http_and_https() {
        assert!(http_url("https://github.com/org/repo/commit/abc").is_ok());
        assert!(http_url("http://localhost:3000/x?y=1").is_ok());
        for bad in [
            "javascript:alert(1)",
            "data:text/html,x",
            "ftp://host/x",
            "//host/x",
            "https://",
            "not a url",
            " https://a.b",
            "https://a.b/\nx",
        ] {
            assert!(http_url(bad).is_err(), "{bad} should be rejected");
        }
        assert!(http_url(&format!("https://a.b/{}", "x".repeat(2048))).is_err());
    }

    #[test]
    fn commit_hash_rules() {
        assert!(commit_hash("a1b2c3d").is_ok());
        assert!(commit_hash(&"a".repeat(40)).is_ok());
        assert!(commit_hash(&"a".repeat(64)).is_ok());
        for bad in ["a1b2c3", "A1B2C3D", "g1b2c3d", "a1b2c3d ", &"a".repeat(65)] {
            assert!(commit_hash(bad).is_err(), "{bad} should be rejected");
        }
        assert!(short_hash("a1b2c3d").is_ok());
        assert!(short_hash(&"a".repeat(13)).is_err());
    }

    #[test]
    fn colour_slug_and_branch_rules() {
        assert!(hex_color("#22c55e").is_ok());
        for bad in ["22c55e", "#22C55E", "#fff", "#22c55ez"] {
            assert!(hex_color(bad).is_err(), "{bad} should be rejected");
        }
        assert!(slug("dev-server-2").is_ok());
        for bad in ["", "Dev", "a--b", "-a", "a-", "a b"] {
            assert!(slug(bad).is_err(), "{bad:?} should be rejected");
        }
        assert!(branch_name("feature/login-form").is_ok());
        for bad in ["", "a b", "a..b", "/a", "a/", "a~1", "a:b", "x."] {
            assert!(branch_name(bad).is_err(), "{bad:?} should be rejected");
        }
    }
}
