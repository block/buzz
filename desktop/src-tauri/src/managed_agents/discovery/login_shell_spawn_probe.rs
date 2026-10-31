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
    #[cfg(unix)]
    assert!(
        is_isolated_process(),
        "counter measurements must run in an isolated test process"
    );
    COUNT.store(0, Ordering::SeqCst);
}

pub(crate) fn count() -> usize {
    COUNT.load(Ordering::SeqCst)
}

#[cfg(unix)]
const CHILD_TEST: &str = "BUZZ_LOGIN_SHELL_PROBE_TEST";

#[cfg(unix)]
fn is_isolated_process() -> bool {
    std::thread::current()
        .name()
        .is_some_and(|name| std::env::var(CHILD_TEST).as_deref() == Ok(name))
}

/// Run a counter-owning test body alone in a fresh libtest process.
///
/// The counter deliberately stays process-wide: discovery's auth workers must
/// count too. A voluntary PATH lock cannot exclude unrelated, unlocked probe
/// callers in the full suite. Process isolation excludes those callers without
/// changing production code or requiring new workers to inherit test context.
/// Call while holding the PATH lock so the child inherits stable environment.
#[cfg(unix)]
pub(crate) fn run_in_isolated_process(test: impl FnOnce()) {
    use std::process::Command;
    use std::time::Duration;

    let thread = std::thread::current();
    let name = thread.name().expect("libtest names its test threads");
    let completed = format!("completed isolated login-shell probe test: {name}");
    if is_isolated_process() {
        test();
        // Receipt only after the assertion body returns, never on entry.
        println!("{completed}");
        return;
    }

    let mut command = Command::new(std::env::current_exe().expect("test executable"));
    command
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env(CHILD_TEST, name);
    let output = super::bounded_command::output_with_timeout(command, Duration::from_secs(300))
        .expect("isolated probe test must finish within five minutes and the output cap");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.lines().any(|line| line.contains(&completed)),
        "isolated probe test {name} failed or did not run: {}\n{stdout}\n{stderr}",
        output.status
    );
}

#[cfg(unix)]
#[test]
fn isolated_counter_excludes_parent_probes_but_counts_own_workers() {
    let _guard = crate::managed_agents::lock_path_mutex();
    let probe_on_worker = || {
        std::thread::spawn(|| {
            // A real production probe, independent of the shared PATH cache.
            super::find_via_login_shell("buzz-absent-probe-isolation-xyzzy")
        })
        .join()
        .expect("probe worker must finish")
    };
    if !is_isolated_process() {
        assert!(probe_on_worker().is_none());
        assert!(count() >= 1, "parent worker must reach the real probe");
    }
    run_in_isolated_process(|| {
        // Before reset: unrelated parent probes must not enter this process.
        assert_eq!(count(), 0, "the child must start with its own counter");
        reset();
        assert!(probe_on_worker().is_none());
    });
    if is_isolated_process() {
        // Outside the closure so omitting its invocation cannot pass this test.
        assert_eq!(count(), 1, "the child's assertion body and worker must run");
    }
}

#[cfg(unix)]
#[test]
#[should_panic(expected = "counter measurements must run in an isolated test process")]
fn reset_rejects_unisolated_measurement() {
    reset();
}
