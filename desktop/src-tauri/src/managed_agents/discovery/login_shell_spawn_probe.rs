//! Test-only counter for login-shell spawn attempts.
//!
//! `run_in_login_shell` is the single subprocess-spawning step on the
//! absent-command resolution path, so counting its calls proves whether a
//! cheap discovery re-spawns after a negative resolution was cached.

use std::sync::atomic::{AtomicUsize, Ordering};

static COUNT: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn record() {
    COUNT.fetch_add(1, Ordering::SeqCst);
}

pub(crate) fn reset() {
    COUNT.store(0, Ordering::SeqCst);
}

pub(crate) fn count() -> usize {
    COUNT.load(Ordering::SeqCst)
}
