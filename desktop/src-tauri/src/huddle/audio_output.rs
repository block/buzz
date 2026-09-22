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
        .huddle_audio
        .output_device
        .lock()
        .map_err(|e| e.to_string())?;
    *guard = if name.is_empty() { None } else { Some(name) };
    Ok(())
}

/// Get the currently selected audio output device name (empty = system default).
#[tauri::command]
pub fn get_audio_output_device(state: State<'_, AppState>) -> Result<String, String> {
    let guard = state
        .huddle_audio
        .output_device
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

fn device_type_is_isolated(device_type: rodio::cpal::DeviceType) -> bool {
    use rodio::cpal::DeviceType;
    matches!(
        device_type,
        DeviceType::Headphones
            | DeviceType::Headset
            | DeviceType::Earpiece
            | DeviceType::HearingAid
    )
}

/// Conservative route-isolation query using cpal's safe structured device
/// description. This is intentionally re-evaluated at confirmed local onset,
/// so a route change cannot leave a stale isolated capability behind.
pub(crate) fn output_route_is_isolated(preferred: Option<&str>) -> bool {
    use rodio::cpal::traits::HostTrait;
    use rodio::DeviceTrait;

    let host = rodio::cpal::default_host();
    let device = match preferred.filter(|name| !name.is_empty()) {
        Some(name) => {
            let Ok(devices) = host.output_devices() else {
                return false;
            };
            let mut matches = devices.filter(|device| {
                device
                    .description()
                    .ok()
                    .map(|description| description.name().to_owned())
                    == Some(name.to_owned())
            });
            let Some(device) = matches.next() else {
                return false;
            };
            if matches.next().is_some() {
                return false;
            }
            device
        }
        None => match host.default_output_device() {
            Some(device) => device,
            None => return false,
        },
    };

    device
        .description()
        .is_ok_and(|description| device_type_is_isolated(description.device_type()))
}

#[cfg(test)]
mod route_isolation_tests {
    use super::device_type_is_isolated;
    use rodio::cpal::DeviceType;

    #[test]
    fn only_positive_isolated_terminal_types_are_accepted() {
        assert!(device_type_is_isolated(DeviceType::Headphones));
        assert!(device_type_is_isolated(DeviceType::Headset));
        assert!(device_type_is_isolated(DeviceType::Earpiece));
        assert!(device_type_is_isolated(DeviceType::HearingAid));
        assert!(!device_type_is_isolated(DeviceType::Speaker));
        assert!(!device_type_is_isolated(DeviceType::Virtual));
        assert!(!device_type_is_isolated(DeviceType::Unknown));
    }
}
