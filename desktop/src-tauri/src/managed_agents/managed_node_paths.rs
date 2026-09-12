use std::path::{Path, PathBuf};

/// Absolute directory containing a packager-supplied Node.js runtime.
///
/// Setting this bypasses Buzz's SHA-pinned Node.js download, so it must only
/// point to a trusted directory containing both `node` and `npm`.
pub(crate) const BUZZ_NODE_BIN_DIR_ENV: &str = "BUZZ_NODE_BIN_DIR";

pub(crate) fn buzz_managed_npm_prefix() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("Buzz").join("node-tools"))
}

const BUZZ_MANAGED_NODE_VERSION: &str = "v24.18.0";

pub(crate) fn buzz_managed_node_root() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("Buzz").join("runtimes").join("node"))
}

fn default_managed_node_bin_dir() -> Option<PathBuf> {
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
    buzz_managed_node_root().map(|root| {
        let dir = root.join(BUZZ_MANAGED_NODE_VERSION).join(platform);
        match bin_subdir {
            Some(sub) => dir.join(sub),
            None => dir,
        }
    })
}

pub(crate) fn validate_node_bin_dir(dir: &Path) -> Result<PathBuf, String> {
    if !dir.is_absolute() {
        return Err(format!("{BUZZ_NODE_BIN_DIR_ENV} must be an absolute path"));
    }
    if !dir.is_dir() {
        return Err(format!(
            "{BUZZ_NODE_BIN_DIR_ENV} is not a directory: {}",
            dir.display()
        ));
    }

    #[cfg(windows)]
    let required = ["node.exe", "npm.cmd", "npm"];
    #[cfg(not(windows))]
    let required = ["node", "npm"];

    for command in required {
        let path = dir.join(command);
        if !is_executable_file(&path) {
            return Err(format!(
                "{BUZZ_NODE_BIN_DIR_ENV} must contain an executable {command}: {}",
                path.display()
            ));
        }
    }

    Ok(dir.to_path_buf())
}

pub(crate) fn buzz_node_bin_dir_override() -> Result<Option<PathBuf>, String> {
    std::env::var_os(BUZZ_NODE_BIN_DIR_ENV)
        .map(|value| validate_node_bin_dir(Path::new(&value)))
        .transpose()
}

pub(crate) fn buzz_node_bin_dir_override_is_set() -> bool {
    std::env::var_os(BUZZ_NODE_BIN_DIR_ENV).is_some()
}

pub(crate) fn buzz_managed_node_bin_dir() -> Option<PathBuf> {
    match buzz_node_bin_dir_override() {
        Ok(Some(dir)) => Some(dir),
        Ok(None) => default_managed_node_bin_dir(),
        Err(_) => None,
    }
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
