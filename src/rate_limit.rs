//! A small in-memory fixed-window limiter. The API runs as a single process, so memory is
//! enough; failures are counted per key (the client IP) and reset when the window passes

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

/// Beyond this many tracked keys, expired windows are dropped before adding another
const PRUNE_ABOVE: usize = 10_000;

pub struct RateLimiter {
    max_failures: u32,
    window: Duration,
    failures: Mutex<HashMap<String, (Instant, u32)>>,
}

impl RateLimiter {
    pub fn new(max_failures: u32, window: Duration) -> Self {
        Self {
            max_failures,
            window,
            failures: Mutex::new(HashMap::new()),
        }
    }

    /// Whether `key` may try again
    pub fn allows(&self, key: &str) -> bool {
        let failures = self
            .failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match failures.get(key) {
            Some((started, count)) => {
                started.elapsed() >= self.window || *count < self.max_failures
            }
            None => true,
        }
    }

    pub fn record_failure(&self, key: &str) {
        let mut failures = self
            .failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failures.len() > PRUNE_ABOVE {
            failures.retain(|_, (started, _)| started.elapsed() < self.window);
        }
        let entry = failures
            .entry(key.to_owned())
            .or_insert((Instant::now(), 0));
        if entry.0.elapsed() >= self.window {
            *entry = (Instant::now(), 0);
        }
        entry.1 += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_max_failures_per_key() {
        let limiter = RateLimiter::new(2, Duration::from_secs(60));
        assert!(limiter.allows("a"));
        limiter.record_failure("a");
        limiter.record_failure("a");
        assert!(!limiter.allows("a"));
        assert!(limiter.allows("b"));
    }

    #[test]
    fn window_expiry_resets_the_count() {
        let limiter = RateLimiter::new(1, Duration::ZERO);
        limiter.record_failure("a");
        assert!(limiter.allows("a"));
    }
}
