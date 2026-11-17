//! Argon2id password hashing. Hashing is CPU-bound (tens of milliseconds by
//! design), so both operations run on the blocking thread pool.

use anyhow::{Context, anyhow};
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};

/// Hashes with Argon2id and a fresh random salt; returns a PHC string.
pub async fn hash(password: String) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || hash_blocking(&password)).await.context("password hashing task failed")?
}

/// Verifies `password` against a PHC string. A malformed stored hash is an
/// error; a wrong password is `Ok(false)`.
pub async fn verify(password: String, phc_hash: String) -> anyhow::Result<bool> {
    tokio::task::spawn_blocking(move || verify_blocking(&password, &phc_hash))
        .await
        .context("password verification task failed")?
}

fn hash_blocking(password: &str) -> anyhow::Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|err| anyhow!("failed to hash password: {err}"))
}

fn verify_blocking(password: &str, phc_hash: &str) -> anyhow::Result<bool> {
    let parsed = PasswordHash::new(phc_hash).map_err(|err| anyhow!("stored password hash is malformed: {err}"))?;
    Ok(Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
}

/// A valid hash of a random value. Verifying against it when the account does
/// not exist keeps login timing independent of whether the email is known.
pub fn dummy_hash() -> &'static str {
    static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HASH.get_or_init(|| hash_blocking(&uuid::Uuid::new_v4().to_string()).expect("hashing a static value cannot fail"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_argon2id_and_salted() {
        let first = hash_blocking("correct horse battery staple").unwrap();
        let second = hash_blocking("correct horse battery staple").unwrap();
        assert!(first.starts_with("$argon2id$"));
        assert_ne!(first, second, "each hash uses a fresh salt");
        assert!(!first.contains("correct horse"));
    }

    #[test]
    fn verify_accepts_only_the_right_password() {
        let phc = hash_blocking("correct horse battery staple").unwrap();
        assert!(verify_blocking("correct horse battery staple", &phc).unwrap());
        assert!(!verify_blocking("Correct horse battery staple", &phc).unwrap());
        assert!(!verify_blocking("", &phc).unwrap());
    }

    #[test]
    fn malformed_hash_is_an_error_not_a_match() {
        assert!(verify_blocking("x", "not-a-phc-string").is_err());
        assert!(!verify_blocking("anything", dummy_hash()).unwrap());
    }
}
