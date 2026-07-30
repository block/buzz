use std::path::Path;
use std::process::Command;

use crate::managed_agents::AcpAvailabilityStatus;

pub(crate) const MIN_GOOSE_VERSION: (u64, u64, u64) = (1, 44, 0);
pub(crate) const MIN_GOOSE_VERSION_DISPLAY: &str = "1.44.0";

/// The oldest `codex-acp` version supported by Buzz managed agents.
///
/// Older 1.x adapters are detected successfully, but can still bundle a Codex runtime
/// that does not reliably give `buzz` CLI subprocesses outbound relay access.
///
/// Bump policy: raise this only when a newer adapter fixes a defect that breaks managed
/// agents, and only to a version already published on npm — every user below the floor is
/// offered a reinstall on their next discovery pass.
pub(crate) const MIN_CODEX_ACP_VERSION: (u64, u64, u64) = (1, 1, 7);

pub(super) fn parse_goose_version_output(output: &str) -> Option<(u64, u64, u64)> {
    let version = output.split_whitespace().last()?;
    let version = version.strip_prefix('v').unwrap_or(version);
    let version = semver::Version::parse(version).ok()?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return None;
    }
    Some((version.major, version.minor, version.patch))
}

pub(super) fn format_version_tuple(version: (u64, u64, u64)) -> String {
    format!("{}.{}.{}", version.0, version.1, version.2)
}

/// Probe the full version of a `codex-acp` binary by running `--version`.
///
/// The 1.x adapter (`@agentclientprotocol/codex-acp`) outputs
/// `@agentclientprotocol/codex-acp <major>.<minor>.<patch>` on stdout and exits 0.
/// The old 0.16.x adapter (`@zed-industries/codex-acp`) is a Rust binary that does
/// not recognise `--version` and exits non-zero.
///
/// Returns the `(major, minor, patch)` triple on success, `None` on any failure
/// (non-zero exit, unparseable output, timeout, or missing binary).
///
/// The parse is deliberately strict: exactly three numeric dot-separated components.
/// Partial versions (`1.2`) and prerelease tags (`1.2.0-rc1`) return `None` and so
/// classify as [`AcpAvailabilityStatus::AdapterOutdated`] — failing closed offers a
/// reinstall rather than running an adapter whose version cannot be compared.
///
/// The probe is bounded by a 5-second deadline. The child is polled with
/// [`std::process::Child::try_wait`] (the repo's standard deadline pattern) and
/// killed if it does not exit in time.
///
/// Stdout is redirected to a temporary file rather than a pipe, so forked
/// descendants cannot hold EOF open. Reads from a regular file return EOF at its
/// current write position regardless of inherited file descriptors, cross-platform.
pub(crate) fn probe_codex_acp_version(binary_path: &Path) -> Option<(u64, u64, u64)> {
    probe_codex_acp_version_with_path(
        binary_path,
        crate::managed_agents::readiness::cli_probe::augmented_path().as_deref(),
    )
}

pub(crate) fn probe_codex_acp_version_with_path(
    binary_path: &Path,
    augmented_path: Option<&str>,
) -> Option<(u64, u64, u64)> {
    probe_cli_version_with_path(binary_path, augmented_path, |stdout| {
        // Output format: "<package-name> <major>.<minor>.<patch>"
        let version_str = stdout.split_whitespace().last()?;
        let mut components = version_str.split('.');
        let major = components.next()?.parse::<u64>().ok()?;
        let minor = components.next()?.parse::<u64>().ok()?;
        let patch = components.next()?.parse::<u64>().ok()?;
        if components.next().is_some() {
            return None;
        }
        Some((major, minor, patch))
    })
}

pub(crate) fn probe_goose_version(binary_path: &Path) -> Option<(u64, u64, u64)> {
    probe_goose_version_with_path(
        binary_path,
        crate::managed_agents::readiness::cli_probe::augmented_path().as_deref(),
    )
}

