use std::future::Future;

use tauri::{AppHandle, Manager};

use super::readiness_transition_token;
use crate::commands::{LmStudioReadiness, LmStudioReadinessState};

fn lmstudio_readiness_basis(readiness: &Result<LmStudioReadiness, String>) -> Vec<u8> {
    readiness
        .as_ref()
        .ok()
        .and_then(|value| {
            serde_json::to_vec(&(
                value.status,
                &value.configured_model,
                &value.loaded_models,
                &value.security_warnings,
                value.bind_exposure,
            ))
            .ok()
        })
        .unwrap_or_else(|| b"model:probe-unavailable".to_vec())
}

fn lmstudio_readiness_transition_token(readiness: &Result<LmStudioReadiness, String>) -> String {
    readiness_transition_token(&lmstudio_readiness_basis(readiness))
}

#[derive(Clone, Debug)]
pub(crate) struct TrustedModelReadinessObservation {
    readiness: Option<LmStudioReadiness>,
    transition_token: String,
}

impl TrustedModelReadinessObservation {
    pub(crate) fn readiness(&self) -> Option<&LmStudioReadiness> {
        self.readiness.as_ref()
    }

    pub(crate) fn transition_token(&self) -> &str {
        &self.transition_token
    }
}

pub(crate) fn trusted_model_readiness_observation(
    readiness: Result<LmStudioReadiness, String>,
) -> TrustedModelReadinessObservation {
    let transition_token = lmstudio_readiness_transition_token(&readiness);
    TrustedModelReadinessObservation {
        readiness: readiness.ok(),
        transition_token,
    }
}

pub(crate) async fn model_readiness_for_schedule<Probe, ProbeFuture>(
    supplied: Option<TrustedModelReadinessObservation>,
    probe: Probe,
) -> TrustedModelReadinessObservation
where
    Probe: FnOnce() -> ProbeFuture,
    ProbeFuture: Future<Output = Result<LmStudioReadiness, String>>,
{
    match supplied {
        Some(observation) => observation,
        None => trusted_model_readiness_observation(probe().await),
    }
}

pub(crate) fn admitted_model(
    readiness: &LmStudioReadiness,
    trusted_lan_mode: bool,
) -> Option<String> {
    if readiness.status != LmStudioReadinessState::Ready && !trusted_lan_mode {
        return None;
    }
    readiness
        .configured_model
        .clone()
        .or_else(|| readiness.loaded_models.first().cloned())
}

pub(crate) async fn trusted_lan_mode_enabled(app: &AppHandle) -> Result<bool, String> {
    let path = app
        .path()
        .app_config_dir()
        .map_err(|_| "trusted LAN configuration unavailable".to_string())?
        .join("trusted-lan-sources.json");
    tokio::task::spawn_blocking(move || {
        crate::command_services::trusted_lan::load_optional(&path)
            .map(|config| config.is_some())
            .map_err(|_| "trusted LAN configuration unavailable".to_string())
    })
    .await
    .map_err(|_| "trusted LAN configuration unavailable".to_string())?
}
