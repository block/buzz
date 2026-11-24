/// Returns `true` when the running install supports Tauri's auto-updater.
///
/// On Linux, Tauri's updater only works for AppImage bundles.  The AppImage
/// runtime sets the `APPIMAGE` environment variable when the binary is
/// executed from an AppImage.  When that variable is absent (e.g. a `.deb`
/// install), the updater plugin will find an update but cannot swap the
/// binary, producing an "invalid binary format" error at install time.
///
/// On macOS and Windows every supported install format is auto-updatable,
/// so this always returns `true` on those platforms.
#[tauri::command]
pub fn is_auto_update_supported() -> bool {
    #[cfg(target_os = "linux")]
    {
        // The AppImage runtime always sets APPIMAGE to the path of the mounted
        // image file.  Its absence means we are running from a .deb, .rpm, or
        // other non-AppImage package that lacks an AppImage update target.
        std::env::var("APPIMAGE").is_ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

/// Maps a platform to its key in the new Buzz app's installer manifest, or
/// `None` when no installer is published for it.
///
/// On Linux a missing `APPIMAGE` variable only means "not an AppImage", so
/// the deb key also requires dpkg; a source build on another distro gets no
/// key instead of a package it cannot install.
fn installer_target_key(
    os: &str,
    arch: &str,
    is_appimage: bool,
    has_dpkg: bool,
) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some("macos-arm64"),
        ("macos", "x86_64") => Some("macos-x64"),
        ("windows", "x86_64") => Some("windows-x64"),
        ("linux", "x86_64") if is_appimage => Some("linux-appimage"),
        ("linux", "x86_64") if has_dpkg => Some("linux-deb"),
        _ => None,
    }
}

/// Returns the running install's key in the new Buzz app's installer
/// manifest, or `None` when the frontend should link to the website instead.
#[tauri::command]
pub fn new_app_installer_target() -> Option<&'static str> {
    installer_target_key(
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::env::var_os("APPIMAGE").is_some(),
        std::path::Path::new("/usr/bin/dpkg").exists(),
    )
}

#[cfg(test)]
mod tests {
    use super::installer_target_key;

    #[test]
    fn installer_target_key_covers_each_shipped_target_and_nothing_else() {
        // (os, arch, is_appimage, has_dpkg, expected)
        let cases = [
            ("macos", "aarch64", false, false, Some("macos-arm64")),
            ("macos", "x86_64", false, false, Some("macos-x64")),
            ("windows", "x86_64", false, false, Some("windows-x64")),
            ("linux", "x86_64", true, false, Some("linux-appimage")),
            // An AppImage on a Debian host is still an AppImage.
            ("linux", "x86_64", true, true, Some("linux-appimage")),
            ("linux", "x86_64", false, true, Some("linux-deb")),
            // Source build on a distro without dpkg.
            ("linux", "x86_64", false, false, None),
            // No arm64 Linux or Windows installer is published.
            ("linux", "aarch64", true, true, None),
            ("windows", "aarch64", false, false, None),
            ("freebsd", "x86_64", false, true, None),
        ];
        for (os, arch, is_appimage, has_dpkg, expected) in cases {
            assert_eq!(
                installer_target_key(os, arch, is_appimage, has_dpkg),
                expected,
                "{os}/{arch} appimage={is_appimage} dpkg={has_dpkg}"
            );
        }
    }
}
