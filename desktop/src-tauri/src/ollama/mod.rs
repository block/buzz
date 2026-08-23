mod client;
mod config;
mod managed;
mod types;

pub use types::*;

pub const OLLAMA_PULL_PROGRESS_EVENT: &str = "ollama-pull-progress";

pub(crate) use client::{delete, probe, pull, show};
pub(crate) use config::{load_config, save_config, validate_endpoint};
pub(crate) use managed::{install, start, stop};

pub(crate) fn model_management_allowed(mode: OllamaOwnershipMode) -> bool {
    matches!(
        mode,
        OllamaOwnershipMode::ExternalManagedModels | OllamaOwnershipMode::Managed
    )
}

/// Resolve the OpenAI-compatible base URL from the machine-level native API
/// endpoint. Agent-local env remains authoritative at the call site.
pub(crate) fn openai_base_url() -> Result<String, String> {
    Ok(openai_base_url_from_endpoint(&load_config()?.endpoint))
}

fn openai_base_url_from_endpoint(endpoint: &str) -> String {
    format!("{}/v1", endpoint.trim_end_matches('/'))
}

/// Start the private runtime on agent demand when the machine ownership mode
/// says Buzz owns it. External daemons are always left alone.
pub(crate) fn ensure_started_for_agent() -> Result<(), String> {
    if load_config()?.mode == OllamaOwnershipMode::Managed {
        start()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_endpoint_maps_to_openai_compatible_v1() {
        assert_eq!(
            openai_base_url_from_endpoint(DEFAULT_OLLAMA_ENDPOINT),
            "http://127.0.0.1:11434/v1"
        );
    }
}
