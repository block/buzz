//! Git Bash discovery shared by Doctor and the buzz-agent readiness gate.
//!
//! The MCP child receives a deliberately small environment. Discovery inspects
//! exactly the shared resolver-key contract forwarded into that child, plus the
//! Git-for-Windows registry. A Doctor green state therefore means `buzz-dev-mcp`
//! can actually start its shell.

#[cfg(windows)]
use std::path::{Path, PathBuf};

/// A Git Bash installation the stripped MCP child can launch.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct GitBashPrerequisite {
    pub available: bool,
    pub path: Option<String>,
    pub install_instructions_url: String,
    pub install_hint: String,
}

#[cfg(windows)]
const INSTALL_URL: &str = "https://git-scm.com/download/win";
#[cfg(windows)]
const INSTALL_HINT: &str =
    "Install Git for Windows and select \"Git from the command line and also from 3rd-party software\" for its PATH option.";

/// Install hint for error messages when `install_shell_command` can't find a shell on Windows.
#[cfg(windows)]
pub(crate) const GIT_BASH_INSTALL_HINT: &str = INSTALL_HINT;

/// Resolve the Git Bash executable path using the same resolver chain as Doctor.
///
/// Returns `Some(path)` on Windows when a usable bash is found, `None` otherwise
/// (including all non-Windows platforms). Honors `BUZZ_SHELL` (any executable) —
/// correct for the Doctor readiness gate where any shell suffices.
#[allow(dead_code)] // used only on Windows, from discover_git_bash + git_bash_available
pub(crate) fn resolve_git_bash_path() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        let env = GitBashEnv::from_process();
        return resolve_git_bash(
            &env.path,
            env.shell_override,
            env.git_bash_override,
            env.system_root,
            env.program_files,
            env.program_files_x86,
            env.local_app_data,
        );
    }

    #[cfg(not(windows))]
    None
}

/// Resolve a bash-compatible shell for install commands and login-shell discovery.
///
/// Unlike `resolve_git_bash_path`, this skips `BUZZ_SHELL` entirely — that override
/// intentionally accepts any executable (`cmd`, `pwsh`) for the MCP child, but install
/// commands and `login_shell_candidates` use bash-only `-l -c` syntax. Skipping the
/// override means the chain falls through to: `GIT_BASH` → PATH scan → derive-from-git
/// → well-known locations → registry.
#[allow(dead_code)] // used only on Windows, from install_shell_command + login_shell_candidates
pub(crate) fn resolve_bash_path() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        let env = GitBashEnv::from_process();
        return resolve_git_bash(
            &env.path,
            None, // skip BUZZ_SHELL — install/login-shell callers require bash
            env.git_bash_override,
            env.system_root,
            env.program_files,
            env.program_files_x86,
            env.local_app_data,
        );
    }

    #[cfg(not(windows))]
    None
}

pub(crate) fn discover_git_bash() -> Option<GitBashPrerequisite> {
    #[cfg(windows)]
    {
        let env = GitBashEnv::from_process();
        let path = resolve_git_bash(
            &env.path,
            env.shell_override,
            env.git_bash_override,
            env.system_root,
            env.program_files,
            env.program_files_x86,
            env.local_app_data,
        );
        return Some(GitBashPrerequisite {
            available: path.is_some(),
            path: path.map(|path| path.display().to_string()),
            install_instructions_url: INSTALL_URL.to_string(),
            install_hint: INSTALL_HINT.to_string(),
        });
    }

    #[cfg(not(windows))]
    None
}

#[cfg(windows)]
pub(crate) fn git_bash_available(overrides: &std::collections::BTreeMap<String, String>) -> bool {
    let env = GitBashEnv::from_process_with_overrides(overrides);
    resolve_git_bash(
        &env.path,
        env.shell_override,
        env.git_bash_override,
        env.system_root,
        env.program_files,
        env.program_files_x86,
        env.local_app_data,
    )
    .is_some()
}

/// All process environment that Git Bash discovery may inspect. Its keys are
/// deliberately sourced from `buzz_agent_pkg::WINDOWS_SHELL_RESOLUTION_ENV`,
/// the exact allowlist forwarded to the otherwise-cleared MCP child.
#[cfg(windows)]
struct GitBashEnv {
    path: String,
    shell_override: Option<PathBuf>,
    git_bash_override: Option<PathBuf>,
    system_root: Option<PathBuf>,
    program_files: Option<PathBuf>,
    program_files_x86: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
}

