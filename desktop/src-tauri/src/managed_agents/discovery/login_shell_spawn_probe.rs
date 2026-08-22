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

// `reset`/`count` are called only from `#[cfg(unix)]` tests, so on Windows they
// are dead code and `-D warnings` rejects them. Fork-only divergence: upstream's
// CI does not build this crate for Windows, ours does. `record` is exempt — it
// is called from production code on every platform.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn reset() {
    COUNT.store(0, Ordering::SeqCst);
}

#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn count() -> usize {
    COUNT.load(Ordering::SeqCst)
}
