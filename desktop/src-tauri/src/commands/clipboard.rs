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
