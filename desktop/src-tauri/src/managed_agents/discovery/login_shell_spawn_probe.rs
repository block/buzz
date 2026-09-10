//! Test-only counter for login-shell spawn attempts.
//!
//! `run_in_login_shell` is the single subprocess-spawning step on the
//! absent-command resolution path, so counting its calls proves whether a
//! cheap discovery re-spawns after a negative resolution was cached.

use std::cell::Cell;

thread_local! {
    static COUNT: Cell<usize> = const { Cell::new(0) };
}

pub(crate) fn record() {
    COUNT.with(|count| count.set(count.get().wrapping_add(1)));
}

pub(crate) fn reset() {
    COUNT.with(|count| count.set(0));
}

pub(crate) fn count() -> usize {
    COUNT.with(Cell::get)
}

#[cfg(test)]
mod tests {
    use super::{count, record, reset};

    #[test]
    fn operations_on_another_thread_do_not_change_this_threads_count() {
        reset();
        record();

        let other_thread_counts = std::thread::spawn(|| {
            reset();
            let after_reset = count();
            record();
            let after_record = count();
            reset();
            (after_reset, after_record)
        })
        .join()
        .expect("probe thread must finish");

        assert_eq!(other_thread_counts, (0, 1));
        assert_eq!(count(), 1);
        reset();
    }
}
