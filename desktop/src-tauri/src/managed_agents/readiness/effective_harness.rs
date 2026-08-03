use std::collections::BTreeMap;

use crate::managed_agents::{
    discovery::known_acp_runtime, normalize_agent_args, types::ManagedAgentRecord,
};

use super::resolve_effective_agent_env_with_def;

/// The complete effective description of a harness spawn.
#[derive(Debug, Clone)]
pub(crate) struct EffectiveHarnessDescriptor {
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// Resolved separately so generic environment layering cannot weaken it.
    pub guardian_policy: GuardianPermissionPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum GuardianPermissionPolicy {
    Monitor,
    Lockdown,
}

impl GuardianPermissionPolicy {
    pub(crate) fn as_env_value(self) -> &'static str {
        match self {
            Self::Monitor => "default",
            Self::Lockdown => "dont-ask",
        }
    }
}

fn resolve_guardian_policy<'a>(
    layers: impl IntoIterator<Item = &'a BTreeMap<String, String>>,
) -> GuardianPermissionPolicy {
    let lockdown_selected = layers.into_iter().any(|env| {
        env.iter().any(|(key, value)| {
            key.eq_ignore_ascii_case("BUZZ_ACP_PERMISSION_MODE")
                && matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "dont-ask" | "dontask" | "plan"
                )
        })
    });
    if lockdown_selected {
        GuardianPermissionPolicy::Lockdown
    } else {
        GuardianPermissionPolicy::Monitor
    }
}

/// Resolve command, arguments, generic environment, and authoritative policy.
pub(crate) fn resolve_effective_harness_descriptor(
    record: &ManagedAgentRecord,
    personas: &[crate::managed_agents::types::AgentDefinition],
    global: &crate::managed_agents::GlobalAgentConfig,
) -> Result<EffectiveHarnessDescriptor, String> {
    let effective_command = crate::managed_agents::try_record_agent_command(record, personas)?;
    let runtime_meta = known_acp_runtime(&effective_command);
    let harness_def = {
        let runtime_id = record
            .runtime
            .as_deref()
            .or_else(|| {
                record.persona_id.as_deref().and_then(|pid| {
                    personas
                        .iter()
                        .find(|p| p.id == pid)
                        .and_then(|p| p.runtime.as_deref())
                })
            })
            .unwrap_or("");
        crate::managed_agents::custom_harnesses::lookup_loaded_harness_by_id(runtime_id)
    };
    let args = {
        let record_args = record.agent_args.clone();
        let instance_has_args = record_args.iter().any(|arg| !arg.trim().is_empty());
        if instance_has_args {
            normalize_agent_args(&effective_command, record_args)
        } else if let Some(ref definition) = harness_def {
            normalize_agent_args(&effective_command, definition.args.clone())
        } else {
            normalize_agent_args(&effective_command, record_args)
        }
    };
    let live_persona = record
        .persona_id
        .as_deref()
        .and_then(|id| personas.iter().find(|persona| persona.id == id));
    let guardian_policy = resolve_guardian_policy(
        harness_def
            .iter()
            .map(|definition| &definition.env)
            .chain(std::iter::once(&global.env_vars))
            .chain(live_persona.map(|persona| &persona.env_vars))
            .chain(std::iter::once(&record.env_vars)),
    );
    let mut effective_env =
        resolve_effective_agent_env_with_def(record, personas, runtime_meta, global, harness_def);
    effective_env
        .env
        .retain(|key, _| !key.eq_ignore_ascii_case("BUZZ_ACP_PERMISSION_MODE"));

    Ok(EffectiveHarnessDescriptor {
        command: effective_command,
        args,
        env: effective_env.env,
        guardian_policy,
    })
}

#[cfg(test)]
mod tests {
    use super::{resolve_guardian_policy, GuardianPermissionPolicy};
    use std::collections::BTreeMap;

    fn layer(value: &str) -> BTreeMap<String, String> {
        BTreeMap::from([("BUZZ_ACP_PERMISSION_MODE".to_string(), value.to_string())])
    }

    #[test]
    fn lower_layers_cannot_weaken_lockdown() {
        for lockdown_index in 0..4 {
            let mut layers = vec![layer("bypass-permissions"); 4];
            layers[lockdown_index] = layer("dont-ask");
            assert_eq!(
                resolve_guardian_policy(layers.iter()),
                GuardianPermissionPolicy::Lockdown,
                "lockdown layer {lockdown_index} was weakened"
            );
        }
    }

    #[test]
    fn absent_or_permissive_legacy_values_resolve_to_monitor() {
        let empty = BTreeMap::new();
        let permissive = layer("accept-edits");
        assert_eq!(
            resolve_guardian_policy([&empty, &permissive]),
            GuardianPermissionPolicy::Monitor
        );
    }

    #[test]
    fn policy_key_matching_is_case_insensitive() {
        let env = BTreeMap::from([("buzz_acp_permission_mode".to_string(), "PLAN".to_string())]);
        assert_eq!(
            resolve_guardian_policy([&env]),
            GuardianPermissionPolicy::Lockdown
        );
    }
}
