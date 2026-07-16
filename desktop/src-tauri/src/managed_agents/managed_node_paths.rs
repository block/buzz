use std::path::PathBuf;

pub(crate) fn buzz_managed_npm_prefix() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("Buzz").join("node-tools"))
}

const BUZZ_MANAGED_NODE_VERSION: &str = "v24.11.0";

pub(crate) fn buzz_managed_node_root() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("Buzz").join("runtimes").join("node"))
}

pub(crate) fn buzz_managed_node_bin_dir() -> Option<PathBuf> {
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        _ => return None,
    };
    buzz_managed_node_root().map(|root| {
        root.join(BUZZ_MANAGED_NODE_VERSION)
            .join(platform)
            .join("bin")
    })
}

pub(crate) fn buzz_managed_node_bin_path() -> Option<PathBuf> {
    buzz_managed_node_bin_dir().map(|bin| bin.join("node"))
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
