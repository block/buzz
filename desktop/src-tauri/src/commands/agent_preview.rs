use base64::Engine;
use std::path::PathBuf;

const MAX_PREVIEW_BYTES: u64 = 20 * 1024 * 1024;
const ALLOWED_IMAGE_MIMES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
];

#[tauri::command]
pub async fn read_agent_preview_image(path: String) -> Result<String, String> {
    let requested = PathBuf::from(path);
    if !requested.is_absolute() {
        return Err("agent preview path must be absolute".into());
    }

    let canonical = tokio::fs::canonicalize(&requested)
        .await
        .map_err(|e| format!("cannot resolve agent preview image: {e}"))?;
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|e| format!("cannot inspect agent preview image: {e}"))?;
    if !metadata.is_file() {
        return Err("agent preview path is not a file".into());
    }
    if metadata.len() > MAX_PREVIEW_BYTES {
        return Err("agent preview image exceeds 20 MiB".into());
    }

    let bytes = tokio::fs::read(&canonical)
        .await
        .map_err(|e| format!("cannot read agent preview image: {e}"))?;
    let mime = infer::get(&bytes)
        .map(|kind| kind.mime_type())
        .filter(|mime| ALLOWED_IMAGE_MIMES.contains(mime))
        .ok_or_else(|| "agent preview file is not a supported image".to_string())?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_types_are_raster_images_only() {
        assert!(ALLOWED_IMAGE_MIMES.contains(&"image/png"));
        assert!(!ALLOWED_IMAGE_MIMES.contains(&"image/svg+xml"));
        assert!(!ALLOWED_IMAGE_MIMES.contains(&"video/mp4"));
    }
}
