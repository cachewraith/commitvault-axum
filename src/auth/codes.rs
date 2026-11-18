//! Emailed one-time codes: generation, keyed hashing, and message text.

use std::time::Duration;

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::mail::OutgoingEmail;

/// How long a code stays valid.
pub const CODE_TTL: Duration = Duration::from_secs(10 * 60);
/// Wrong guesses after which a code stops working (a new one must be sent).
pub const MAX_CODE_ATTEMPTS: i32 = 5;
/// Minimum time between two codes of the same purpose for one account.
pub const RESEND_INTERVAL: Duration = Duration::from_secs(60);
pub const CODE_LEN: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodePurpose {
    Login,
    PasswordReset,
}

impl CodePurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::PasswordReset => "password_reset",
        }
    }
}

/// Six uniformly random decimal digits.
pub fn generate() -> String {
    format!("{:06}", rand::random_range(0..1_000_000u32))
}

/// Keyed SHA-256 so a leaked `auth_codes` table cannot be brute-forced
/// offline without `SESSION_SECRET`. Binding purpose and user means a code is
/// only good for what it was issued for.
pub fn hash(secret: &str, purpose: CodePurpose, user_id: Uuid, code: &str) -> String {
    let digest = Sha256::new()
        .chain_update(secret.as_bytes())
        .chain_update(b"\n")
        .chain_update(purpose.as_str().as_bytes())
        .chain_update(b"\n")
        .chain_update(user_id.as_bytes())
        .chain_update(b"\n")
        .chain_update(code.as_bytes())
        .finalize();
    hex(&digest)
}

pub fn matches(stored_hash: &str, candidate_hash: &str) -> bool {
    stored_hash.as_bytes().ct_eq(candidate_hash.as_bytes()).into()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `a•••@example.com` — enough to recognise the address, not to learn it.
pub fn email_hint(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => {
            let first: String = local.chars().take(1).collect();
            format!("{first}•••@{domain}")
        }
        None => "your email address".to_owned(),
    }
}

pub fn email(purpose: CodePurpose, to: &str, code: &str) -> OutgoingEmail {
    let minutes = CODE_TTL.as_secs() / 60;
    let (subject, intro, warning) = match purpose {
        CodePurpose::Login => (
            "Your CommitVault sign-in code",
            "Use this code to finish signing in to CommitVault:",
            "If you did not just sign in, someone knows your password. Change it now.",
        ),
        CodePurpose::PasswordReset => (
            "Your CommitVault password reset code",
            "Use this code to choose a new CommitVault password:",
            "If you did not ask to reset your password, you can ignore this email.",
        ),
    };
    OutgoingEmail {
        to: to.to_owned(),
        subject: subject.to_owned(),
        body: format!("{intro}\n\n    {code}\n\nThe code expires in {minutes} minutes and works once.\n\n{warning}\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_six_digits() {
        for _ in 0..200 {
            let code = generate();
            assert_eq!(code.len(), CODE_LEN);
            assert!(code.bytes().all(|b| b.is_ascii_digit()));
        }
    }

    #[test]
    fn hash_is_bound_to_secret_purpose_and_user() {
        let user = Uuid::now_v7();
        let base = hash("secret", CodePurpose::Login, user, "123456");
        assert!(matches(&base, &hash("secret", CodePurpose::Login, user, "123456")));
        assert!(!matches(&base, &hash("secret", CodePurpose::Login, user, "123457")));
        assert!(!matches(&base, &hash("other", CodePurpose::Login, user, "123456")));
        assert!(!matches(&base, &hash("secret", CodePurpose::PasswordReset, user, "123456")));
        assert!(!matches(&base, &hash("secret", CodePurpose::Login, Uuid::now_v7(), "123456")));
        assert!(!base.contains("123456"));
    }

    #[test]
    fn hint_masks_the_local_part() {
        assert_eq!(email_hint("admin@example.com"), "a•••@example.com");
    }
}
