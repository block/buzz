//! First-frame window reveal helpers.

#[cfg(target_os = "macos")]
pub(crate) const INITIAL_RENDER_READY_EVENT: &str = "initial-render-ready";

pub(crate) fn reveal_initial_window<R: tauri::Runtime>(window: &tauri::Window<R>) {
    if let Err(error) = window.show() {
        eprintln!("buzz-desktop: failed to reveal main window: {error}");
        return;
    }
    if let Err(error) = window.set_focus() {
        eprintln!("buzz-desktop: failed to focus main window: {error}");
    }
}

/// Reveal a DGX Spark AppImage without handing its first WebKit surface to
/// Mutter. GNOME Remote Login's virtual display can crash GNOME Shell when the
/// initial XWayland surface is managed immediately. Realize and map it outside
/// window-manager control, then hand the same window back after one frame.
#[cfg(target_os = "linux")]
pub(crate) fn reveal_initial_linux_window<R: tauri::Runtime>(
    window: &tauri::Window<R>,
    needs_unmanaged_first_map: bool,
) {
    if !needs_unmanaged_first_map {
        reveal_initial_window(window);
        return;
    }

    use gtk::prelude::*;

    let gtk_window = match window.gtk_window() {
        Ok(window) => window,
        Err(error) => {
            eprintln!("buzz-desktop: failed to obtain GTK main window: {error}");
            reveal_initial_window(window);
            return;
        }
    };

    gtk_window.realize();
    let Some(gdk_window) = gtk_window.window() else {
        eprintln!("buzz-desktop: GTK main window has no realized GDK window");
        reveal_initial_window(window);
        return;
    };

    gdk_window.set_override_redirect(true);
    gtk_window.show_all();
    gdk_window.raise();
    eprintln!("buzz-desktop: DGX Spark first map bypassed Mutter");

    let tauri_window = window.clone();
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
        gdk_window.hide();
        gdk_window.set_override_redirect(false);
        gdk_window.show();
        gdk_window.raise();
        if let Err(error) = tauri_window.set_focus() {
            eprintln!("buzz-desktop: failed to focus remapped main window: {error}");
        }
        eprintln!("buzz-desktop: DGX Spark window handed back to Mutter");
    });
}

#[cfg(target_os = "macos")]
pub(crate) fn set_initial_window_backing<R: tauri::Runtime>(window: &tauri::Window<R>) {
    // The window remains transparent at runtime for vibrancy. Use an opaque
    // native backing only across the first visible frames so the previous app
    // cannot show through before WebKit has submitted its first surface.
    if let Err(error) = window.set_background_color(Some(tauri::window::Color(17, 21, 24, 255))) {
        eprintln!("buzz-desktop: failed to set initial window backing: {error}");
    }
}

#[cfg(target_os = "macos")]
pub(crate) async fn clear_initial_window_backing<R: tauri::Runtime>(window: &tauri::Window<R>) {
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    if let Err(error) = window.set_background_color(None) {
        eprintln!("buzz-desktop: failed to clear initial window backing: {error}");
    }
}

#[cfg(target_os = "macos")]
pub(crate) async fn wait_for_stable_initial_window_geometry<R: tauri::Runtime>(
    window: &tauri::Window<R>,
) {
    const MAX_POLLS: usize = 120;
    const REQUIRED_STABLE_POLLS: usize = 4;

    let mut previous_bounds = None;
    let mut stable_polls = 0;

    for _ in 0..MAX_POLLS {
        // Accept whatever geometry the window-state plugin restores — maximized
        // or a normal saved size. macOS applies the restore asynchronously, so
        // consecutive identical outer bounds are enough to know it settled.
        let bounds = match (window.outer_position(), window.outer_size()) {
            (Ok(position), Ok(size)) => Some((position.x, position.y, size.width, size.height)),
            _ => None,
        };

        if bounds.is_some() && bounds == previous_bounds {
            stable_polls += 1;
            if stable_polls >= REQUIRED_STABLE_POLLS {
                return;
            }
        } else {
            stable_polls = 0;
        }
        previous_bounds = bounds;

        tokio::time::sleep(std::time::Duration::from_millis(16)).await;
    }

    eprintln!("buzz-desktop: initial window geometry did not settle before reveal timeout");
}
