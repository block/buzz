use futures_util::StreamExt;
use serde::Deserialize;
use tauri::{AppHandle, Emitter};

use super::{
    OllamaModel, OllamaModelInfo, OllamaPullProgress, OllamaStatus, OLLAMA_PULL_PROGRESS_EVENT,
};

const MAX_ERROR_BYTES: usize = 16 * 1024;
const MAX_PULL_LINE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Deserialize)]
struct VersionResponse {
    version: String,
}

#[derive(Debug, Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<OllamaModel>,
}

#[derive(Debug, Deserialize)]
struct PullLine {
    #[serde(default)]
    status: String,
    digest: Option<String>,
    completed: Option<u64>,
    total: Option<u64>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ShowResponse {
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    details: serde_json::Value,
    #[serde(default)]
    model_info: serde_json::Value,
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("build Ollama client: {error}"))
}

fn pull_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(6 * 60 * 60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("build Ollama pull client: {error}"))
}

fn api_url(endpoint: &str, path: &str) -> String {
    format!("{}{}", endpoint.trim_end_matches('/'), path)
}

async fn response_error(response: reqwest::Response, action: &str) -> String {
    let status = response.status();
    let bytes = response.bytes().await.unwrap_or_default();
    let bounded = &bytes[..bytes.len().min(MAX_ERROR_BYTES)];
    let detail = String::from_utf8_lossy(bounded).trim().to_string();
    if detail.is_empty() {
        format!("Ollama {action} failed with HTTP {}", status.as_u16())
    } else {
        format!(
            "Ollama {action} failed with HTTP {}: {detail}",
            status.as_u16()
        )
    }
}

pub(crate) async fn probe(config: super::OllamaMachineConfig) -> OllamaStatus {
    let installed = super::managed::runtime_installed();
    let running = super::managed::runtime_running().unwrap_or(false);
    let supported = super::managed::install_supported();
    let request = async {
        let client = client()?;
        let version_response = client
            .get(api_url(&config.endpoint, "/api/version"))
            .send()
            .await
            .map_err(|error| format!("connect to Ollama: {error}"))?;
        if !version_response.status().is_success() {
            return Err(response_error(version_response, "version probe").await);
        }
        let version = version_response
            .json::<VersionResponse>()
            .await
            .map_err(|error| format!("parse Ollama version: {error}"))?
            .version;
        let tags_response = client
            .get(api_url(&config.endpoint, "/api/tags"))
            .send()
            .await
            .map_err(|error| format!("list Ollama models: {error}"))?;
        if !tags_response.status().is_success() {
            return Err(response_error(tags_response, "model listing").await);
        }
        let models = tags_response
            .json::<TagsResponse>()
            .await
            .map_err(|error| format!("parse Ollama model list: {error}"))?
            .models;
        Ok::<_, String>((version, models))
    }
    .await;

    match request {
        Ok((version, models)) => OllamaStatus {
            config,
            reachable: true,
            version: Some(version),
            models,
            error: None,
            managed_runtime_installed: installed,
            managed_runtime_running: running,
            managed_install_supported: supported,
        },
        Err(error) => OllamaStatus {
            config,
            reachable: false,
            version: None,
            models: Vec::new(),
            error: Some(error),
            managed_runtime_installed: installed,
            managed_runtime_running: running,
            managed_install_supported: supported,
        },
    }
}

pub(crate) async fn show(endpoint: &str, model: &str) -> Result<OllamaModelInfo, String> {
    let model = validate_model(model)?;
    let response = client()?
        .post(api_url(endpoint, "/api/show"))
        .json(&serde_json::json!({ "model": model }))
        .send()
        .await
        .map_err(|error| format!("inspect Ollama model: {error}"))?;
    if !response.status().is_success() {
        return Err(response_error(response, "model inspection").await);
    }
    let body = response
        .json::<ShowResponse>()
        .await
        .map_err(|error| format!("parse Ollama model metadata: {error}"))?;
    let supports_tools = body.capabilities.iter().any(|value| value == "tools");
    Ok(OllamaModelInfo {
        model: model.to_string(),
        capabilities: body.capabilities,
        supports_tools,
        details: body.details,
        model_info: body.model_info,
    })
}

pub(crate) async fn pull(app: &AppHandle, endpoint: &str, model: &str) -> Result<(), String> {
    let model = validate_model(model)?.to_string();
    let response = pull_client()?
        .post(api_url(endpoint, "/api/pull"))
        .json(&serde_json::json!({ "model": model, "stream": true }))
        .send()
        .await
        .map_err(|error| format!("pull Ollama model: {error}"))?;
    if !response.status().is_success() {
        return Err(response_error(response, "model pull").await);
    }

    let mut stream = response.bytes_stream();
    let mut pending = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("read Ollama pull stream: {error}"))?;
        pending.extend_from_slice(&chunk);
        if pending.len() > MAX_PULL_LINE_BYTES && !pending.contains(&b'\n') {
            return Err("Ollama pull response line exceeded the safety limit".to_string());
        }
        while let Some(index) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=index).collect();
            emit_pull_line(app, &model, &line)?;
        }
    }
    if !pending.iter().all(u8::is_ascii_whitespace) {
        emit_pull_line(app, &model, &pending)?;
    }
    Ok(())
}

fn emit_pull_line(app: &AppHandle, model: &str, line: &[u8]) -> Result<(), String> {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    if line.iter().all(u8::is_ascii_whitespace) {
        return Ok(());
    }
    if line.len() > MAX_PULL_LINE_BYTES {
        return Err("Ollama pull response line exceeded the safety limit".to_string());
    }
    let line: PullLine = serde_json::from_slice(line)
        .map_err(|error| format!("parse Ollama pull progress: {error}"))?;
    if let Some(error) = line.error {
        return Err(format!("Ollama model pull failed: {error}"));
    }
    let done = matches!(line.status.as_str(), "success" | "done");
    let payload = OllamaPullProgress {
        model: model.to_string(),
        status: line.status,
        digest: line.digest,
        completed: line.completed,
        total: line.total,
        done,
    };
    app.emit(OLLAMA_PULL_PROGRESS_EVENT, payload)
        .map_err(|error| format!("emit Ollama pull progress: {error}"))
}

pub(crate) async fn delete(endpoint: &str, model: &str) -> Result<(), String> {
    let model = validate_model(model)?;
    let response = client()?
        .delete(api_url(endpoint, "/api/delete"))
        .json(&serde_json::json!({ "model": model }))
        .send()
        .await
        .map_err(|error| format!("delete Ollama model: {error}"))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(response_error(response, "model deletion").await)
    }
}

fn validate_model(model: &str) -> Result<&str, String> {
    let model = model.trim();
    if model.is_empty() {
        return Err("Ollama model is required".to_string());
    }
    if model.len() > 512 || model.chars().any(char::is_control) {
        return Err("Ollama model is invalid".to_string());
    }
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_model_names_without_overconstraining_registry_syntax() {
        assert_eq!(validate_model("qwen3:8b").unwrap(), "qwen3:8b");
        assert!(validate_model("  ").is_err());
        assert!(validate_model("bad\nmodel").is_err());
    }

    #[test]
    fn native_api_urls_never_inherit_openai_v1_paths() {
        assert_eq!(
            api_url("http://127.0.0.1:11434", "/api/tags"),
            "http://127.0.0.1:11434/api/tags"
        );
    }
}
