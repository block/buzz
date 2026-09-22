//! Tauri native-picker adapter for the reusable local Pocket voice library.

use std::path::PathBuf;

use buzz_voice_pkg::imported::{ImportedVoice, PocketVoiceLibrary};
use tauri::{AppHandle, Manager};

pub fn voices_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("tts").join("pocket-voices"))
        .map_err(|error| format!("could not locate local voice storage: {error}"))
}

fn library(app: &AppHandle) -> Result<PocketVoiceLibrary, String> {
    voices_dir(app).map(PocketVoiceLibrary::new)
}

pub fn load_registry(app: &AppHandle) -> Result<Vec<ImportedVoice>, String> {
    library(app)?.load()
}

pub fn resolve_file(app: &AppHandle, voice: &ImportedVoice) -> Result<PathBuf, String> {
    library(app)?.resolve_file(voice)
}

pub async fn pick_and_import(app: &AppHandle) -> Result<Option<ImportedVoice>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter(
            "Audio",
            &["wav", "m4a", "mp3", "flac", "ogg", "oga", "aif", "aiff"],
        )
        .pick_file(move |path| {
            let _ = sender.send(path);
        });
    let Some(file_path) = receiver
        .await
        .map_err(|_| "voice picker closed unexpectedly".to_string())?
    else {
        return Ok(None);
    };
    let path = file_path
        .as_path()
        .ok_or("the selected voice path is invalid")?
        .to_path_buf();
    let voice_library = library(app)?;
    tokio::task::spawn_blocking(move || voice_library.import_path(&path))
        .await
        .map_err(|error| format!("voice import task failed: {error}"))?
        .map(Some)
}

pub fn delete(app: &AppHandle, key: &str) -> Result<(), String> {
    library(app)?.delete(key)
}