#[cfg(windows)]
impl GitBashEnv {
    fn from_process() -> Self {
        Self::from_process_with_overrides(&Default::default())
    }

    fn from_process_with_overrides(overrides: &std::collections::BTreeMap<String, String>) -> Self {
        let values: std::collections::HashMap<_, _> = buzz_agent_pkg::WINDOWS_SHELL_RESOLUTION_ENV
            .iter()
            .filter_map(|key| {
                overrides
                    .iter()
                    .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
                    .map(|(_, value)| std::ffi::OsString::from(value))
                    .or_else(|| std::env::var_os(key))
                    .map(|value| (*key, value))
            })
            .collect();
        Self::from_lookup(|key| values.get(key).cloned())
    }

    fn from_lookup(mut get: impl FnMut(&str) -> Option<std::ffi::OsString>) -> Self {
        Self {
            path: get("PATH")
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            shell_override: get("BUZZ_SHELL").map(PathBuf::from),
            git_bash_override: get("GIT_BASH").map(PathBuf::from),
            system_root: get("SystemRoot").map(PathBuf::from),
            program_files: get("ProgramFiles").map(PathBuf::from),
            program_files_x86: get("ProgramFiles(x86)").map(PathBuf::from),
            local_app_data: get("LOCALAPPDATA").map(PathBuf::from),
        }
    }
}

#[cfg(windows)]
pub(crate) fn resolve_git_bash(
    path_env: &str,
    shell_override: Option<PathBuf>,
    git_bash_override: Option<PathBuf>,
    system_root: Option<PathBuf>,
    program_files: Option<PathBuf>,
    program_files_x86: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
) -> Option<PathBuf> {
    shell_override
        .and_then(|path| resolve_shell_override(&path, path_env))
        .or_else(|| git_bash_override.filter(|path| path.is_file()))
        .or_else(|| scan_path_for_bash(path_env, system_root.as_deref()))
        .or_else(|| {
            scan_path_for_command(Path::new("git.exe"), path_env, None)
                .and_then(|git| bash_from_git(&git))
        })
        .or_else(|| {
            git_bash_from_standard_paths([program_files, program_files_x86, local_app_data])
        })
        .or_else(git_bash_from_registry)
}

/// Resolve `BUZZ_SHELL` with the same rooted/bare-name semantics as the MCP
/// resolver. This intentionally accepts any executable shell; its presence is
/// sufficient for the MCP child and therefore for the readiness gate.
#[cfg(windows)]
fn resolve_shell_override(shell: &Path, path_env: &str) -> Option<PathBuf> {
    if shell.components().count() > 1 || shell.has_root() {
        shell.is_file().then(|| shell.to_path_buf())
    } else {
        scan_path_for_command(shell, path_env, None)
    }
}

#[cfg(windows)]
fn bash_from_git(git: &Path) -> Option<PathBuf> {
    let bash = git.parent()?.parent()?.join("bin").join("bash.exe");
    bash.is_file().then_some(bash)
}

#[cfg(windows)]
fn scan_path_for_bash(path_env: &str, system_root: Option<&Path>) -> Option<PathBuf> {
    scan_path_for_command(Path::new("bash.exe"), path_env, system_root)
}

