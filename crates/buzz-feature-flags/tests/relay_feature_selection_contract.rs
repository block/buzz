use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;

const FIXTURE_LOCK_REGEN_RECIPE: &str =
    "cargo generate-lockfile --manifest-path crates/buzz-feature-flags/tests/fixtures/relay-feature-selection/Cargo.toml";

fn fixture_manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/relay-feature-selection/Cargo.toml")
}

fn fixture_lockfile() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/relay-feature-selection/Cargo.lock")
}

fn crate_readme() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md")
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
        .unwrap_or_else(|error| {
            panic!(
                "failed to run cargo check for {case_name}: {error}\nfixture lock regeneration recipe: `{FIXTURE_LOCK_REGEN_RECIPE}`"
            )
        })
}

#[test]
fn relay_fixture_lock_regeneration_recipe_is_documented() {
    let readme = std::fs::read_to_string(crate_readme()).expect("read crate README");
    assert!(
        readme.contains(FIXTURE_LOCK_REGEN_RECIPE),
        "README.md must document fixture lock regeneration with `{FIXTURE_LOCK_REGEN_RECIPE}`"
    );
}

#[test]
fn relay_readme_documents_fail_closed_launchdarkly_startup_contract() {
    let readme = std::fs::read_to_string(crate_readme()).expect("read crate README");

    assert!(
        readme.contains("start_with_default_executor_and_wait"),
        "README.md must show LaunchDarkly startup via start_with_default_executor_and_wait"
    );
    assert!(
        readme.contains("async fn build_feature_flag_evaluator"),
        "README.md must declare LaunchDarkly build_feature_flag_evaluator as async"
    );
    assert!(
        readme.contains(
            "start_with_default_executor_and_wait(Duration::from_secs(5))\n        .await"
        ),
        "README.md must await start_with_default_executor_and_wait"
    );
    assert!(
        readme.contains("fail-closed"),
        "README.md must state fail-closed startup policy"
    );
    assert!(
        readme.contains("exact-one compile_error guards"),
        "README.md must require exact-one compile_error guards in the consumer"
    );
    assert!(
        readme.contains("future flag registry"),
        "README.md must defer environment-name uniqueness enforcement to a future registry"
    );
}

#[test]
fn relay_artifact_provider_features_require_exactly_one_selection() {
    assert!(
        fixture_lockfile().is_file(),
        "relay feature selection fixture must check in Cargo.lock for locked nested cargo runs\nregenerate it with `{FIXTURE_LOCK_REGEN_RECIPE}`"
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
