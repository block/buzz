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
        self.0.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
}

pub fn with_clipboard<T>(
    app: &tauri::AppHandle,
    operation: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
) -> Result<T, String> {
    let state = app.state::<ClipboardState>();
    let mut stored = state.0.lock().unwrap_or_else(|e| e.into_inner());
    if stored.is_none() {
        *stored = Some(arboard::Clipboard::new().map_err(|e| format!("clipboard error: {e}"))?);
    }
    operation(stored.as_mut().expect("clipboard initialized"))
        .map_err(|e| format!("clipboard error: {e}"))
}

/// Read plain text from the system clipboard through the native shell.
///
/// Browser clipboard reads are permission-gated or unavailable in embedded
/// webviews. Arboard provides one consistent path across WKWebView, WebView2,
/// and WebKitGTK. The operation runs on the main thread for macOS/AppKit safety.
#[tauri::command]
pub async fn read_clipboard_text(app: tauri::AppHandle) -> Result<String, String> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Result<String, String>>(1);
    let clipboard_app = app.clone();
    app.run_on_main_thread(move || {
        let result = with_clipboard(&clipboard_app, arboard::Clipboard::get_text);
        let _ = tx.send(result);
    })
    .map_err(|e| format!("main thread dispatch failed: {e}"))?;

    rx.recv()
        .map_err(|_| "clipboard result channel closed unexpectedly".to_string())?
}

/// Upper bound on the PNG handed to the renderer, matching the media cap.
const MAX_CLIPBOARD_PNG_BYTES: usize = 50 * 1024 * 1024;

/// Sanity bound on the decoded buffer, matching the `image` crate's own
/// default allocation limit. This is not the transfer budget: a 5K screenshot
/// is ~59 MB of RGBA but only a few MB once encoded.
const MAX_CLIPBOARD_RGBA_BYTES: u64 = 512 * 1024 * 1024;

fn encode_rgba_as_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(std::io::Cursor::new(&mut png), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("failed to encode clipboard image: {e}"))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| format!("failed to encode clipboard image: {e}"))?;
    }
    Ok(png)
}

/// Read an image from the system clipboard as PNG bytes.
///
/// WebKitGTK never exposes a clipboard image as a `DataTransferItem`, so the
/// browser paste path sees an empty `DataTransfer` and attaches nothing.
/// Arboard reads the same X11 or Wayland selection the rest of the desktop
/// sees. An empty response means the clipboard holds no image, which is the
/// ordinary outcome for a text paste rather than a failure.
///
/// The read is dispatched to the main thread, which arboard requires on macOS.
/// It decodes the clipboard's compressed image, so it is the costly half; PNG
/// re-encoding stays off that thread.
#[tauri::command]
pub async fn read_clipboard_image(app: tauri::AppHandle) -> Result<tauri::ipc::Response, String> {
    type ImageRead = Result<Option<(u32, u32, Vec<u8>)>, String>;

    let (tx, rx) = std::sync::mpsc::sync_channel::<ImageRead>(1);
    let clipboard_app = app.clone();
    app.run_on_main_thread(move || {
        let result = with_clipboard(&clipboard_app, |clipboard| match clipboard.get_image() {
            Ok(image) => Ok(Some(image)),
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(e) => Err(e),
        })
        .and_then(|image| {
            let Some(image) = image else { return Ok(None) };
            let (width, height) = (image.width as u64, image.height as u64);
            if width == 0 || height == 0 {
                return Ok(None);
            }
            if width * height * 4 > MAX_CLIPBOARD_RGBA_BYTES {
                return Err("clipboard image too large to paste".to_string());
            }
            Ok(Some((
                image.width as u32,
                image.height as u32,
                image.bytes.into_owned(),
            )))
        });
        let _ = tx.send(result);
    })
    .map_err(|e| format!("main thread dispatch failed: {e}"))?;

    let image = rx
        .recv()
        .map_err(|_| "clipboard result channel closed unexpectedly".to_string())??;

    let Some((width, height, rgba)) = image else {
        return Ok(tauri::ipc::Response::new(Vec::new()));
    };
    let png = encode_rgba_as_png(width, height, &rgba)?;
    if png.len() > MAX_CLIPBOARD_PNG_BYTES {
        return Err("clipboard image too large to paste".to_string());
    }
    Ok(tauri::ipc::Response::new(png))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_rgba_is_encoded_as_a_valid_png() {
        let rgba = vec![0xffu8; 2 * 2 * 4];
        let png = encode_rgba_as_png(2, 2, &rgba).expect("encode");

        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");

        let decoder = png::Decoder::new(std::io::Cursor::new(&png));
        let mut reader = decoder.read_info().expect("read info");
        let mut out = vec![0u8; reader.output_buffer_size().expect("buffer size")];
        let info = reader.next_frame(&mut out).expect("decode frame");
        assert_eq!((info.width, info.height), (2, 2));
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(&out[..info.buffer_size()], &rgba[..]);
    }

    #[test]
    fn mismatched_buffer_length_is_an_error_not_a_panic() {
        let err = encode_rgba_as_png(2, 2, &[0xffu8; 4]).expect_err("short buffer must fail");
        assert!(err.contains("failed to encode clipboard image"), "{err}");
    }
}
