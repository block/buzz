use std::path::PathBuf;

pub(crate) fn buzz_managed_npm_prefix() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("Buzz").join("node-tools"))
}

const BUZZ_MANAGED_NODE_VERSION: &str = "v24.11.0";

pub(crate) fn buzz_managed_node_root() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("Buzz").join("runtimes").join("node"))
}

/// Map `(OS, ARCH)` to the Node.js official platform directory name, or
/// `None` when Buzz does not ship a managed runtime for that host.
pub(crate) fn managed_node_platform(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some("darwin-arm64"),
        ("macos", "x86_64") => Some("darwin-x64"),
        ("linux", "x86_64") => Some("linux-x64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("windows", "x86_64") => Some("win-x64"),
        ("windows", "aarch64") => Some("win-arm64"),
        _ => None,
    }
}

pub(crate) fn buzz_managed_node_bin_dir() -> Option<PathBuf> {
    let platform = managed_node_platform(std::env::consts::OS, std::env::consts::ARCH)?;
    buzz_managed_node_root().map(|root| {
        let base = root.join(BUZZ_MANAGED_NODE_VERSION).join(platform);
        // Official Windows Node zips put `node.exe` at the platform root (no
        // `bin/` subdirectory). Unix tarballs keep the usual `bin/` layout.
        #[cfg(windows)]
        {
            base
        }
        #[cfg(not(windows))]
        {
            base.join("bin")
        }
    })
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
    use super::*;

    #[test]
    fn managed_node_platform_covers_supported_hosts() {
        assert_eq!(
            managed_node_platform("macos", "aarch64"),
            Some("darwin-arm64")
        );
        assert_eq!(managed_node_platform("macos", "x86_64"), Some("darwin-x64"));
        assert_eq!(managed_node_platform("linux", "x86_64"), Some("linux-x64"));
        assert_eq!(
            managed_node_platform("linux", "aarch64"),
            Some("linux-arm64")
        );
        assert_eq!(managed_node_platform("windows", "x86_64"), Some("win-x64"));
        assert_eq!(
            managed_node_platform("windows", "aarch64"),
            Some("win-arm64")
        );
        assert_eq!(managed_node_platform("windows", "x86"), None);
        assert_eq!(managed_node_platform("freebsd", "x86_64"), None);
    }
}
