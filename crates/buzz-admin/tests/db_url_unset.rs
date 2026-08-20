//! Regression test for #2837: `buzz-admin` must fail loudly when
//! `DATABASE_URL` is unset, instead of silently falling back to hardcoded
//! dev database credentials.
//!
//! Spawns the built `buzz-admin` binary with `DATABASE_URL` removed from the
//! environment and asserts a non-zero exit code plus a clear error message.
//! This guards against reintroducing the `unwrap_or_else(|_| "postgres://buzz:buzz_dev@...")`
//! fallback that masked missing configuration as a Postgres auth failure.

use std::process::Command;

#[test]
fn migrate_fails_loudly_when_database_url_unset() {
    let bin = env!("CARGO_BIN_EXE_buzz-admin");
    let output = Command::new(bin)
        .arg("migrate")
        .env_remove("DATABASE_URL")
        .output()
        .expect("failed to spawn buzz-admin");

    assert!(
        !output.status.success(),
        "buzz-admin migrate should fail when DATABASE_URL is unset, but exited successfully"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("DATABASE_URL"),
        "stderr should mention DATABASE_URL, got: {stderr}"
    );
}
