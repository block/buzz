//! Audio output device enumeration, selection, and sink creation.

use tauri::State;

use crate::app_state::AppState;

/// List available audio output devices. Returns (name, is_default) pairs.
#[tauri::command]
pub async fn list_audio_output_devices() -> Result<Vec<AudioOutputDevice>, String> {
    tokio::task::spawn_blocking(list_audio_output_devices_blocking)
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

fn list_audio_output_devices_blocking() -> Result<Vec<AudioOutputDevice>, String> {
    use rodio::cpal::traits::HostTrait;
    use rodio::DeviceTrait;

    let host = rodio::cpal::default_host();
    let default_name = host
        .default_output_device()
        .and_then(|d| d.description().ok().map(|desc| desc.name().to_string()));
    let devices = host
        .output_devices()
        .map_err(|e| format!("enumerate output devices: {e}"))?;

    let mut result = Vec::new();
    for device in devices {
        if let Ok(name) = device.description().map(|d| d.name().to_string()) {
            let is_default = default_name.as_deref() == Some(&name);
            result.push(AudioOutputDevice { name, is_default });
        }
    }
    Ok(result)
}

/// Set the preferred audio output device by name. Empty string = system default.
/// Takes effect on the next huddle start/join (does not change a live stream).
#[tauri::command]
pub fn set_audio_output_device(name: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state
        .audio_output_device
        .lock()
        .map_err(|e| e.to_string())?;
    *guard = if name.is_empty() { None } else { Some(name) };
    Ok(())
}

/// Get the currently selected audio output device name (empty = system default).
#[tauri::command]
pub fn get_audio_output_device(state: State<'_, AppState>) -> Result<String, String> {
    let guard = state
        .audio_output_device
        .lock()
        .map_err(|e| e.to_string())?;
    Ok(guard.clone().unwrap_or_default())
}

#[derive(Debug, serde::Serialize)]
pub struct AudioOutputDevice {
    pub name: String,
    pub is_default: bool,
}

/// Open a rodio sink for a named output device, falling back to default.
pub(crate) fn open_output_sink_by_name(
    preferred: Option<&str>,
) -> Result<rodio::MixerDeviceSink, String> {
    use rodio::cpal::traits::HostTrait;
    use rodio::DeviceTrait;

    if let Some(name) = preferred {
        let host = rodio::cpal::default_host();
        if let Ok(devices) = host.output_devices() {
            for device in devices {
                if device
                    .description()
                    .ok()
                    .map(|d| d.name().to_string())
                    .as_deref()
                    == Some(name)
                {
                    if let Ok(sink) = rodio::DeviceSinkBuilder::from_device(device) {
                        return sink
                            .open_stream()
                            .map_err(|e| format!("audio output ({name}): {e}"));
                    }
                }
            }
        }
        eprintln!(
            "buzz-desktop: preferred output device {name:?} not found, falling back to default"
        );
    }

    rodio::DeviceSinkBuilder::open_default_sink().map_err(|e| format!("audio output: {e}"))
}
