use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use tauri::{Emitter, Manager, RunEvent, WindowEvent};

use crate::{
    app_state::AppState,
    commands::ClipboardState,
    huddle::HuddlePhase,
    shutdown::{is_restart_request, shut_down_app},
};

#[cfg(all(feature = "mesh-llm", target_os = "macos"))]
use crate::shutdown::{hard_exit_after_mesh_shutdown, relaunch_after_mesh_shutdown};
#[cfg(target_os = "macos")]
use crate::tray_menu::show_main_window;

pub(crate) fn run(app: tauri::App) {
    let shutdown_done = Arc::new(AtomicBool::new(false));

    #[cfg(unix)]
    crate::shutdown::install_signal_handler(app.handle().clone(), Arc::clone(&shutdown_done));

    let run_shutdown_done = Arc::clone(&shutdown_done);
    let restart_requested = Arc::new(AtomicBool::new(false));
    app.run(move |app_handle, event| match event {
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => show_main_window(app_handle),
        #[cfg(target_os = "macos")]
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "main" => {
            // Keep the webview alive so Buzz can be reopened from its tray menu.
            api.prevent_close();
            if let Some(window) = app_handle.get_webview_window("main") {
                if let Err(error) = window.hide() {
                    eprintln!("buzz-desktop: failed to hide main window: {error}");
                }
            }
        }
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { .. },
            ..
        } if label.starts_with("huddle-") => {
            let is_active_huddle_window =
                app_handle
                    .state::<AppState>()
                    .huddle()
                    .ok()
                    .is_some_and(|huddle| {
                        !matches!(huddle.phase, HuddlePhase::Idle | HuddlePhase::Leaving)
                            && huddle
                                .ephemeral_channel_id
                                .as_deref()
                                .is_some_and(|channel_id| label == format!("huddle-{channel_id}"))
                    });
            if is_active_huddle_window {
                if let Err(error) = app_handle.emit("huddle-companion-returned", ()) {
                    eprintln!("buzz-desktop: failed to restore huddle drawer: {error}");
                }
            }
        }
        RunEvent::ExitRequested { code, .. } => {
            if is_restart_request(code) {
                restart_requested.store(true, Ordering::SeqCst);
            }
            shut_down_app(app_handle, &run_shutdown_done);
        }
        RunEvent::Exit => {
            shut_down_app(app_handle, &run_shutdown_done);
            app_handle.state::<ClipboardState>().release();

            #[cfg(all(feature = "mesh-llm", target_os = "macos"))]
            if restart_requested.load(Ordering::SeqCst) {
                relaunch_after_mesh_shutdown(app_handle);
            }

            // AppKit terminates through libc exit(), which runs C++ static
            // destructors. The embedded ggml/Metal runtime currently aborts in
            // that destructor phase even after its node has stopped cleanly.
            // End the process only after Buzz and Mesh shutdown above, while
            // deliberately skipping those native global destructors.
            #[cfg(all(feature = "mesh-llm", target_os = "macos"))]
            hard_exit_after_mesh_shutdown();
        }
        _ => {}
    });
}
