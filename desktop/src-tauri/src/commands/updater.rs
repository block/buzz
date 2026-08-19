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
