//! Integration test: ledger deletion happens before the setup-mode early return.
//!
//! Exercises the boot-boundary ordering invariant: when `resume_on_restart=false`,
//! the ledger file must be deleted even when `BUZZ_ACP_SETUP_PAYLOAD` triggers
//! the early `run_setup_listener()` return — i.e., the deletion is NOT bypassed
//! by the setup-mode branch.
//!
//! The helper-only tests in `ledger.rs` (which call `delete_ledger_file` directly)
//! pass regardless of whether the call site in `lib.rs` is reachable. This test
//! is discriminating: it spawns the real binary and asserts the file-system
//! side-effect on the actual boot path.
//!
//! Setup:
//!   - Private key: nsec of scalar 1 → known pubkey prefix `79be667ef9dcbbac`.
//!   - Relay URL: `ws://127.0.0.1:1` → sha256 prefix `4d9ff4283940665b`.
//!   - Ledger filename: `pending-turns-79be667ef9dcbbac-4d9ff4283940665b.json`.
//!   - The binary connects to an unreachable relay (ws://127.0.0.1:1, connection
//!     refused). Connection refused is non-terminal and retried with backoff, so
//!     the binary keeps running — but the ledger deletion happens in the first
//!     milliseconds of boot, long before the relay connect attempt. The test polls
//!     for deletion and kills the child once confirmed rather than waiting for it
//!     to exhaust its retry budget (~31 s).

use std::time::Duration;

/// nsec1 encoding of private key scalar 1.
const TEST_NSEC: &str = "nsec1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqsmhltgl";

/// A minimal valid `BUZZ_ACP_SETUP_PAYLOAD`.
const SETUP_PAYLOAD: &str = concat!(
    r#"{"agent_name":"Test","#,
    r#""agent_pubkey":"79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798","#,
    r#""requirements":[]}"#
);

/// Ledger filename for:
///   pubkey prefix = first 16 hex chars of 79be667ef9dcbbac55a06…
///   relay hash    = first 16 hex chars of sha256("ws://127.0.0.1:1")
///                 = 4d9ff4283940665b
const LEDGER_FILENAME: &str = "pending-turns-79be667ef9dcbbac-4d9ff4283940665b.json";

#[test]
fn test_setup_mode_boot_deletes_ledger_when_resume_off() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state_dir = dir.path();

    // Step 1: seed the ledger file that the binary would delete.
    let ledger_path = state_dir.join(LEDGER_FILENAME);
    std::fs::write(&ledger_path, b"{}").expect("write seed ledger file");
    assert!(ledger_path.exists(), "seed file must exist before spawn");

    let bin = env!("CARGO_BIN_EXE_buzz-acp");

    // Step 2: spawn with resume_on_restart=false and a setup payload.
    // The relay is unreachable so run_setup_listener exits quickly.
    let mut child = std::process::Command::new(bin)
        .env("BUZZ_PRIVATE_KEY", TEST_NSEC)
        .env("BUZZ_RELAY_URL", "ws://127.0.0.1:1")
        .env("BUZZ_ACP_RESUME_ON_RESTART", "false")
        .env("BUZZ_ACP_STATE_DIR", state_dir)
        .env("BUZZ_ACP_SETUP_PAYLOAD", SETUP_PAYLOAD)
        .env("RUST_LOG", "error")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn buzz-acp binary");

    // The ledger file must be deleted early in the boot sequence — before
    // the relay connect attempt.  The relay (`ws://127.0.0.1:1`) is
    // unreachable, so the binary retries for ~31 seconds before exiting.
    // We don't wait for the binary to exit; we poll for the file deletion
    // (which happens in the first milliseconds of boot) and then kill the
    // child once we have our answer.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let deleted = loop {
        if !ledger_path.exists() {
            break true;
        }
        match child.try_wait().expect("wait on child") {
            Some(_) => break false, // exited before deleting — also a failure
            None if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            None => {
                // Still running after 10s but file still exists — treat as failure.
                break false;
            }
        }
    };
    child.kill().ok();
    child.wait().ok();

    // Step 3: assert the ledger file was deleted during boot (before setup-mode return).
    assert!(
        deleted,
        "ledger file must be deleted even when BUZZ_ACP_SETUP_PAYLOAD triggers \
         the setup-listener early return (boot-boundary deletion ordering)"
    );
}
