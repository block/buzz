//! Well-known Windows binary directories for agent CLIs and Node/npm.
//!
//! GUI-launched apps often miss shell-profile PATH entries, so discovery and
//! install shells probe standard install locations plus version-manager env
//! vars (nvm-windows / nvm4w).

use std::path::PathBuf;

/// Collect Windows well-known binary dirs from the live process environment.
pub(crate) fn windows_well_known_binary_dirs_from_env() -> Vec<PathBuf> {
    windows_well_known_binary_dirs(WindowsWellKnownEnv {
        home: dirs::home_dir(),
        appdata: std::env::var_os("APPDATA").map(PathBuf::from),
        local_app_data: std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        userprofile: std::env::var_os("USERPROFILE").map(PathBuf::from),
        program_files: std::env::var_os("ProgramFiles").map(PathBuf::from),
        program_files_x86: std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
        // nvm-windows / nvm4w active symlink or install root (when set).
        nvm_symlink: std::env::var_os("NVM_SYMLINK").map(PathBuf::from),
        nvm_home: std::env::var_os("NVM_HOME").map(PathBuf::from),
    })
}

/// Inputs for [`windows_well_known_binary_dirs`] — pure so unit tests inject
/// paths without mutating process-global `OnceLock` / env state.
#[derive(Debug, Default, Clone)]
pub(crate) struct WindowsWellKnownEnv {
    pub home: Option<PathBuf>,
    pub appdata: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    pub userprofile: Option<PathBuf>,
    pub program_files: Option<PathBuf>,
    pub program_files_x86: Option<PathBuf>,
    pub nvm_symlink: Option<PathBuf>,
    pub nvm_home: Option<PathBuf>,
}

/// Windows-only install locations Buzz must probe for agent CLIs and Node/npm.
///
/// Covers official Node MSI layout, per-user npm globals, nvm-windows / nvm4w
/// env roots, Codex installer, and goose's default `%USERPROFILE%\goose` dir
/// (github.com/block/buzz/issues/2239).
pub(crate) fn windows_well_known_binary_dirs(env: WindowsWellKnownEnv) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // Per-user npm global shims (`npm install -g` without custom prefix).
    if let Some(appdata) = env.appdata {
        paths.push(appdata.join("npm"));
    }

    // Official Node.js MSI (x64) and occasional 32-bit install.
    // https://nodejs.org — default install dir is %ProgramFiles%\nodejs.
    if let Some(program_files) = env.program_files {
        paths.push(program_files.join("nodejs"));
    }
    if let Some(program_files_x86) = env.program_files_x86 {
        paths.push(program_files_x86.join("nodejs"));
    }

    if let Some(local) = env.local_app_data {
        // Some per-user Node installers and winget layouts.
        paths.push(local.join("Programs").join("nodejs"));
        paths.push(
            local
                .join("Programs")
                .join("OpenAI")
                .join("Codex")
                .join("bin"),
        );
    }

    // nvm-windows sets NVM_SYMLINK to the active node dir (often Program Files\nodejs).
    // nvm4w / similar may set NVM_HOME to the install root with a `nodejs` child.
    if let Some(symlink) = env.nvm_symlink {
        paths.push(symlink);
    }
    if let Some(nvm_home) = env.nvm_home {
        paths.push(nvm_home.join("nodejs"));
        paths.push(nvm_home);
    }

    // Back-compat: official goose Windows installer defaults to
    // %USERPROFILE%\goose when GOOSE_BIN_DIR is unset. New Buzz installs pin
    // GOOSE_BIN_DIR=$HOME/.local/bin (already probed via common_binary_paths).
    // Keep this probe so pre-existing installs still resolve (#2239).
    if let Some(home) = env.home {
        paths.push(home.join("goose"));
    } else if let Some(profile) = env.userprofile {
        paths.push(profile.join("goose"));
    }

    paths
}

/// Existing well-known Windows dirs that should be on the install-shell PATH so
/// `npm`/`node` resolve even when the GUI process PATH is thin.
pub(crate) fn windows_existing_well_known_path_dirs() -> Vec<PathBuf> {
    windows_well_known_binary_dirs_from_env()
        .into_iter()
        .filter(|dir| dir.is_dir())
        .collect()
}
