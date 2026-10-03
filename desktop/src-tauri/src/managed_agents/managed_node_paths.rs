use std::path::PathBuf;

pub(crate) fn buzz_managed_npm_prefix() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("Buzz").join("node-tools"))
}

const BUZZ_MANAGED_NODE_VERSION: &str = "v24.18.0";

pub(crate) fn buzz_managed_node_root() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("Buzz").join("runtimes").join("node"))
}

pub(crate) fn buzz_managed_node_bin_dir() -> Option<PathBuf> {
    let (platform, bin_subdir): (&str, Option<&str>) =
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => ("darwin-arm64", Some("bin")),
            ("macos", "x86_64") => ("darwin-x64", Some("bin")),
            ("linux", "x86_64") => ("linux-x64", Some("bin")),
            ("linux", "aarch64") => ("linux-arm64", Some("bin")),
            // Windows zips have node.exe + npm.cmd at the archive root — no bin/ subdir
            ("windows", "x86_64") => ("win-x64", None),
            ("windows", "aarch64") => ("win-arm64", None),
            _ => return None,
        };
    buzz_managed_node_root()
        .map(|root| select_node_bin_dir(&root, BUZZ_MANAGED_NODE_VERSION, platform, bin_subdir))
}

/// Pick the managed Node bin dir: the pinned version when its `node` binary is
/// on disk, otherwise the newest installed runtime under `root`.
///
/// An app update that bumps `BUZZ_MANAGED_NODE_VERSION` leaves the previously
/// downloaded runtime on disk under its old version dir. Until the new version
/// is downloaded, the pinned dir does not exist — resolving to it puts a dead
/// entry on every managed-agent PATH, and when the login-shell PATH probe also
/// yields no `node` (e.g. `brew shellenv` lives in `.zshrc`, which `$SHELL -l
/// -c` never sources), every agent dies at spawn with `env: node: No such file
/// or directory`. Falling back to the installed runtime keeps agents working
/// while the pinned version is (re)installed; the readiness probe still
/// compares `node --version` against the pin, so the upgrade path is
/// unaffected.
///
/// Returns the pinned dir when nothing usable is installed, preserving the
/// invariant that resolution is `Some` on every supported platform — callers
/// use `is_none()` as a platform-support check and as the install destination.
fn select_node_bin_dir(
    root: &std::path::Path,
    pinned_version: &str,
    platform: &str,
    bin_subdir: Option<&str>,
) -> PathBuf {
    let pinned_dir = version_bin_dir(root, pinned_version, platform, bin_subdir);
    if has_node_binary(&pinned_dir) {
        return pinned_dir;
    }

    let mut installed: Vec<((u64, u64, u64), PathBuf)> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let version = parse_node_version(&name)?;
            let dir = version_bin_dir(root, &name, platform, bin_subdir);
            has_node_binary(&dir).then_some((version, dir))
        })
        .collect();
    installed.sort();
    installed
        .pop()
        .map(|(_, dir)| dir)
        .unwrap_or(pinned_dir)
}

fn version_bin_dir(
    root: &std::path::Path,
    version: &str,
    platform: &str,
    bin_subdir: Option<&str>,
) -> PathBuf {
    let dir = root.join(version).join(platform);
    match bin_subdir {
        Some(sub) => dir.join(sub),
        None => dir,
    }
}

fn has_node_binary(bin_dir: &std::path::Path) -> bool {
    #[cfg(windows)]
    let node = bin_dir.join("node.exe");
    #[cfg(not(windows))]
    let node = bin_dir.join("node");
    is_executable_file(&node)
}

