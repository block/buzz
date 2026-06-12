use super::types::*;

/// Route a config write to the correct mechanism and return the result.
///
/// This does NOT execute the write — it determines what mechanism should be
/// used and returns the `WriteConfigResult` describing the action. The caller
/// (Tauri command) is responsible for executing the actual write (updating
/// the record and restarting, or sending an observer control event).
pub(crate) fn plan_config_write(
    surface: &RuntimeConfigSurface,
    target: &WriteConfigTarget,
) -> WriteConfigResult {
    let field = match target {
        WriteConfigTarget::Model => surface.normalized.model.as_ref(),
        WriteConfigTarget::Provider => surface.normalized.provider.as_ref(),
        WriteConfigTarget::Mode => surface.normalized.mode.as_ref(),
        WriteConfigTarget::ThinkingEffort => surface.normalized.thinking_effort.as_ref(),
        WriteConfigTarget::MaxOutputTokens => surface.normalized.max_output_tokens.as_ref(),
        WriteConfigTarget::ContextLimit => surface.normalized.context_limit.as_ref(),
        WriteConfigTarget::SystemPrompt => surface.normalized.system_prompt.as_ref(),
        WriteConfigTarget::Advanced { key } => {
            let adv = surface.advanced.iter().find(|f| f.key == *key);
            return match adv {
                Some(f) if f.is_writable => WriteConfigResult {
                    success: true,
                    mechanism_used: f.write_via.clone(),
                    requires_restart: matches!(
                        f.write_via,
                        ConfigWriteMechanism::RespawnWithEnvVar { .. }
                    ),
                    error: None,
                },
                Some(_) => WriteConfigResult {
                    success: false,
                    mechanism_used: ConfigWriteMechanism::ReadOnly,
                    requires_restart: false,
                    error: Some(format!("field '{key}' is read-only")),
                },
                None => WriteConfigResult {
                    success: false,
                    mechanism_used: ConfigWriteMechanism::ReadOnly,
                    requires_restart: false,
                    error: Some(format!("unknown advanced field '{key}'")),
                },
            };
        }
    };

    match field {
        Some(f) if f.is_writable => WriteConfigResult {
            success: true,
            mechanism_used: f.write_via.clone(),
            requires_restart: matches!(f.write_via, ConfigWriteMechanism::RespawnWithEnvVar { .. }),
            error: None,
        },
        Some(_) => WriteConfigResult {
            success: false,
            mechanism_used: ConfigWriteMechanism::ReadOnly,
            requires_restart: false,
            error: Some("field is read-only".to_string()),
        },
        None => WriteConfigResult {
            success: false,
            mechanism_used: ConfigWriteMechanism::ReadOnly,
            requires_restart: false,
            error: Some("field not available for this runtime".to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface_with_writable_model() -> RuntimeConfigSurface {
        RuntimeConfigSurface {
            runtime_id: Some("goose".to_string()),
            runtime_label: Some("Goose".to_string()),
            is_pre_spawn: false,
            normalized: NormalizedConfig {
                model: Some(NormalizedField {
                    value: Some("claude-opus-4".to_string()),
                    origin: ConfigOrigin::BuzzExplicit,
                    is_writable: true,
                    write_via: ConfigWriteMechanism::AcpSetConfigOption {
                        config_id: "model".to_string(),
                    },
                    overridden_value: None,
                    overridden_origin: None,
                }),
                provider: None,
                mode: None,
                thinking_effort: None,
                max_output_tokens: None,
                context_limit: None,
                system_prompt: None,
            },
            advanced: vec![],
            sources: ConfigSourceReport {
                acp_native: ConfigTierStatus::NotApplicable,
                acp_config_options: ConfigTierStatus::Available,
                env_vars: ConfigTierStatus::Available,
                config_file: ConfigTierStatus::NotApplicable,
                config_file_path: None,
            },
        }
    }

    #[test]
    fn writable_model_returns_acp_mechanism() {
        let surface = surface_with_writable_model();
        let result = plan_config_write(&surface, &WriteConfigTarget::Model);
        assert!(result.success);
        assert!(!result.requires_restart);
        assert!(matches!(
            result.mechanism_used,
            ConfigWriteMechanism::AcpSetConfigOption { .. }
        ));
    }

    #[test]
    fn missing_field_returns_error() {
        let surface = surface_with_writable_model();
        let result = plan_config_write(&surface, &WriteConfigTarget::Mode);
        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[test]
    fn respawn_mechanism_requires_restart() {
        let mut surface = surface_with_writable_model();
        surface.normalized.model = Some(NormalizedField {
            value: Some("my-model".to_string()),
            origin: ConfigOrigin::EnvVar,
            is_writable: true,
            write_via: ConfigWriteMechanism::RespawnWithEnvVar {
                env_key: "GOOSE_MODEL".to_string(),
            },
            overridden_value: None,
            overridden_origin: None,
        });
        let result = plan_config_write(&surface, &WriteConfigTarget::Model);
        assert!(result.success);
        assert!(result.requires_restart);
    }
}
