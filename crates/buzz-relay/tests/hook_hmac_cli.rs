//! End-to-end fail-closed check for the `buzz-relay hook-hmac` helper the
//! generated pre-receive hook shells out to.
//!
//! The unit tests in `main.rs` cover signing with separate in-memory readers.
//! This exercises the real binary and pins the remaining process boundary: a
//! missing secret descriptor must not yield anything the hook could mistake
//! for a signature.

use std::process::{Command, Stdio};

const HELPER: &str = env!("CARGO_BIN_EXE_buzz-relay");

#[test]
fn helper_fails_closed_without_secret_fd() {
    let output = Command::new(HELPER)
        .arg("hook-hmac")
        .stdin(Stdio::null())
        .output()
        .expect("run hook-hmac helper");

    assert!(
        !output.status.success(),
        "helper must fail when fd 3 was never opened, got {:?}",
        output.status
    );
    assert!(
        output.stdout.is_empty(),
        "a failed signing attempt must not print a signature: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hook HMAC secret fd 3"),
        "error must name the missing secret descriptor, got {stderr:?}"
    );
}
