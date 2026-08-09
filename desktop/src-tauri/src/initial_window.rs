//! First-frame window reveal helpers.

#[cfg(target_os = "macos")]
pub(crate) const INITIAL_RENDER_READY_EVENT: &str = "initial-render-ready";

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn reveal_initial_window<R: tauri::Runtime>(window: &tauri::Window<R>) {
    if let Err(error) = window.show() {
        eprintln!("buzz-desktop: failed to reveal main window: {error}");
        return;
    }
    if let Err(error) = window.set_focus() {
        eprintln!("buzz-desktop: failed to focus main window: {error}");
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn reveal_linux_window(window: &gtk::ApplicationWindow) {
    use gtk::prelude::{GtkWindowExt, WidgetExt};

    // Mutter on GNOME/X11 can accept the Tauri GTK window as a managed
    // client but leave it permanently in WM_STATE=Iconic without ever mapping
    // a frame. Realize the hidden window first, then opt this Linux fallback
    // out of window-manager redirection before its first map request.
    window.realize();
    if let Some(gdk_window) = window.window() {
        gdk_window.set_override_redirect(true);
        window.show_all();
        gdk_window.raise();

        let application_window = window.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
            // Once the surface has completed one successful map, hand it back
            // to Mutter through GTK itself. Remapping only the low-level GDK
            // surface leaves Mutter without a correctly registered
            // _NET_WM_PING client, producing a false "not responding" dialog.
            application_window.hide();
            if let Some(managed_window) = application_window.window() {
                managed_window.set_override_redirect(false);
            }
            application_window.show_all();
            application_window.present();
        });
    } else {
        window.show_all();
    }
}

pub(crate) fn focus_existing_window<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    #[cfg(target_os = "linux")]
    match window.gtk_window() {
        Ok(window) => reveal_linux_window(&window),
        Err(error) => {
            eprintln!("buzz-desktop: failed to access native GTK window: {error}");
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = window.set_focus();
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn install_window_state<R: tauri::Runtime>(
    builder: tauri::Builder<R>,
) -> tauri::Builder<R> {
    // Restoring a maximized window races Mutter's initial hidden map on
    // affected GNOME/X11 systems and leaves WM_STATE=Iconic.
    builder
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn install_window_state<R: tauri::Runtime>(
    builder: tauri::Builder<R>,
) -> tauri::Builder<R> {
    use tauri_plugin_window_state::StateFlags;

    builder.plugin(
        tauri_plugin_window_state::Builder::default()
            // Visibility is excluded: the native reveal plugin below
            // shows the window after saved geometry has been restored.
            .with_state_flags(StateFlags::all() & !StateFlags::VISIBLE)
            .build(),
    )
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
