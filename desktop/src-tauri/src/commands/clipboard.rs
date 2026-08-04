use std::sync::Mutex;

use tauri::Manager;

/// App-lifetime clipboard ownership keeps copied data available on Linux and
/// serializes access on Windows. All operations still run on Tauri's main
/// thread for macOS/AppKit safety.
pub struct ClipboardState(Mutex<Option<arboard::Clipboard>>);

impl ClipboardState {
    pub fn new() -> Self {
        Self(Mutex::new(None))
    }

    pub fn release(&self) {
        if let Ok(mut clipboard) = self.0.lock() {
            clipboard.take();
        }
    }
}

pub fn with_clipboard<T>(
    app: &tauri::AppHandle,
    operation: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
) -> Result<T, String> {
    let state = app.state::<ClipboardState>();
    let mut stored = state
        .0
        .lock()
        .map_err(|_| "clipboard state lock poisoned".to_string())?;
    if stored.is_none() {
        *stored = Some(arboard::Clipboard::new().map_err(|e| format!("clipboard error: {e}"))?);
    }
    operation(stored.as_mut().expect("clipboard initialized"))
        .map_err(|e| format!("clipboard error: {e}"))
}

/// Read the system clipboard image, treating a non-image clipboard as an
/// expected empty result rather than an error. WebKitGTK does not always
/// surface Wayland image clipboard offers as `ClipboardEvent` file items.
pub fn read_system_clipboard_image(
    app: &tauri::AppHandle,
) -> Result<Option<arboard::ImageData<'static>>, String> {
    let state = app.state::<ClipboardState>();
    let mut stored = state
        .0
        .lock()
        .map_err(|_| "clipboard state lock poisoned".to_string())?;
    if stored.is_none() {
        *stored = Some(arboard::Clipboard::new().map_err(|e| format!("clipboard error: {e}"))?);
    }

    match stored.as_mut().expect("clipboard initialized").get_image() {
        Ok(image) => Ok(Some(image)),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(error) => Err(format!("clipboard error: {error}")),
    }
}
