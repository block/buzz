//! Pure admission policy for the disconnected Command Adviser model.

use serde::Serialize;

use crate::managed_agents::runtime::{QUALIFIED_CONTEXT_LENGTH, QUALIFIED_GENERATION_CAPACITY};
use crate::managed_agents::AgentModelInfo;

pub(crate) const QUALIFIED_MODEL_ID: &str = "google/gemma-4-26b-a4b";
pub(crate) const QUALIFIED_INSTANCE_ID: &str = "gemma4-26b-official";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OfflineModelPolicy {
    pub(crate) model_id: &'static str,
    pub(crate) instance_id: &'static str,
    pub(crate) required_context_length: u64,
    pub(crate) require_tools: bool,
    pub(crate) require_vision: bool,
    pub(crate) generation_capacity: u64,
}

impl OfflineModelPolicy {
    pub(crate) fn command_adviser() -> Self {
        Self {
            model_id: QUALIFIED_MODEL_ID,
            instance_id: QUALIFIED_INSTANCE_ID,
            required_context_length: QUALIFIED_CONTEXT_LENGTH,
            require_tools: true,
            require_vision: true,
            generation_capacity: QUALIFIED_GENERATION_CAPACITY,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OfflineModelAdmissionState {
    Ready,
    NotLoaded,
    WrongModel,
    MissingCapability,
    InsufficientContext,
    InvalidRuntime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineRuntimeIdentity {
    pub model_id: String,
    pub instance_id: String,
    pub context_length: u64,
    pub parallel: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineModelAdmission {
    pub state: OfflineModelAdmissionState,
    pub admitted_tier: Option<String>,
    pub runtime: Option<OfflineRuntimeIdentity>,
    pub reason_codes: Vec<String>,
}

fn rejected(state: OfflineModelAdmissionState, reason: &str) -> OfflineModelAdmission {
    OfflineModelAdmission {
        state,
        admitted_tier: None,
        runtime: None,
        reason_codes: vec![reason.to_string()],
    }
}

pub(crate) fn evaluate_offline_admission(
    policy: &OfflineModelPolicy,
    models: &[AgentModelInfo],
) -> OfflineModelAdmission {
    let loaded = models
        .iter()
        .filter(|model| model.is_loaded)
        .collect::<Vec<_>>();
    if loaded.is_empty() {
        return rejected(OfflineModelAdmissionState::NotLoaded, "no_loaded_model");
    }
    let Some(model) = loaded
        .iter()
        .copied()
        .find(|model| model.id == policy.model_id)
    else {
        return rejected(
            OfflineModelAdmissionState::WrongModel,
            "qualified_model_not_loaded",
        );
    };
    if model.loaded_instance_ids.len() != 1
        || model.loaded_instance_ids.first().map(String::as_str) != Some(policy.instance_id)
    {
        return rejected(
            OfflineModelAdmissionState::WrongModel,
            "qualified_instance_not_loaded",
        );
    }
    let vision = model
        .capabilities
        .as_ref()
        .and_then(|value| value.get("vision"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let tools = model
        .capabilities
        .as_ref()
        .and_then(|value| value.get("trained_for_tool_use"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if (policy.require_vision && !vision) || (policy.require_tools && !tools) {
        return rejected(
            OfflineModelAdmissionState::MissingCapability,
            "required_capability_missing",
        );
    }
    let Some(context_length) = model.loaded_context_length else {
        return rejected(
            OfflineModelAdmissionState::InvalidRuntime,
            "loaded_context_unreported",
        );
    };
    if context_length < policy.required_context_length {
        return rejected(
            OfflineModelAdmissionState::InsufficientContext,
            "loaded_context_below_64k",
        );
    }
    let Some(parallel) = model.loaded_parallelism else {
        return rejected(
            OfflineModelAdmissionState::InvalidRuntime,
            "loaded_parallelism_unreported",
        );
    };
    if parallel != policy.generation_capacity {
        return rejected(
            OfflineModelAdmissionState::InvalidRuntime,
            "loaded_parallelism_must_equal_one",
        );
    }
    OfflineModelAdmission {
        state: OfflineModelAdmissionState::Ready,
        admitted_tier: Some("64k".to_string()),
        runtime: Some(OfflineRuntimeIdentity {
            model_id: model.id.clone(),
            instance_id: model.loaded_instance_ids[0].clone(),
            context_length,
            parallel,
        }),
        reason_codes: Vec::new(),
    }
}
