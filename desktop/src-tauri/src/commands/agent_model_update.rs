use crate::managed_agents::ManagedAgentRecord;

/// Apply model/provider/system-prompt patches while preserving the linked
/// instance write guard. Definition-linked instances resolve these fields from
/// their definition, so persisting instance overrides would create dead state.
pub(super) fn apply_model_provider_prompt_update(
    record: &mut ManagedAgentRecord,
    model: Option<Option<String>>,
    provider: Option<Option<String>>,
    system_prompt: Option<Option<String>>,
) {
    if record.persona_id.is_some() {
        return;
    }
    if let Some(model_update) = model {
        record.model = model_update;
    }
    if let Some(provider_update) = provider {
        record.provider = provider_update;
    }
    if let Some(prompt_update) = system_prompt {
        record.system_prompt = prompt_update;
    }
}