pub(crate) fn probe_goose_version_with_path(
    binary_path: &Path,
    augmented_path: Option<&str>,
) -> Option<(u64, u64, u64)> {
    probe_cli_version_with_path(binary_path, augmented_path, parse_goose_version_output)
}

fn probe_cli_version_with_path<F>(
    binary_path: &Path,
    augmented_path: Option<&str>,
    parse_stdout: F,
) -> Option<(u64, u64, u64)>
where
    F: FnOnce(&str) -> Option<(u64, u64, u64)>,
{
    use std::io::{Read as _, Seek as _, SeekFrom};
    use std::time::{Duration, Instant};
    const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

    // A regular file returns EOF at its current size even when a descendant
    // inherits its descriptor, bounding the post-exit read cross-platform.
    let mut tmp = tempfile::tempfile().ok()?;

    let mut command = Command::new(binary_path);
    command.arg("--version");
    if let Some(path) = augmented_path {
        command.env("PATH", path);
    }
    crate::util::configure_no_window(&mut command);
    let mut child = command
        .stdout(tmp.try_clone().ok()?)
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    // Poll until the deadline rather than blocking on stdout EOF.
    let deadline = Instant::now() + VERSION_PROBE_TIMEOUT;
    let exit_status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };

    if !exit_status.success() {
        return None;
    }

    // Read at most 4 KiB from the regular file without blocking.
    tmp.seek(SeekFrom::Start(0)).ok()?;
    let mut buf = Vec::with_capacity(128);
    let _ = (&mut tmp as &mut dyn std::io::Read)
        .take(4096)
        .read_to_end(&mut buf);

    let stdout = String::from_utf8_lossy(&buf);
    parse_stdout(&stdout)
}

/// Classifies a resolved codex-acp binary path as [`AcpAvailabilityStatus::Available`]
/// or [`AcpAvailabilityStatus::AdapterOutdated`].
///
/// The 0.16.x adapter (`@zed-industries/codex-acp`) does not recognise `--version`
/// and exits non-zero — that probe failure yields `AdapterOutdated`. An adapter is
/// available only when its version is at least [`MIN_CODEX_ACP_VERSION`].
///
/// Used by `discover_acp_runtimes`, `cli_login_requirements`, and
/// `install_acp_runtime_blocking` so the version-gate logic is not duplicated.
pub(crate) fn codex_adapter_availability(path: &Path) -> AcpAvailabilityStatus {
    match probe_codex_acp_version(path) {
        Some(version) if version >= MIN_CODEX_ACP_VERSION => AcpAvailabilityStatus::Available,
        _ => AcpAvailabilityStatus::AdapterOutdated,
    }
}

pub(crate) fn goose_cli_availability(path: &Path) -> AcpAvailabilityStatus {
    match probe_goose_version(path) {
        Some(version) if version >= MIN_GOOSE_VERSION => AcpAvailabilityStatus::Available,
        _ => AcpAvailabilityStatus::CliOutdated,
    }
}

/// Returns `true` when the codex-acp binary at `path` is below
/// [`MIN_CODEX_ACP_VERSION`] or cannot be probed using `augmented_path`. Thin wrapper
/// around [`codex_adapter_is_outdated_with_path`].
#[cfg(test)]
pub(crate) fn codex_adapter_is_outdated(path: &Path) -> bool {
    codex_adapter_is_outdated_with_path(
        path,
        crate::managed_agents::readiness::cli_probe::augmented_path().as_deref(),
    )
}

/// Returns `true` when the codex-acp binary at `path` is below
/// [`MIN_CODEX_ACP_VERSION`] or cannot be probed with the supplied PATH.
pub(crate) fn codex_adapter_is_outdated_with_path(
    path: &Path,
    augmented_path: Option<&str>,
) -> bool {
    !matches!(
        probe_codex_acp_version_with_path(path, augmented_path),
        Some(version) if version >= MIN_CODEX_ACP_VERSION
    )
}
