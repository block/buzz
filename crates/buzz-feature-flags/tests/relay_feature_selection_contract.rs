use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;

fn fixture_manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/relay-feature-selection/Cargo.toml")
}

fn fixture_lockfile() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/relay-feature-selection/Cargo.lock")
}

fn cargo_bin() -> OsString {
    env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

fn cargo_check(target_dir: &Path, case_name: &str, features: &[&str]) -> std::process::Output {
    let mut command = Command::new(cargo_bin());
    command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("check")
        .arg("--quiet")
        .arg("--locked")
        .arg("--manifest-path")
        .arg(fixture_manifest())
        .arg("--no-default-features")
        .env("CARGO_TARGET_DIR", target_dir);

    if !features.is_empty() {
        command.arg("--features").arg(features.join(","));
    }

    command
        .output()
        .unwrap_or_else(|error| panic!("failed to run cargo check for {case_name}: {error}"))
}

#[test]
fn relay_artifact_provider_features_require_exactly_one_selection() {
    assert!(
        fixture_lockfile().is_file(),
        "relay feature selection fixture must check in Cargo.lock for locked nested cargo runs"
    );

    let target_dir = TempDir::new().expect("temp target dir");

    let valid_cases = [
        ("static", &["static-feature-flags"][..]),
        ("environment", &["environment-feature-flags"][..]),
        ("launchdarkly", &["launchdarkly-feature-flags"][..]),
    ];

    for (case_name, features) in valid_cases {
        let output = cargo_check(target_dir.path(), case_name, features);
        assert!(
            output.status.success(),
            "expected {case_name} to compile successfully\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let invalid_cases = [
        ("none", &[][..]),
        (
            "static-and-environment",
            &["static-feature-flags", "environment-feature-flags"][..],
        ),
        (
            "static-and-launchdarkly",
            &["static-feature-flags", "launchdarkly-feature-flags"][..],
        ),
        (
            "environment-and-launchdarkly",
            &["environment-feature-flags", "launchdarkly-feature-flags"][..],
        ),
        (
            "all-three",
            &[
                "static-feature-flags",
                "environment-feature-flags",
                "launchdarkly-feature-flags",
            ][..],
        ),
    ];

    for (case_name, features) in invalid_cases {
        let output = cargo_check(target_dir.path(), case_name, features);
        assert!(
            !output.status.success(),
            "expected {case_name} to fail compilation"
        );

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("select exactly one relay feature flag provider feature"),
            "expected {case_name} failure to explain the exact-one contract\nstderr:\n{stderr}"
        );
    }
}
