//! Shared process-global env guard for tests.
//!
//! Rust's default test runner executes tests on parallel threads in one
//! process, so any test that mutates process-global environment variables
//! (`std::env::set_var` / `remove_var`) can be observed mid-flight by another
//! test reading the same variable. This crate previously had two independent
//! per-module mutexes (`ENV_MUTEX` in `config.rs`, `ENV_LOCK` in
//! `telemetry.rs`) serializing the *writers* — but test helpers in a dozen
//! other modules call `Config::from_env()` (which reads the very vars
//! config.rs tests mutate, e.g. `BUZZ_S3_ADDRESSING_STYLE`,
//! `BUZZ_REDIS_POOL_SIZE`) while holding **no lock at all**. A reader
//! interleaving with a writer mid-mutation observes an invalid value and
//! `from_env()` errors — an order-dependent flake.
//!
//! [`EnvGuard`] fixes both halves: a single crate-wide mutex serializes every
//! env-mutating test AND every env-reading helper, and the guard records each
//! touched key's prior value so it is restored on drop — even when the test
//! panics or returns early.
//!
//! Reader idiom (test helpers that only need a consistent snapshot):
//! ```ignore
//! let config = {
//!     let _env = crate::test_env::EnvGuard::new();
//!     crate::config::Config::from_env().expect("default config loads")
//! };
//! ```
//! Keep the guard scope tight and synchronous — never hold it across an
//! `.await`.
//!
//! A test must create at most one guard at a time (builder-style
//! `set`/`remove` chain, or `new()` followed by `set_now`/`remove_now`);
//! creating a second guard while one is live would deadlock on the mutex.

use std::ffi::{OsStr, OsString};
use std::sync::{Mutex, MutexGuard};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Serializes process-global env mutations across the crate's tests and
/// restores every touched key to its pre-test value on drop.
pub struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
    /// `(key, prior_value)`, in first-touch order; restored in reverse on drop.
    originals: Vec<(&'static str, Option<OsString>)>,
}

impl EnvGuard {
    /// Acquire the crate-wide env lock. The lock is held until the guard is
    /// dropped, so no two tests can interleave env mutations.
    pub fn new() -> Self {
        Self {
            _lock: ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            originals: Vec::new(),
        }
    }

    fn record(&mut self, key: &'static str) {
        if !self.originals.iter().any(|(k, _)| *k == key) {
            self.originals.push((key, std::env::var_os(key)));
        }
    }

    /// Set `key` while holding the lock and record its prior value for
    /// restoration. Builder style: chain multiple calls before binding.
    pub fn set(mut self, key: &'static str, value: impl AsRef<OsStr>) -> Self {
        self.set_now(key, value);
        self
    }

    /// Set `key` (mid-test mutation while the guard already holds the lock).
    pub fn set_now(&mut self, key: &'static str, value: impl AsRef<OsStr>) {
        self.record(key);
        std::env::set_var(key, value);
    }

    /// Remove `key` while holding the lock and record its prior value for
    /// restoration. Builder style: chain multiple calls before binding.
    pub fn remove(mut self, key: &'static str) -> Self {
        self.remove_now(key);
        self
    }

    /// Remove `key` (mid-test mutation while the guard already holds the lock).
    pub fn remove_now(&mut self, key: &'static str) {
        self.record(key);
        std::env::remove_var(key);
    }
}

impl Default for EnvGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, original) in self.originals.iter().rev() {
            match original {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}
