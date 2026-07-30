use super::super::{
    goose_cli_availability, parse_goose_version_output, probe_goose_version_with_path,
    MIN_GOOSE_VERSION, MIN_GOOSE_VERSION_DISPLAY,
};
use crate::managed_agents::AcpAvailabilityStatus;

#[test]
fn goose_min_version_constants_are_consistent() {
    assert_eq!(MIN_GOOSE_VERSION, (1, 44, 0));
    assert_eq!(MIN_GOOSE_VERSION_DISPLAY, "1.44.0");
}

#[test]
fn parse_goose_version_output_accepts_expected_shapes() {
    assert_eq!(parse_goose_version_output("1.44.0"), Some((1, 44, 0)));
    assert_eq!(parse_goose_version_output("v1.44.0"), Some((1, 44, 0)));
    assert_eq!(parse_goose_version_output("goose 1.44.0"), Some((1, 44, 0)));
    assert_eq!(
        parse_goose_version_output("goose-cli 1.44.0"),
        Some((1, 44, 0))
    );
}

#[test]
fn parse_goose_version_output_rejects_unusable_shapes() {
    assert_eq!(parse_goose_version_output("goose 1.44"), None);
    assert_eq!(parse_goose_version_output("goose 1.44.0-rc1"), None);
    assert_eq!(parse_goose_version_output("goose version unknown"), None);
}

#[cfg(unix)]
#[test]
fn probe_goose_version_parses_goose_version_output() {
    let temp = tempfile::tempdir().expect("temp dir");
    let goose_path = write_goose_shim(temp.path(), "goose 1.44.0");

    assert_eq!(
        probe_goose_version_with_path(&goose_path, None),
        Some((1, 44, 0))
    );
}

#[cfg(unix)]
#[test]
fn goose_cli_availability_accepts_minimum_supported_version() {
    let temp = tempfile::tempdir().expect("temp dir");
    let goose_path = write_goose_shim(temp.path(), "goose 1.44.0");

    assert_eq!(
        goose_cli_availability(&goose_path),
        AcpAvailabilityStatus::Available
    );
}

#[cfg(unix)]
#[test]
fn goose_cli_availability_rejects_older_versions() {
    let temp = tempfile::tempdir().expect("temp dir");
    let goose_path = write_goose_shim(temp.path(), "goose 1.43.9");

    assert_eq!(
        goose_cli_availability(&goose_path),
        AcpAvailabilityStatus::CliOutdated
    );
}

#[cfg(unix)]
fn write_goose_shim(dir: &std::path::Path, output: &str) -> std::path::PathBuf {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let goose_path = dir.join("goose");
    fs::write(&goose_path, format!("#!/bin/sh\necho '{output}'\n")).expect("write goose shim");
    fs::set_permissions(&goose_path, fs::Permissions::from_mode(0o755)).expect("chmod goose shim");
    goose_path
}