#[cfg(windows)]
fn scan_path_for_command(
    name: &Path,
    path_env: &str,
    system_root: Option<&Path>,
) -> Option<PathBuf> {
    let needs_exe = name.extension().is_none();
    std::env::split_paths(path_env).find_map(|dir| {
        if system_root.is_some_and(|root| is_under_dir(&dir, root)) {
            return None;
        }
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        if needs_exe {
            let mut candidate = dir.join(name);
            candidate.set_extension("exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    })
}

#[cfg(windows)]
fn is_under_dir(dir: &Path, root: &Path) -> bool {
    let mut dir_components = dir.components();
    root.components().all(|root_component| {
        dir_components.next().is_some_and(|dir_component| {
            dir_component
                .as_os_str()
                .eq_ignore_ascii_case(root_component.as_os_str())
        })
    })
}

/// Probe machine and per-user Git for Windows registry keys after the standard
/// install-location fallback has been exhausted.
#[cfg(windows)]
fn git_bash_from_registry() -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
        KEY_READ,
    };

    const KEY: &str = "SOFTWARE\\GitForWindows";
    const VALUE: &str = "InstallPath";
    let key: Vec<u16> = KEY.encode_utf16().chain(Some(0)).collect();
    let value: Vec<u16> = VALUE.encode_utf16().chain(Some(0)).collect();

    // SAFETY: Inputs are null-terminated UTF-16 for the duration of each call,
    // and every successfully opened handle is closed before trying the next hive.
    unsafe {
        for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
            let mut handle = std::ptr::null_mut();
            if RegOpenKeyExW(hive, key.as_ptr(), 0, KEY_READ, &mut handle) != ERROR_SUCCESS {
                continue;
            }

            let mut byte_len = 0;
            let status = RegQueryValueExW(
                handle,
                value.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut byte_len,
            );
            if (status != ERROR_SUCCESS && status != ERROR_MORE_DATA) || byte_len == 0 {
                RegCloseKey(handle);
                continue;
            }

            let mut data = vec![0u16; (byte_len as usize).div_ceil(2)];
            let status = RegQueryValueExW(
                handle,
                value.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                data.as_mut_ptr().cast(),
                &mut byte_len,
            );
            RegCloseKey(handle);
            if status != ERROR_SUCCESS {
                continue;
            }

            while data.last() == Some(&0) {
                data.pop();
            }
            let bash = PathBuf::from(OsString::from_wide(&data))
                .join("bin")
                .join("bash.exe");
            if bash.is_file() {
                return Some(bash);
            }
        }
    }

    None
}

#[cfg(windows)]
fn git_bash_from_standard_paths(
    [program_files, program_files_x86, local_app_data]: [Option<PathBuf>; 3],
) -> Option<PathBuf> {
    [
        program_files.map(|base| base.join("Git")),
        program_files_x86.map(|base| base.join("Git")),
        local_app_data.map(|base| base.join("Programs").join("Git")),
    ]
    .into_iter()
    .flatten()
    .map(|install_root| install_root.join("bin").join("bash.exe"))
    .find(|bash| bash.is_file())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const DETECTOR_ENV_KEYS: &[&str] = &[
        "PATH",
        "BUZZ_SHELL",
        "GIT_BASH",
        "SystemRoot",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "LOCALAPPDATA",
    ];

    #[test]
    fn test_detector_env_keys_match_agent_shell_resolution_contract() {
        assert_eq!(
            DETECTOR_ENV_KEYS,
            buzz_agent_pkg::WINDOWS_SHELL_RESOLUTION_ENV,
            "Doctor and the env-cleared MCP child must inspect the same resolver inputs"
        );

        let env = GitBashEnv::from_lookup(|key| Some(key.into()));
        assert_eq!(env.path, "PATH");
        assert_eq!(env.shell_override, Some(PathBuf::from("BUZZ_SHELL")));
        assert_eq!(env.git_bash_override, Some(PathBuf::from("GIT_BASH")));
        assert_eq!(env.system_root, Some(PathBuf::from("SystemRoot")));
        assert_eq!(env.program_files, Some(PathBuf::from("ProgramFiles")));
        assert_eq!(
            env.program_files_x86,
            Some(PathBuf::from("ProgramFiles(x86)"))
        );
        assert_eq!(env.local_app_data, Some(PathBuf::from("LOCALAPPDATA")));
    }

    #[test]
    fn test_git_cmd_on_path_resolves_sibling_bash() {
        let temp = tempdir().expect("tempdir");
        let git = temp
            .path()
            .join("Program Files")
            .join("Git")
            .join("cmd")
            .join("git.exe");
        let bash = temp
            .path()
            .join("Program Files")
            .join("Git")
            .join("bin")
            .join("bash.exe");
        std::fs::create_dir_all(git.parent().expect("git parent")).expect("mkdir git");
        std::fs::create_dir_all(bash.parent().expect("bash parent")).expect("mkdir bash");
        std::fs::write(&git, []).expect("git");
        std::fs::write(&bash, []).expect("bash");

        let path = std::env::join_paths([git.parent().expect("cmd dir")]).expect("PATH");
        assert_eq!(
            resolve_git_bash(
                path.to_str().expect("utf8"),
                None,
                None,
                None,
                None,
                None,
                None
            ),
            Some(bash)
        );
    }

    #[test]
    fn test_program_files_x86_git_bash_resolves() {
        let temp = tempdir().expect("tempdir");
        let program_files_x86 = temp.path().join("Program Files (x86)");
        let bash = program_files_x86.join("Git").join("bin").join("bash.exe");
        std::fs::create_dir_all(bash.parent().expect("bash parent")).expect("mkdir bash");
        std::fs::write(&bash, []).expect("bash");

        assert_eq!(
            resolve_git_bash("", None, None, None, None, Some(program_files_x86), None,),
            Some(bash)
        );
    }

    #[test]
    fn test_effective_buzz_shell_override_marks_agent_ready() {
        let temp = tempdir().expect("tempdir");
        let shell = temp.path().join("pwsh.exe");
        std::fs::write(&shell, []).expect("shell");

        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert("buzz_shell".to_string(), shell.display().to_string());
        let env = GitBashEnv::from_process_with_overrides(&overrides);
        assert_eq!(env.shell_override, Some(shell.clone()));
        assert_eq!(
            resolve_git_bash(
                &env.path,
                env.shell_override,
                env.git_bash_override,
                env.system_root,
                env.program_files,
                env.program_files_x86,
                env.local_app_data,
            ),
            Some(shell)
        );
    }

    #[test]
    fn test_buzz_shell_override_wins_over_git_bash_discovery() {
        let temp = tempdir().expect("tempdir");
        let shell = temp.path().join("pwsh.exe");
        let bash = temp.path().join("bash.exe");
        std::fs::write(&shell, []).expect("shell");
        std::fs::write(&bash, []).expect("bash");

        let path = std::env::join_paths([temp.path()]).expect("PATH");
        assert_eq!(
            resolve_git_bash(
                path.to_str().expect("utf8"),
                Some(shell.clone()),
                Some(bash),
                None,
                None,
                None,
                None,
            ),
            Some(shell)
        );
    }

    // ── Regression: install/login-shell must skip non-bash BUZZ_SHELL ─────────

    /// When BUZZ_SHELL=pwsh.exe, `resolve_git_bash` with `shell_override=None`
    /// (the `resolve_bash_path` code path) skips it and falls through to the
    /// bash.exe on PATH. The readiness gate (`shell_override=Some`) still
    /// returns pwsh — both contracts hold simultaneously.
    #[test]
    fn test_install_path_skips_buzz_shell_pwsh() {
        let temp = tempdir().expect("tempdir");
        let pwsh = temp.path().join("pwsh.exe");
        let bash = temp.path().join("bash.exe");
        std::fs::write(&pwsh, []).expect("pwsh");
        std::fs::write(&bash, []).expect("bash");

        let path = std::env::join_paths([temp.path()]).expect("PATH");
        let path_str = path.to_str().expect("utf8");

        // Readiness gate: BUZZ_SHELL=pwsh accepted (Doctor green).
        assert_eq!(
            resolve_git_bash(path_str, Some(pwsh.clone()), None, None, None, None, None),
            Some(pwsh),
            "readiness gate must accept BUZZ_SHELL=pwsh"
        );

        // Install path: shell_override=None skips pwsh, finds bash on PATH.
        assert_eq!(
            resolve_git_bash(path_str, None, None, None, None, None, None),
            Some(bash),
            "install path must skip BUZZ_SHELL and find bash on PATH"
        );
    }

    /// Same as above but with BUZZ_SHELL=cmd.exe.
    #[test]
    fn test_install_path_skips_buzz_shell_cmd() {
        let temp = tempdir().expect("tempdir");
        let cmd = temp.path().join("cmd.exe");
        let bash = temp.path().join("bash.exe");
        std::fs::write(&cmd, []).expect("cmd");
        std::fs::write(&bash, []).expect("bash");

        let path = std::env::join_paths([temp.path()]).expect("PATH");
        let path_str = path.to_str().expect("utf8");

        // Readiness gate: BUZZ_SHELL=cmd accepted.
        assert_eq!(
            resolve_git_bash(path_str, Some(cmd.clone()), None, None, None, None, None),
            Some(cmd),
            "readiness gate must accept BUZZ_SHELL=cmd"
        );

        // Install path: shell_override=None skips cmd, finds bash on PATH.
        assert_eq!(
            resolve_git_bash(path_str, None, None, None, None, None, None),
            Some(bash),
            "install path must skip BUZZ_SHELL and find bash on PATH"
        );
    }
}
