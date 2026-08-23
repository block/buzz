use tauri::AppHandle;

use crate::ollama::{
    self, DeleteOllamaModelInput, OllamaMachineConfig, OllamaModelInfo, OllamaStatus,
};

/// Load the machine-wide Ollama configuration.
#[tauri::command]
pub fn get_ollama_config() -> Result<OllamaMachineConfig, String> {
    ollama::load_config()
}

/// Validate and persist machine-wide Ollama configuration.
#[tauri::command]
pub fn set_ollama_config(config: OllamaMachineConfig) -> Result<OllamaMachineConfig, String> {
    ollama::save_config(config)
}

/// Probe the persisted Ollama endpoint and return its native model inventory.
#[tauri::command]
pub async fn get_ollama_status() -> Result<OllamaStatus, String> {
    Ok(ollama::probe(ollama::load_config()?).await)
}

/// Probe an endpoint without persisting it, for config-only import flows.
#[tauri::command]
pub async fn detect_ollama(endpoint: Option<String>) -> Result<OllamaStatus, String> {
    let mut config = ollama::load_config()?;
    if let Some(endpoint) = endpoint {
        config.endpoint = ollama::validate_endpoint(&endpoint)?;
    }
    Ok(ollama::probe(config).await)
}

/// Read a model's capabilities through Ollama's native API.
#[tauri::command]
pub async fn show_ollama_model(model: String) -> Result<OllamaModelInfo, String> {
    let config = ollama::load_config()?;
    ollama::show(&config.endpoint, &model).await
}

/// Pull a model when the configured ownership mode permits model management.
#[tauri::command]
pub async fn pull_ollama_model(model: String, app: AppHandle) -> Result<(), String> {
    let config = ollama::load_config()?;
    if !ollama::model_management_allowed(config.mode) {
        return Err(
            "Ollama is in connect-only mode; enable model management before pulling models"
                .to_string(),
        );
    }
    ollama::pull(&app, &config.endpoint, &model).await
}

/// Delete a model after explicit frontend confirmation.
#[tauri::command]
pub async fn delete_ollama_model(input: DeleteOllamaModelInput) -> Result<(), String> {
    if !input.confirmed {
        return Err("Ollama model deletion requires explicit confirmation".to_string());
    }
    let config = ollama::load_config()?;
    if !ollama::model_management_allowed(config.mode) {
        return Err(
            "Ollama is in connect-only mode; enable model management before deleting models"
                .to_string(),
        );
    }
    ollama::delete(&config.endpoint, &input.model).await
}

/// Install the pinned, checksum-verified runtime declared by this Buzz build.
#[tauri::command]
pub async fn install_managed_ollama() -> Result<(), String> {
    ollama::install().await
}

/// Start only Buzz's private Ollama runtime; external daemons are never touched.
#[tauri::command]
pub async fn start_managed_ollama() -> Result<OllamaStatus, String> {
    let config = ollama::load_config()?;
    if config.mode != ollama::OllamaOwnershipMode::Managed {
        return Err("select fully managed Ollama before starting the private runtime".to_string());
    }
    tokio::task::spawn_blocking(ollama::start)
        .await
        .map_err(|error| format!("managed Ollama start task failed: {error}"))??;
    Ok(ollama::probe(config).await)
}

/// Stop only the process started and tracked by Buzz.
#[tauri::command]
pub async fn stop_managed_ollama() -> Result<(), String> {
    tokio::task::spawn_blocking(ollama::stop)
        .await
        .map_err(|error| format!("managed Ollama stop task failed: {error}"))?
}
