//! Failure throttling for unauthenticated credential checks.
//!
//! `/api/auth/login`, the password-reset endpoints and public share unlock all
//! compare a secret supplied by an anonymous caller. Without a brake, a LAN
//! neighbour (or anyone the server is exposed to) can try passwords and share
//! PINs as fast as the server answers — bcrypt at cost 10 still leaves
//! thousands of guesses per minute across parallel connections.
//!
//! Attempts are keyed by what is being attacked (email address, share token)
//! rather than by peer IP: the server sits behind TLS termination and HTTP/3
//! paths that do not all carry `ConnectInfo`, and an IP key is trivially
//! rotated anyway. Keying by target does mean an attacker can slow down a
//! specific account on purpose, so failures decay and the response is a delay
//! first, a hard refusal only well past any human typo rate.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use crate::errors::{SarcaError, SarcaResult};

/// Failures allowed before each attempt starts costing a delay.
const FREE_ATTEMPTS: u32 = 5;
/// Failures after which the key is refused outright until it decays.
const LOCK_ATTEMPTS: u32 = 25;
/// Longest delay applied to a single attempt.
const MAX_DELAY: Duration = Duration::from_secs(4);
/// A key with no failures for this long starts over.
const DECAY: Duration = Duration::from_mins(15);
/// Cap on tracked keys, so a flood of distinct emails cannot grow the map
/// without bound.
const MAX_KEYS: usize = 4096;

#[derive(Debug, Clone, Copy)]
struct Attempts {
    failures: u32,
    last: Instant,
}

/// What the caller must do before the secret is even compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Proceed, after sleeping this long (zero for the first few attempts).
    Delay(Duration),
    /// Too many failures: refuse without comparing anything.
    Locked,
}

#[derive(Debug, Clone, Default)]
pub struct FailureThrottle {
    inner: Arc<Mutex<HashMap<String, Attempts>>>,
}

impl FailureThrottle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decide what an attempt against `key` costs, at time `now`.
    fn decide_at(&self, key: &str, now: Instant) -> Decision {
        // The lock is confined to this block: no caller should wait on the map
        // while another request is computing (or sleeping out) its penalty.
        let entry = {
            let mut map = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
            prune(&mut map, now);
            match map.get(key).copied() {
                // Idle long enough that the record no longer counts.
                Some(stale) if now.duration_since(stale.last) >= DECAY => {
                    map.remove(key);
                    None
                },
                found => found,
            }
        };

        let Some(entry) = entry else {
            return Decision::Delay(Duration::ZERO);
        };
        if entry.failures >= LOCK_ATTEMPTS {
            return Decision::Locked;
        }
        Decision::Delay(delay_for(entry.failures))
    }

    pub fn decide(&self, key: &str) -> Decision {
        self.decide_at(key, Instant::now())
    }

    /// Wait out the penalty for `key`, or report that it is locked.
    ///
    /// Sleeping here (rather than returning 429 immediately) keeps the cost on
    /// the attacker's connection while leaving a mistyped password usable.
    pub async fn check(&self, key: &str) -> SarcaResult<()> {
        match self.decide(key) {
            Decision::Locked => Err(SarcaError::TooManyAttempts),
            Decision::Delay(delay) => {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                Ok(())
            },
        }
    }

    fn record_failure_at(&self, key: &str, now: Instant) {
        let mut map = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        prune(&mut map, now);
        if map.len() >= MAX_KEYS && !map.contains_key(key) {
            // Full and this key is new: drop the least recently touched one.
            if let Some(oldest) = map.iter().min_by_key(|(_, a)| a.last).map(|(k, _)| k.clone()) {
                map.remove(&oldest);
            }
        }
        // Anything older than DECAY starts the count over.
        let previous = map
            .get(key)
            .filter(|entry| now.duration_since(entry.last) < DECAY)
            .map_or(0, |entry| entry.failures);
        map.insert(
            key.to_owned(),
            Attempts {
                failures: previous.saturating_add(1),
                last: now,
            },
        );
    }

    /// Count one rejected secret.
    pub fn record_failure(&self, key: &str) {
        self.record_failure_at(key, Instant::now());
    }

    /// Forget a key after the correct secret was supplied.
    pub fn record_success(&self, key: &str) {
        let mut map = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        map.remove(key);
    }
}

