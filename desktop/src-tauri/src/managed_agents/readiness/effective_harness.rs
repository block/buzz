use std::collections::BTreeMap;

use crate::managed_agents::{
    discovery::known_acp_runtime, normalize_agent_args, types::ManagedAgentRecord,
    GuardianProtectionLevel, GuardianRuntimeProtection, HarnessSource,
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
    /// Bound to the resolved catalog source so a custom executable cannot gain
    /// protection merely by borrowing a built-in command name.
    pub guardian_protection: GuardianRuntimeProtection,
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

pub(crate) fn guardian_runtime_protection(
    runtime_id: &str,
    source: HarnessSource,
) -> GuardianRuntimeProtection {
    // Buzz Agent's permission boundary is owned and tested in this repository.
    // Every external adapter remains unqualified until an exact-version,
    // host-mode and side-effect conformance result is published.
    if source == HarnessSource::Builtin && runtime_id == "buzz-agent" {
        return GuardianRuntimeProtection {
            level: GuardianProtectionLevel::L2,
            summary: "Buzz permission gate tested; native tool coverage is not claimed".to_string(),
            lockdown_allowed: true,
        };
    }

    GuardianRuntimeProtection {
        level: GuardianProtectionLevel::L0,
        summary: if source == HarnessSource::Custom {
            "Unsupported until this custom runtime has a signed conformance result".to_string()
        } else {
            "Protection qualification pending for this exact runtime and host mode".to_string()
        },
        lockdown_allowed: false,
    }
}

pub(crate) fn validate_guardian_launch(
    descriptor: &EffectiveHarnessDescriptor,
) -> Result<(), String> {
    if descriptor.guardian_policy != GuardianPermissionPolicy::Lockdown {
        return Ok(());
    }

    if descriptor.guardian_protection.lockdown_allowed {
        Ok(())
    } else {
        Err(format!(
            "Guardian Lockdown refused to launch {}: {}",
            descriptor.command, descriptor.guardian_protection.summary
        ))
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
    let guardian_protection = if harness_def.is_some() {
        guardian_runtime_protection("custom", HarnessSource::Custom)
    } else {
        runtime_meta
            .map(|runtime| guardian_runtime_protection(runtime.id, HarnessSource::Builtin))
            .unwrap_or_else(|| guardian_runtime_protection("custom", HarnessSource::Custom))
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
    // Guardian owns the managed harness permission mode. Preserving a generic
    // `accept-edits` or `bypass-permissions` value here would let a lower env
    // layer punch through an inherited lockdown (or bypass monitor evidence).
    // Non-Guardian permission modes remain available to unmanaged buzz-acp
    // processes, but managed agents intentionally resolve to default/dont-ask.
    effective_env
        .env
        .retain(|key, _| !key.eq_ignore_ascii_case("BUZZ_ACP_PERMISSION_MODE"));

    Ok(EffectiveHarnessDescriptor {
        command: effective_command,
        args,
        env: effective_env.env,
        guardian_policy,
        guardian_protection,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        guardian_runtime_protection, resolve_guardian_policy, validate_guardian_launch,
        EffectiveHarnessDescriptor, GuardianPermissionPolicy,
    };
    use crate::managed_agents::{GuardianProtectionLevel, HarnessSource};
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

    #[test]
    fn only_buzz_agent_is_currently_lockdown_qualified() {
        let buzz = guardian_runtime_protection("buzz-agent", HarnessSource::Builtin);
        assert_eq!(buzz.level, GuardianProtectionLevel::L2);
        assert!(buzz.lockdown_allowed);

        let codex = guardian_runtime_protection("codex", HarnessSource::Builtin);
        assert_eq!(codex.level, GuardianProtectionLevel::L0);
        assert!(!codex.lockdown_allowed);
    }

    fn descriptor(command: &str, policy: GuardianPermissionPolicy) -> EffectiveHarnessDescriptor {
        EffectiveHarnessDescriptor {
            command: command.to_string(),
            args: Vec::new(),
            env: BTreeMap::new(),
            guardian_policy: policy,
            guardian_protection: guardian_runtime_protection(command, HarnessSource::Builtin),
        }
    }

    #[test]
    fn lockdown_refuses_unqualified_runtime_before_spawn() {
        let error =
            validate_guardian_launch(&descriptor("codex-acp", GuardianPermissionPolicy::Lockdown))
                .unwrap_err();
        assert!(error.contains("Lockdown refused"));
    }

    #[test]
    fn monitor_and_qualified_lockdown_remain_available() {
        assert!(validate_guardian_launch(&descriptor(
            "codex-acp",
            GuardianPermissionPolicy::Monitor,
        ))
        .is_ok());
        assert!(validate_guardian_launch(&descriptor(
            "buzz-agent",
            GuardianPermissionPolicy::Lockdown,
        ))
        .is_ok());
    }

    #[test]
    fn custom_binary_cannot_borrow_buzz_agent_qualification() {
        let mut custom = descriptor("buzz-agent", GuardianPermissionPolicy::Lockdown);
        custom.guardian_protection =
            guardian_runtime_protection("buzz-agent", HarnessSource::Custom);

        assert!(validate_guardian_launch(&custom).is_err());
    }
}
