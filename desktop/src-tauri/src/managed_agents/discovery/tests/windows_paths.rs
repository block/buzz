//! Windows well-known binary dir coverage (goose default install + Node layouts).

use super::super::{windows_well_known_binary_dirs, WindowsWellKnownEnv};

/// Goose's official Windows CLI installer writes to %USERPROFILE%\goose.
/// Discovery must probe that directory or onboarding shows Installed then
/// blocks Next (#2239).
#[test]
fn test_windows_well_known_dirs_include_userprofile_goose() {
    let home = std::path::PathBuf::from(r"C:\Users\tester");
    let dirs = windows_well_known_binary_dirs(WindowsWellKnownEnv {
        home: Some(home.clone()),
        appdata: Some(std::path::PathBuf::from(r"C:\Users\tester\AppData\Roaming")),
        local_app_data: Some(std::path::PathBuf::from(r"C:\Users\tester\AppData\Local")),
        userprofile: Some(std::path::PathBuf::from(r"C:\Users\tester")),
        ..Default::default()
    });
    assert!(
        dirs.iter().any(|p| p == &home.join("goose")),
        "expected %USERPROFILE%\\goose in well-known dirs, got {dirs:?}"
    );
    assert!(
        dirs.iter()
            .any(|p| p.ends_with(std::path::Path::new(r"AppData\Roaming\npm"))),
        "expected APPDATA\\npm in well-known dirs, got {dirs:?}"
    );
}

/// Official Node MSI + nvm-windows env roots must be on the probe list so GUI
/// PATH gaps do not hide system npm for Codex/Claude adapter installs (#2238).
#[test]
fn test_windows_well_known_dirs_include_official_node_and_nvm_env() {
    let dirs = windows_well_known_binary_dirs(WindowsWellKnownEnv {
        program_files: Some(std::path::PathBuf::from(r"C:\Program Files")),
        program_files_x86: Some(std::path::PathBuf::from(r"C:\Program Files (x86)")),
        local_app_data: Some(std::path::PathBuf::from(r"C:\Users\tester\AppData\Local")),
        nvm_symlink: Some(std::path::PathBuf::from(r"C:\Program Files\nodejs")),
        nvm_home: Some(std::path::PathBuf::from(
            r"C:\Users\tester\AppData\Roaming\nvm",
        )),
        ..Default::default()
    });
    assert!(
        dirs.iter()
            .any(|p| p == std::path::Path::new(r"C:\Program Files\nodejs")),
        "expected official Program Files\\nodejs, got {dirs:?}"
    );
    assert!(
        dirs.iter()
            .any(|p| p == std::path::Path::new(r"C:\Program Files (x86)\nodejs")),
        "expected Program Files (x86)\\nodejs, got {dirs:?}"
    );
    assert!(
        dirs.iter().any(|p| {
            p == std::path::Path::new(r"C:\Users\tester\AppData\Local\Programs\nodejs")
        }),
        "expected LocalAppData\\Programs\\nodejs, got {dirs:?}"
    );
    assert!(
        dirs.iter()
            .any(|p| { p == std::path::Path::new(r"C:\Users\tester\AppData\Roaming\nvm\nodejs") }),
        "expected NVM_HOME\\nodejs, got {dirs:?}"
    );
}

/// When home_dir is unavailable, fall back to USERPROFILE for the goose dir.
#[test]
fn test_windows_well_known_dirs_goose_falls_back_to_userprofile() {
    let profile = std::path::PathBuf::from(r"C:\Users\fallback");
    let dirs = windows_well_known_binary_dirs(WindowsWellKnownEnv {
        userprofile: Some(profile.clone()),
        ..Default::default()
    });
    assert_eq!(
        dirs,
        vec![profile.join("goose")],
        "USERPROFILE fallback must still find goose's default install dir"
    );
}
