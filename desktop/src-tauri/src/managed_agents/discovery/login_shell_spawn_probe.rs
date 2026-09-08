//! Test-only counter for login-shell spawn attempts.
//!
//! `run_in_login_shell` and the discovery calls under test are synchronous.
//! Count the calling thread's attempts, not unrelated parallel tests' probes;
//! the process-environment lock does not serialize every resolver caller.

use std::cell::Cell;

thread_local! {
    static COUNT: Cell<usize> = const { Cell::new(0) };
}

pub(crate) fn record() {
    COUNT.with(|count| count.set(count.get() + 1));
}

pub(crate) fn reset() {
    COUNT.with(|count| count.set(0));
}

pub(crate) fn count() -> usize {
    COUNT.with(Cell::get)
}

#[test]
fn counts_this_synchronous_probe_not_parallel_callers() {
    reset();
    record();
    std::thread::spawn(|| {
        assert_eq!(count(), 0);
        record();
        record();
        assert_eq!(count(), 2);
        reset();
    })
    .join()
    .unwrap();
    assert_eq!(count(), 1);
    reset();
}