/// Parse a Node dist dir name (`v24.18.0`) into a numerically comparable
/// triple. Plain lexicographic ordering would rank `v24.9.0` above `v24.11.0`.
fn parse_node_version(name: &str) -> Option<(u64, u64, u64)> {
    let rest = name.strip_prefix('v')?;
    let mut parts = rest.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub(crate) fn buzz_managed_node_bin_path() -> Option<PathBuf> {
    buzz_managed_node_bin_dir().map(|bin| {
        #[cfg(windows)]
        {
            bin.join("node.exe")
        }
        #[cfg(not(windows))]
        {
            bin.join("node")
        }
    })
}

pub(crate) fn buzz_managed_npm_bin_dir() -> Option<PathBuf> {
    buzz_managed_npm_prefix().map(|prefix| {
        #[cfg(windows)]
        {
            prefix
        }
        #[cfg(not(windows))]
        {
            prefix.join("bin")
        }
    })
}

pub(crate) fn buzz_managed_command_path(command: &str, basename: &str) -> Option<PathBuf> {
    if command.contains(std::path::MAIN_SEPARATOR)
        || !matches!(
            command,
            "codex-acp" | "claude-agent-acp" | "claude-code-acp" | "node" | "npm"
        )
    {
        return None;
    }

    let mut dirs = Vec::new();
    if let Some(managed_bin) = buzz_managed_npm_bin_dir() {
        dirs.push(managed_bin);
    }
    if let Some(managed_node_bin) = buzz_managed_node_bin_dir() {
        dirs.push(managed_node_bin);
    }

    dirs.into_iter()
        .map(|dir| dir.join(basename))
        .find(|candidate| is_executable_file(candidate))
}

fn is_executable_file(path: &std::path::Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_node_version, select_node_bin_dir};
    use std::path::Path;

    #[test]
    fn parse_node_version_accepts_dist_dir_names() {
        assert_eq!(parse_node_version("v24.18.0"), Some((24, 18, 0)));
        assert_eq!(parse_node_version("v0.10.48"), Some((0, 10, 48)));
    }

    #[test]
    fn parse_node_version_rejects_non_version_names() {
        assert_eq!(parse_node_version("24.18.0"), None, "missing v prefix");
        assert_eq!(parse_node_version("v24.18"), None, "too few components");
        assert_eq!(parse_node_version("v24.18.0.1"), None, "too many components");
        assert_eq!(parse_node_version("v24.18.0-rc1"), None, "non-numeric patch");
        assert_eq!(parse_node_version("tmp"), None);
        assert_eq!(parse_node_version(""), None);
    }

    #[test]
    fn parse_node_version_orders_numerically_not_lexicographically() {
        // The regression this guards: "v24.9.0" > "v24.11.0" as strings.
        assert!(parse_node_version("v24.9.0") < parse_node_version("v24.11.0"));
    }

    /// Plant a fake runtime layout `root/<version>/<platform>/bin/node` with the
    /// executable bit set, mirroring the extracted nodejs.org tarball shape.
    #[cfg(unix)]
    fn plant_runtime(root: &Path, version: &str) {
        use std::os::unix::fs::PermissionsExt;
        let bin = root.join(version).join("darwin-arm64").join("bin");
        std::fs::create_dir_all(&bin).expect("create bin dir");
        let node = bin.join("node");
        std::fs::write(&node, "#!/bin/sh\n").expect("write node");
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755))
            .expect("chmod node");
    }

    #[cfg(unix)]
    fn select(root: &Path, pinned: &str) -> std::path::PathBuf {
        select_node_bin_dir(root, pinned, "darwin-arm64", Some("bin"))
    }

    #[cfg(unix)]
    #[test]
    fn pinned_version_wins_when_installed() {
        let temp = tempfile::tempdir().expect("tempdir");
        plant_runtime(temp.path(), "v24.18.0");
        plant_runtime(temp.path(), "v24.11.0");
        let result = select(temp.path(), "v24.18.0");
        assert!(result.starts_with(temp.path().join("v24.18.0")), "{result:?}");
    }

    #[cfg(unix)]
    #[test]
    fn missing_pinned_falls_back_to_newest_installed() {
        let temp = tempfile::tempdir().expect("tempdir");
        // v24.9.0 sorts above v24.11.0 lexicographically — numeric order must win.
        plant_runtime(temp.path(), "v24.9.0");
        plant_runtime(temp.path(), "v24.11.0");
        let result = select(temp.path(), "v24.18.0");
        assert!(result.starts_with(temp.path().join("v24.11.0")), "{result:?}");
    }

    #[cfg(unix)]
    #[test]
    fn fallback_skips_dirs_without_a_node_binary() {
        let temp = tempfile::tempdir().expect("tempdir");
        plant_runtime(temp.path(), "v24.11.0");
        // Higher version present but empty (e.g. interrupted extraction).
        std::fs::create_dir_all(temp.path().join("v99.0.0").join("darwin-arm64").join("bin"))
            .expect("create empty runtime dir");
        let result = select(temp.path(), "v24.18.0");
        assert!(result.starts_with(temp.path().join("v24.11.0")), "{result:?}");
    }

    #[cfg(unix)]
    #[test]
    fn fallback_ignores_non_version_dirs() {
        let temp = tempfile::tempdir().expect("tempdir");
        plant_runtime(temp.path(), "v24.11.0");
        plant_runtime(temp.path(), "zz-not-a-version");
        let result = select(temp.path(), "v24.18.0");
        assert!(result.starts_with(temp.path().join("v24.11.0")), "{result:?}");
    }

    #[cfg(unix)]
    #[test]
    fn returns_pinned_dir_when_nothing_installed() {
        // Callers rely on Some(pinned) as the platform-support signal and the
        // install destination, even when nothing exists on disk yet.
        let temp = tempfile::tempdir().expect("tempdir");
        let result = select(temp.path(), "v24.18.0");
        assert!(result.starts_with(temp.path().join("v24.18.0")), "{result:?}");
    }

    #[cfg(unix)]
    #[test]
    fn returns_pinned_dir_when_root_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("does-not-exist");
        let result = select(&root, "v24.18.0");
        assert!(result.starts_with(root.join("v24.18.0")), "{result:?}");
    }
}
