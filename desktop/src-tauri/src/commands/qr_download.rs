use base64::{engine::general_purpose::STANDARD, Engine as _};

use crate::commands::export_util::save_bytes_with_dialog;
use crate::commands::media::sanitize_filename;
use crate::commands::personas::PNG_MAGIC;

fn decode_png_data_url(data_url: &str) -> Result<Vec<u8>, String> {
    let encoded = data_url
        .strip_prefix("data:image/png;base64,")
        .ok_or_else(|| "invalid PNG data URL".to_string())?;
    let bytes = STANDARD
        .decode(encoded.trim())
        .map_err(|_| "invalid PNG data URL".to_string())?;
    if !bytes.starts_with(&PNG_MAGIC) {
        return Err("invalid PNG data URL".to_string());
    }
    Ok(bytes)
}

/// Save a generated PNG data URL through the native save-file dialog.
#[tauri::command]
pub async fn save_png_data_url(
    data_url: String,
    filename: String,
    app: tauri::AppHandle,
) -> Result<bool, String> {
    let bytes = decode_png_data_url(&data_url)?;
    let filename = sanitize_filename(&filename);
    save_bytes_with_dialog(&app, &filename, "PNG image", &["png"], &bytes).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_png_data_url_accepts_png_payload() {
        let encoded = STANDARD.encode([PNG_MAGIC.as_slice(), &[0, 1, 2]].concat());
        let bytes = decode_png_data_url(&format!("data:image/png;base64,{encoded}")).unwrap();
        assert!(bytes.starts_with(&PNG_MAGIC));
    }

    #[test]
    fn decode_png_data_url_rejects_wrong_mime_or_bytes() {
        let png = STANDARD.encode(PNG_MAGIC);
        assert!(decode_png_data_url(&format!("data:image/jpeg;base64,{png}")).is_err());

        let not_png = STANDARD.encode(b"not a png");
        assert!(decode_png_data_url(&format!("data:image/png;base64,{not_png}")).is_err());
        assert!(decode_png_data_url("data:image/png;base64,not-base64!").is_err());
    }
}
