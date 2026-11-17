//! In-memory limiter for failed login attempts (fixed window per key).
//!
//! State lives in the process: this is a single-instance, single-user
//! application, so a shared store (Redis) would add operational cost without
//! benefit. Restarting the server resets the counters, which is acceptable.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy)]
struct Window {
    started: Instant,
    failures: u32,
}

#[derive(Debug)]
pub struct LoginRateLimiter {
    window: Duration,
    state: Mutex<HashMap<String, Window>>,
}

/// Failed attempts allowed per client address inside one window.
pub const MAX_FAILURES_PER_IP: u32 = 5;
/// Failed attempts allowed per account inside one window, across all addresses.
/// Higher than the per-address limit so one noisy client cannot easily lock
/// the owner out, while distributed guessing is still throttled.
pub const MAX_FAILURES_PER_EMAIL: u32 = 20;
const WINDOW: Duration = Duration::from_secs(15 * 60);
const PRUNE_THRESHOLD: usize = 10_000;

impl Default for LoginRateLimiter {
    fn default() -> Self {
        Self::new(WINDOW)
    }
}

impl LoginRateLimiter {
    pub fn new(window: Duration) -> Self {
        Self { window, state: Mutex::new(HashMap::new()) }
    }

    /// `Err(seconds_until_retry)` when `key` has used up its budget.
    pub fn check(&self, key: &str, max_failures: u32) -> Result<(), u64> {
        let now = Instant::now();
        let state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match state.get(key) {
            Some(w) if now.duration_since(w.started) < self.window && w.failures >= max_failures => {
                Err((self.window - now.duration_since(w.started)).as_secs().max(1))
            }
            _ => Ok(()),
        }
    }

    pub fn record_failure(&self, key: &str) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.len() >= PRUNE_THRESHOLD {
            state.retain(|_, w| now.duration_since(w.started) < self.window);
        }
        let entry = state.entry(key.to_owned()).or_insert(Window { started: now, failures: 0 });
        if now.duration_since(entry.started) >= self.window {
            *entry = Window { started: now, failures: 0 };
        }
        entry.failures += 1;
    }

    pub fn reset(&self, key: &str) {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_the_limit_and_reports_retry_time() {
        let limiter = LoginRateLimiter::new(Duration::from_secs(60));
        for _ in 0..3 {
            assert!(limiter.check("ip:1.2.3.4", 3).is_ok());
            limiter.record_failure("ip:1.2.3.4");
        }
        let retry = limiter.check("ip:1.2.3.4", 3).unwrap_err();
        assert!((1..=60).contains(&retry));
        assert!(limiter.check("ip:5.6.7.8", 3).is_ok(), "other keys are unaffected");
    }

    #[test]
    fn reset_clears_the_counter() {
        let limiter = LoginRateLimiter::new(Duration::from_secs(60));
        limiter.record_failure("k");
        limiter.record_failure("k");
        limiter.reset("k");
        assert!(limiter.check("k", 2).is_ok());
    }

    #[test]
    fn window_expiry_restores_the_budget() {
        let limiter = LoginRateLimiter::new(Duration::from_millis(30));
        limiter.record_failure("k");
        assert!(limiter.check("k", 1).is_err());
        std::thread::sleep(Duration::from_millis(40));
        assert!(limiter.check("k", 1).is_ok());
        limiter.record_failure("k");
        assert!(limiter.check("k", 2).is_ok(), "a new window starts from zero");
    }
}
