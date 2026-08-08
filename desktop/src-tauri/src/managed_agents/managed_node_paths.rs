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
        match (std::env::consts::OS, target_arch()) {
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

/// Detect the actual target architecture at runtime.
///
/// On Windows ARM64, an x64-built Buzz binary runs under emulation with
/// `std::env::consts::ARCH` returning "x86_64" (compile-time constant).
/// However, npm packages are installed with the system (ARM64) Node's
/// native modules, so we must provision win-arm64 Node to match.
///
/// On non-Windows platforms, compile-time detection is correct.
#[cfg(target_os = "windows")]
pub(crate) fn target_arch() -> &'static str {
    windows_runtime_arch()
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn target_arch() -> &'static str {
    std::env::consts::ARCH
}

/// IMAGE_FILE_MACHINE_* constants (stable Win32 ABI values). Defined locally
/// because the `windows-sys` feature gate owning them is off in this crate.
#[cfg(target_os = "windows")]
const IMAGE_FILE_MACHINE_ARM64: u16 = 0xAA64;
#[cfg(target_os = "windows")]
const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;

/// On Windows, detect the actual machine architecture using IsWow64Process2.
/// Returns "aarch64" for ARM64 machines, "x86_64" otherwise. Any failure
/// (function missing pre-Win10-1709, call error) falls back to the
/// compile-time arch so behavior is unchanged from before this fix.
#[cfg(target_os = "windows")]
fn windows_runtime_arch() -> &'static str {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};

    let mut process_machine: u16 = 0;
    let mut native_machine: u16 = 0;

    unsafe {
        if IsWow64Process2(GetCurrentProcess(), &mut process_machine, &mut native_machine) != 0 {
            match native_machine {
                IMAGE_FILE_MACHINE_ARM64 => return "aarch64",
                IMAGE_FILE_MACHINE_AMD64 => return "x86_64",
                _ => return std::env::consts::ARCH,
            }
        }
    }

    std::env::consts::ARCH
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

    /// On non-Windows platforms, runtime arch detection must defer to the
    /// compile-time value (cf. `windows_runtime_arch()` on Windows).
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn target_arch_returns_compile_time_value_on_non_windows() {
        assert_eq!(target_arch(), std::env::consts::ARCH);
    }

    /// On Windows, runtime arch detection must return a supported value and
    /// never panic. On x64 hardware under native execution it must return
    /// `"x86_64"`; on ARM64 hardware under emulation it must return `"aarch64"`.
    #[cfg(target_os = "windows")]
    #[test]
    fn target_arch_returns_known_value_on_windows() {
        let arch = target_arch();
        assert!(
            matches!(arch, "x86_64" | "aarch64"),
            "unexpected runtime arch: {arch}"
, "unexpected runtime arch: {arch}"
        );
    }
}
