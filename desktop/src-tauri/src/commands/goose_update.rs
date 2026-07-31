use std::{
    io::{Read as _, Seek as _, SeekFrom},
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

use semver::Version;
use serde::Serialize;

const LATEST_GOOSE_RELEASE_URL: &str = "https://github.com/aaif-goose/goose/releases/latest";
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const RELEASE_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// Read-only comparison between the installed Goose CLI and the latest stable
/// Goose release.
///
/// This is intentionally separate from runtime availability: failure to check
/// for an update must never make an installed Goose runtime unavailable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum GooseUpdateStatus {
    /// The installed version is equal to or newer than the latest stable release.
    UpToDate {
        installed_version: String,
        latest_version: String,
    },
    /// A newer stable release is available for the installed Goose CLI.
    UpdateAvailable {
        installed_version: String,
        latest_version: String,
    },
}

/// Check whether the resolved Goose CLI is behind the latest stable release.
///
/// The command is called only by Settings. It never runs `goose update` and
/// therefore cannot modify the installed binary.
#[tauri::command]
pub async fn check_goose_update_status() -> Result<GooseUpdateStatus, String> {
    let installed_version = tokio::task::spawn_blocking(|| {
        let path = crate::managed_agents::resolve_command("goose")
            .ok_or_else(|| "Goose is not installed.".to_string())?;
        probe_goose_version(&path)
    })
    .await
    .map_err(|error| format!("Goose version probe task failed: {error}"))??;

    let client = reqwest::Client::builder()
        .timeout(RELEASE_CHECK_TIMEOUT)
        .user_agent("buzz-desktop")
        .build()
        .map_err(|error| format!("Could not create the Goose update client: {error}"))?;
    let latest_version = fetch_latest_stable_version(&client, LATEST_GOOSE_RELEASE_URL).await?;

    Ok(classify_versions(installed_version, latest_version))
}

fn probe_goose_version(path: &Path) -> Result<Version, String> {
    probe_goose_version_with_timeout(path, VERSION_PROBE_TIMEOUT)
}

fn probe_goose_version_with_timeout(path: &Path, timeout: Duration) -> Result<Version, String> {
    let mut stdout = tempfile::tempfile()
        .map_err(|error| format!("Could not capture Goose version output: {error}"))?;
    let stderr = tempfile::tempfile()
        .map_err(|error| format!("Could not capture Goose version errors: {error}"))?;

    let mut command = Command::new(path);
    command
        .arg("--version")
        .stdout(
            stdout
                .try_clone()
                .map_err(|error| format!("Could not capture Goose version output: {error}"))?,
        )
        .stderr(stderr);
    crate::util::configure_no_window(&mut command);

    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not run {} --version: {error}", path.display()))?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{} --version timed out after {} seconds.",
                    path.display(),
                    timeout.as_secs()
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "Could not wait for {} --version: {error}",
                    path.display()
                ));
            }
        }
    };

    if !status.success() {
        return Err(format!(
            "{} --version exited with {status}.",
            path.display()
        ));
    }

    stdout
        .seek(SeekFrom::Start(0))
        .map_err(|error| format!("Could not read Goose version output: {error}"))?;
    let mut output = String::new();
    (&mut stdout as &mut dyn std::io::Read)
        .take(4096)
        .read_to_string(&mut output)
        .map_err(|error| format!("Could not read Goose version output: {error}"))?;

    parse_installed_version(&output)
}

fn parse_installed_version(output: &str) -> Result<Version, String> {
    let raw = output
        .split_whitespace()
        .last()
        .ok_or_else(|| "Goose did not report an installed version.".to_string())?;
    let version = raw.strip_prefix('v').unwrap_or(raw);
    Version::parse(version)
        .map_err(|error| format!("Could not parse installed Goose version {raw:?}: {error}"))
}

async fn fetch_latest_stable_version(
    client: &reqwest::Client,
    release_url: &str,
) -> Result<Version, String> {
    let response = client
        .head(release_url)
        .send()
        .await
        .map_err(|error| format!("Could not check the latest Goose release: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Goose release check returned HTTP {}.",
            response.status()
        ));
    }

    parse_latest_release_url(response.url())
}

fn parse_latest_release_url(url: &reqwest::Url) -> Result<Version, String> {
    if url.host_str() != Some("github.com") {
        return Err(format!(
            "Goose release check redirected to an unexpected host: {url}"
        ));
    }

    let segments = url
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
        .unwrap_or_default();
    let [owner, repository, releases, tag, raw_version] = segments.as_slice() else {
        return Err(format!(
            "Goose release check returned an unexpected URL: {url}"
        ));
    };
    if (*owner, *repository, *releases, *tag) != ("aaif-goose", "goose", "releases", "tag") {
        return Err(format!(
            "Goose release check returned an unexpected URL: {url}"
        ));
    }

    let version = raw_version.strip_prefix('v').unwrap_or(raw_version);
    Version::parse(version)
        .map_err(|error| format!("Could not parse latest Goose version {raw_version:?}: {error}"))
}

