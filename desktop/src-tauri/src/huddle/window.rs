//! Native companion-window lifecycle for an active Huddle.

use std::sync::atomic::{AtomicU32, Ordering};

use tauri::{Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

use crate::app_state::AppState;

/// Count of companion destroys that must not restore the drawer.
///
/// Dock (`close_huddle_companion`) and zombie-recreate each note one expected
/// `Destroyed`. A boolean suppress flag was wrong: `open_huddle_window` cleared
/// it before a late `Destroyed` from the prior dock/recreate, so the main app
/// flipped back to the drawer while the new companion window was still up
/// (drawer ⊕ window both visible).
static PENDING_SUPPRESSED_DESTROYS: AtomicU32 = AtomicU32::new(0);

/// Note that the next matching `Destroyed` must not restore the drawer.
fn note_suppress_companion_return() {
    PENDING_SUPPRESSED_DESTROYS.fetch_add(1, Ordering::SeqCst);
}

/// Returns true once when a destroy intentionally should not dock the huddle.
pub(crate) fn take_suppress_companion_return() -> bool {
    let mut current = PENDING_SUPPRESSED_DESTROYS.load(Ordering::SeqCst);
    loop {
        if current == 0 {
            return false;
        }
        match PENDING_SUPPRESSED_DESTROYS.compare_exchange(
            current,
            current - 1,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => return true,
            Err(observed) => current = observed,
        }
    }
}

/// Close the companion belonging to an ended huddle. The native lifecycle is
/// authoritative here because a webview can be suspended while it is closing.
pub(super) fn close_huddle_window(app: &tauri::AppHandle, ephemeral_channel_id: &str) {
    if ephemeral_channel_id.is_empty() {
        return;
    }
    let label = format!("huddle-{ephemeral_channel_id}");
    destroy_huddle_window(app, &label, false);
}

/// Force-destroy the companion. `close()` alone can leave a visible macOS
/// window when CloseRequested races the webview teardown; `destroy()` removes
/// it from the window map so docking to the drawer actually clears the stage.
fn destroy_huddle_window(app: &tauri::AppHandle, label: &str, suppress_return: bool) {
    let Some(window) = app.get_webview_window(label) else {
        return;
    };
    if suppress_return {
        note_suppress_companion_return();
    }
    // Hide first so the user does not see a lingering frame if destroy is slow.
    if let Err(error) = window.hide() {
        eprintln!("buzz-desktop: failed to hide huddle companion before destroy: {error}");
    }
    if let Err(error) = window.destroy() {
        eprintln!(
            "buzz-desktop: failed to destroy huddle companion ({error}); falling back to close"
        );
        if let Err(close_error) = window.close() {
            eprintln!("buzz-desktop: failed to close huddle companion: {close_error}");
            if suppress_return {
                // close also failed — drop the pending suppress so a later real
                // dock still restores the drawer.
                let _ = take_suppress_companion_return();
            }
        }
    }
}

/// Whether the active huddle still has a companion webview in the window map.
#[tauri::command]
pub fn huddle_companion_window_exists(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let ephemeral_channel_id = match state.huddle()?.ephemeral_channel_id.clone() {
        Some(id) if !id.is_empty() => id,
        _ => return Ok(false),
    };
    let label = format!("huddle-{ephemeral_channel_id}");
    Ok(app.get_webview_window(&label).is_some())
}

/// Close the active companion without leaving the huddle. The main window uses
/// this to restore its drawer presentation while retaining the audio session.
///
/// Drawer ⊕ window: destroy must succeed before the drawer is restored so both
/// never remain visible. Destroyed is suppressed for this intentional teardown
/// so we emit `huddle-companion-returned` exactly once after teardown. The
/// pending suppress count is left for Destroyed to consume — open must never
/// clear it, or a late Destroyed can re-open the drawer after expand.
#[tauri::command]
pub fn close_huddle_companion(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let ephemeral_channel_id = state
        .huddle()?
        .ephemeral_channel_id
        .clone()
        .ok_or("no active huddle")?;
    let label = format!("huddle-{ephemeral_channel_id}");
    // Note suppress once for the eventual Destroyed; retries must not stack.
    destroy_huddle_window(&app, &label, true);
    if app.get_webview_window(&label).is_some() {
        // destroy() can leave a zombie on macOS; one more hide+destroy+close pass
        // without another suppress note (Destroyed still consumes the one above).
        destroy_huddle_window(&app, &label, false);
    }
    if app.get_webview_window(&label).is_some() {
        let _ = take_suppress_companion_return();
        return Err("failed to destroy huddle companion window".to_string());
    }
    app.emit("huddle-companion-returned", ())
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Open the active huddle's ephemeral channel in a focused companion window.
/// The main window remains the owner of microphone capture; closing this room
/// must never leave the shared huddle session.
#[tauri::command]
pub async fn open_huddle_window(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let ephemeral_channel_id = state
        .huddle()?
        .ephemeral_channel_id
        .clone()
        .ok_or("no active huddle")?;
    let label = format!("huddle-{ephemeral_channel_id}");

    // Do NOT clear pending suppressed destroys here. A late Destroyed from a
    // prior dock/recreate must still be consumed; clearing early was the XOR
    // race that restored the drawer while this companion stayed open.

    if let Some(window) = app.get_webview_window(&label) {
        match window.show().and_then(|_| window.set_focus()) {
            Ok(()) => return Ok(()),
            Err(error) => {
                eprintln!("buzz-desktop: existing huddle companion unusable ({error}); recreating");
                destroy_huddle_window(&app, &label, true);
            }
        }
    }

    // destroy() removes the OS window asynchronously. Brief poll so Builder
    // does not race the still-registered label ("already exists").
    for _ in 0..20 {
        if app.get_webview_window(&label).is_none() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    // Surviving window after a failed-focus destroy: focus it — never error
    // with "already exists" while the companion is still on stage (that path
    // demoted React to drawer and left drawer ⊕ window both visible).
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    match WebviewWindowBuilder::new(&app, label.clone(), WebviewUrl::App("index.html".into()))
        .title("Huddle")
        .inner_size(960.0, 720.0)
        .min_inner_size(720.0, 520.0)
        .build()
    {
        Ok(_) => Ok(()),
        Err(error) => {
            let message = error.to_string();
            if message.to_ascii_lowercase().contains("already exists") {
                if let Some(window) = app.get_webview_window(&label) {
                    let _ = window.show();
                    let _ = window.set_focus();
                    return Ok(());
                }
                // Label reserved but get_webview_window missed it — still treat
                // as open success so the frontend keeps presentation "window".
                eprintln!(
                    "buzz-desktop: huddle companion label {label} already exists; treating open as success"
                );
                return Ok(());
            }
            Err(message)
        }
    }
}
