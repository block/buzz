//! Native labels follow the frontend preference; activity names stay verbatim.
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Runtime};

static KOREAN: AtomicBool = AtomicBool::new(false);

pub fn text<'a>(english: &'a str, korean: &'a str) -> &'a str {
    if KOREAN.load(Ordering::Relaxed) {
        korean
    } else {
        english
    }
}

pub fn predefined(korean: &str) -> Option<&str> {
    KOREAN.load(Ordering::Relaxed).then_some(korean)
}

#[tauri::command]
pub fn set_desktop_language<R: Runtime>(app: AppHandle<R>, language: String) -> Result<(), String> {
    if !matches!(language.as_str(), "en" | "ko" | "zh-Hans" | "zh-Hant") {
        return Err("Unsupported desktop language".into());
    }
    KOREAN.store(language == "ko", Ordering::Relaxed);
    #[cfg(target_os = "macos")]
    app.set_menu(crate::app_menu::build(&app).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    #[cfg(target_os = "macos")]
    crate::tray_menu::refresh_language(&app)?;
    #[cfg(not(target_os = "macos"))]
    let _ = app;
    Ok(())
}
