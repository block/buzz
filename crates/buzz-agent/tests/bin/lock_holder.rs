//! Test-only helper: a real second process that takes the coordinator's
//! cross-process advisory lock and holds it until killed.
//!
//! The auth coordinator single-flights per cache key on an `fs2` advisory lock
//! (`flock` on Unix, `LockFileEx` on Windows). To prove the *cross-process*
//! contract — a genuine other process serializes the flow, and its death
//! releases the lock with no PID files or lock-breaking — a test needs an
//! actual separate process on the same lock file, not a second in-process
//! handle. This binary is that process.
//!
//! Driven by two env vars:
//!   LOCK_HELPER_PATH   — the lock file to acquire (the coordinator's
//!                        `<hash>.json.lock`).
//!   LOCK_HELPER_READY  — a marker file created *after* the lock is held, so
//!                        the parent test can synchronize on ownership before
//!                        racing the coordinator.
//!
//! After signaling readiness it blocks forever; the parent kills it to model a
//! crash mid-flow.

use std::fs;

use fs2::FileExt;

fn main() {
    let lock_path = std::env::var("LOCK_HELPER_PATH").expect("LOCK_HELPER_PATH set");
    let ready_path = std::env::var("LOCK_HELPER_READY").expect("LOCK_HELPER_READY set");

    if let Some(parent) = std::path::Path::new(&lock_path).parent() {
        fs::create_dir_all(parent).expect("create lock parent dir");
    }
    // Open exactly as the coordinator does so we contend on the same inode.
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .expect("open lock file");
    file.lock_exclusive()
        .expect("hold the exclusive advisory lock");

    // Signal ownership only once the lock is truly held.
    fs::write(&ready_path, b"held").expect("write ready marker");

    // Hold the lock until the parent kills us (crash stand-in). The kernel
    // releases the advisory lock on process death.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