/// Keys are namespaced so a share token can never collide with an email
/// address, and one bounded map can serve every endpoint.
pub mod keys {
    /// Login is keyed by the address being attacked, case-folded so `A@b.c`
    /// and `a@b.c` share a budget.
    pub fn login(email: &str) -> String {
        format!("login:{}", email.trim().to_lowercase())
    }

    /// Share unlock is keyed by the link token, which is what an attacker is
    /// guessing passwords for.
    pub fn share_unlock(token: &str) -> String {
        format!("unlock:{token}")
    }
}

fn delay_for(failures: u32) -> Duration {
    if failures < FREE_ATTEMPTS {
        return Duration::ZERO;
    }
    let steps = failures - FREE_ATTEMPTS;
    let millis = 250_u64.saturating_mul(1_u64 << steps.min(6));
    Duration::from_millis(millis).min(MAX_DELAY)
}

fn prune(map: &mut HashMap<String, Attempts>, now: Instant) {
    map.retain(|_, a| now.duration_since(a.last) < DECAY);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> &'static str {
        "user@example.com"
    }

    #[test]
    fn the_first_attempts_are_free() {
        let throttle = FailureThrottle::new();
        for _ in 0..FREE_ATTEMPTS {
            assert_eq!(throttle.decide(key()), Decision::Delay(Duration::ZERO));
            throttle.record_failure(key());
        }
        assert!(matches!(throttle.decide(key()), Decision::Delay(d) if d > Duration::ZERO));
    }

    #[test]
    fn the_delay_grows_and_is_capped() {
        assert_eq!(delay_for(0), Duration::ZERO);
        assert_eq!(delay_for(FREE_ATTEMPTS), Duration::from_millis(250));
        assert_eq!(delay_for(FREE_ATTEMPTS + 1), Duration::from_millis(500));
        assert_eq!(delay_for(LOCK_ATTEMPTS - 1), MAX_DELAY);
    }

    #[test]
    fn enough_failures_lock_the_key() {
        let throttle = FailureThrottle::new();
        for _ in 0..LOCK_ATTEMPTS {
            throttle.record_failure(key());
        }
        assert_eq!(throttle.decide(key()), Decision::Locked);
    }

    #[test]
    fn a_success_clears_the_penalty() {
        let throttle = FailureThrottle::new();
        for _ in 0..LOCK_ATTEMPTS {
            throttle.record_failure(key());
        }
        throttle.record_success(key());
        assert_eq!(throttle.decide(key()), Decision::Delay(Duration::ZERO));
    }

    #[test]
    fn failures_decay_so_a_key_cannot_be_locked_forever() {
        let throttle = FailureThrottle::new();
        let start = Instant::now();
        for _ in 0..LOCK_ATTEMPTS {
            throttle.record_failure_at(key(), start);
        }
        assert_eq!(throttle.decide_at(key(), start), Decision::Locked);

        let later = start + DECAY + Duration::from_secs(1);
        assert_eq!(throttle.decide_at(key(), later), Decision::Delay(Duration::ZERO));
    }

    #[test]
    fn keys_are_independent() {
        let throttle = FailureThrottle::new();
        for _ in 0..LOCK_ATTEMPTS {
            throttle.record_failure(key());
        }
        assert_eq!(throttle.decide("someone@else.test"), Decision::Delay(Duration::ZERO));
    }

    #[test]
    fn namespaces_keep_surfaces_apart() {
        assert_ne!(keys::login("t"), keys::share_unlock("t"));
        // Case and padding must not buy a fresh budget.
        assert_eq!(keys::login(" A@B.C "), keys::login("a@b.c"));
    }

    #[test]
    fn the_map_stays_bounded() {
        let throttle = FailureThrottle::new();
        for i in 0..(MAX_KEYS + 64) {
            throttle.record_failure(&format!("user{i}@example.com"));
        }
        let len = throttle.inner.lock().unwrap().len();
        assert!(len <= MAX_KEYS, "tracked {len} keys");
    }
}