fn classify_versions(installed: Version, latest: Version) -> GooseUpdateStatus {
    let installed_version = installed.to_string();
    let latest_version = latest.to_string();

    if installed < latest {
        GooseUpdateStatus::UpdateAvailable {
            installed_version,
            latest_version,
        }
    } else {
        GooseUpdateStatus::UpToDate {
            installed_version,
            latest_version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_installed_goose_version() {
        assert_eq!(
            parse_installed_version("1.44.0\n").expect("plain version"),
            Version::new(1, 44, 0)
        );
        assert_eq!(
            parse_installed_version("goose v1.45.0\n").expect("prefixed version"),
            Version::new(1, 45, 0)
        );
    }

    #[test]
    fn rejects_malformed_installed_version() {
        let error = parse_installed_version("goose latest").expect_err("must reject");
        assert!(error.contains("Could not parse installed Goose version"));
    }

    #[test]
    fn parses_official_latest_release_redirect() {
        let url = reqwest::Url::parse("https://github.com/aaif-goose/goose/releases/tag/v1.45.0")
            .expect("valid URL");

        assert_eq!(
            parse_latest_release_url(&url).expect("official release"),
            Version::new(1, 45, 0)
        );
    }

    #[test]
    fn rejects_unexpected_release_redirects() {
        for raw in [
            "https://example.com/aaif-goose/goose/releases/tag/v1.45.0",
            "https://github.com/other/goose/releases/tag/v1.45.0",
            "https://github.com/aaif-goose/goose/releases/latest",
            "https://github.com/aaif-goose/goose/releases/tag/stable",
        ] {
            let url = reqwest::Url::parse(raw).expect("valid URL");
            assert!(parse_latest_release_url(&url).is_err(), "must reject {raw}");
        }
    }

    #[test]
    fn classifies_behind_equal_and_newer_versions() {
        assert!(matches!(
            classify_versions(Version::new(1, 44, 0), Version::new(1, 45, 0)),
            GooseUpdateStatus::UpdateAvailable { .. }
        ));
        assert!(matches!(
            classify_versions(Version::new(1, 45, 0), Version::new(1, 45, 0)),
            GooseUpdateStatus::UpToDate { .. }
        ));
        assert!(matches!(
            classify_versions(Version::new(1, 46, 0), Version::new(1, 45, 0)),
            GooseUpdateStatus::UpToDate { .. }
        ));
    }

    #[test]
    fn stable_release_supersedes_same_version_prerelease() {
        let installed = Version::parse("1.45.0-rc.1").expect("valid prerelease");
        let latest = Version::new(1, 45, 0);

        assert!(matches!(
            classify_versions(installed, latest),
            GooseUpdateStatus::UpdateAvailable { .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn probes_resolved_goose_executable() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("temp dir");
        let executable = dir.path().join("goose");
        std::fs::write(&executable, "#!/bin/sh\necho 1.44.0\n").expect("write fake Goose");
        let mut permissions = std::fs::metadata(&executable)
            .expect("fake Goose metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).expect("make fake Goose executable");

        assert_eq!(
            probe_goose_version_with_timeout(&executable, Duration::from_secs(3))
                .expect("version probe"),
            Version::new(1, 44, 0)
        );
    }

    #[cfg(unix)]
    #[test]
    fn reports_failed_version_probe() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("temp dir");
        let executable = dir.path().join("goose");
        std::fs::write(&executable, "#!/bin/sh\nexit 2\n").expect("write failing fake Goose");
        let mut permissions = std::fs::metadata(&executable)
            .expect("fake Goose metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).expect("make fake Goose executable");

        let error = probe_goose_version_with_timeout(&executable, Duration::from_secs(3))
            .expect_err("probe must fail");
        assert!(error.contains("exited with"));
    }

    #[cfg(unix)]
    #[test]
    fn bounds_hung_version_probe() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("temp dir");
        let executable = dir.path().join("goose");
        std::fs::write(&executable, "#!/bin/sh\nexec sleep 10\n").expect("write hung fake Goose");
        let mut permissions = std::fs::metadata(&executable)
            .expect("fake Goose metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).expect("make fake Goose executable");

        let error = probe_goose_version_with_timeout(&executable, Duration::from_millis(100))
            .expect_err("probe must time out");
        assert!(error.contains("timed out"));
    }
}
