use std::io::Write;
use std::path::{Path, PathBuf};

use atomic_write_file::AtomicWriteFile;
use url::Url;

use super::{OllamaMachineConfig, OllamaOwnershipMode, DEFAULT_OLLAMA_ENDPOINT};

pub(crate) fn config_path() -> Result<PathBuf, String> {
    dirs::data_dir()
        .map(|path| path.join("Buzz").join("ollama").join("config.json"))
        .ok_or_else(|| "failed to resolve the app-data directory for Ollama settings".to_string())
}

pub(crate) fn load_config() -> Result<OllamaMachineConfig, String> {
    load_config_from(&config_path()?)
}

pub(crate) fn load_config_from(path: &Path) -> Result<OllamaMachineConfig, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let config = serde_json::from_slice(&bytes)
                .map_err(|error| format!("parse Ollama settings: {error}"))?;
            validate_config(config)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(OllamaMachineConfig::default())
        }
        Err(error) => Err(format!("read Ollama settings: {error}")),
    }
}

pub(crate) fn save_config(config: OllamaMachineConfig) -> Result<OllamaMachineConfig, String> {
    let config = validate_config(config)?;
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create Ollama settings directory: {error}"))?;
    }
    let bytes = serde_json::to_vec_pretty(&config)
        .map_err(|error| format!("serialize Ollama settings: {error}"))?;
    let mut file = AtomicWriteFile::open(&path)
        .map_err(|error| format!("open Ollama settings for atomic write: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("set Ollama settings permissions: {error}"))?;
    }
    file.write_all(&bytes)
        .map_err(|error| format!("write Ollama settings: {error}"))?;
    file.commit()
        .map_err(|error| format!("commit Ollama settings: {error}"))?;
    Ok(config)
}

pub(crate) fn validate_endpoint(endpoint: &str) -> Result<String, String> {
    let endpoint = endpoint.trim().trim_end_matches('/');
    if endpoint.is_empty() {
        return Err("Ollama endpoint is required".to_string());
    }
    let mut url =
        Url::parse(endpoint).map_err(|error| format!("invalid Ollama endpoint: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Ollama endpoint must use http or https".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Ollama endpoint must not contain credentials".to_string());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("Ollama endpoint must not contain a query or fragment".to_string());
    }
    if url.host_str().is_none() {
        return Err("Ollama endpoint must contain a host".to_string());
    }
    if !matches!(url.path(), "" | "/") {
        return Err("Ollama endpoint must not contain a path (omit /v1)".to_string());
    }
    url.set_path("");
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn validate_config(mut config: OllamaMachineConfig) -> Result<OllamaMachineConfig, String> {
    config.endpoint = validate_endpoint(&config.endpoint)?;
    config.selected_model = config
        .selected_model
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty());
    if config.mode == OllamaOwnershipMode::Managed && config.endpoint != DEFAULT_OLLAMA_ENDPOINT {
        return Err(format!(
            "Buzz-managed Ollama must use its loopback endpoint {DEFAULT_OLLAMA_ENDPOINT}"
        ));
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_validation_rejects_credentials_paths_and_fragments() {
        assert!(validate_endpoint("http://user:pass@localhost:11434").is_err());
        assert!(validate_endpoint("http://localhost:11434/v1").is_err());
        assert!(validate_endpoint("http://localhost:11434/#x").is_err());
        assert_eq!(
            validate_endpoint("http://localhost:11434/").unwrap(),
            "http://localhost:11434"
        );
    }

    #[test]
    fn missing_config_uses_connect_only_default() {
        let temp = tempfile::tempdir().unwrap();
        let config = load_config_from(&temp.path().join("missing.json")).unwrap();
        assert_eq!(config.mode, OllamaOwnershipMode::ConnectOnly);
        assert_eq!(config.endpoint, DEFAULT_OLLAMA_ENDPOINT);
    }

    #[test]
    fn managed_mode_is_loopback_only() {
        let config = OllamaMachineConfig {
            endpoint: "https://ollama.example".to_string(),
            mode: OllamaOwnershipMode::Managed,
            selected_model: None,
        };
        assert!(validate_config(config).is_err());
    }
}
