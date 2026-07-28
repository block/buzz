//! Regression test for the `--help` secret-leak fix: clap's `env` attribute
//! on `--private-key` used to render the *current value* of `BUZZ_PRIVATE_KEY`
//! inline in `--help` output (e.g. `[env: BUZZ_PRIVATE_KEY=nsec1...]`). This
//! spawns the real `buzz` binary as a subprocess with a synthetic secret set
//! only in the child's environment, so it exercises actual clap rendering
//! without mutating the test process's own environment (which would be
//! unsound under parallel test execution).

use std::process::Command;

const SYNTHETIC_NSEC: &str = "nsec1synthetic-do-not-use-0000000000000000000000000000000000";

fn run_help_with_secret_env(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_buzz"))
        .args(args)
        .env("BUZZ_PRIVATE_KEY", SYNTHETIC_NSEC)
        .output()
        .expect("failed to spawn buzz binary");

    // clap prints --help to stdout normally, but be defensive and check both
    // streams so a future clap/config change can't silently reintroduce the
    // leak via stderr.
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn long_help_does_not_leak_private_key_env_value() {
    let combined = run_help_with_secret_env(&["--help"]);
    assert!(
        !combined.contains(SYNTHETIC_NSEC),
        "--help leaked the BUZZ_PRIVATE_KEY value:\n{combined}"
    );
    // The env var name itself is expected/allowed to appear (documents how
    // to configure the key) — only the value must be hidden.
    assert!(
        combined.contains("BUZZ_PRIVATE_KEY"),
        "--help should still name BUZZ_PRIVATE_KEY so users know how to set it"
    );
}

#[test]
fn short_help_does_not_leak_private_key_env_value() {
    let combined = run_help_with_secret_env(&["-h"]);
    assert!(
        !combined.contains(SYNTHETIC_NSEC),
        "-h leaked the BUZZ_PRIVATE_KEY value:\n{combined}"
    );
}
