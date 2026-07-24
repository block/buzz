use std::collections::BTreeMap;

use super::*;
use crate::managed_agents::discovery::known_acp_runtime_exact;

fn effective_env(env: BTreeMap<String, String>) -> EffectiveAgentEnv {
    EffectiveAgentEnv {
        env,
        config_file_path: None,
        effective_command: "buzz-lmstudio-agent".to_string(),
    }
}

#[test]
fn native_runtime_requires_a_structured_model() {
    let missing = agent_readiness(&effective_env(BTreeMap::from([(
        "BUZZ_AGENT_PROVIDER".to_string(),
        "lmstudio-native".to_string(),
    )])));
    assert_eq!(
        missing.requirements(),
        &[Requirement::NormalizedField {
            field: "model".to_string()
        }]
    );

    let configured = effective_env(BTreeMap::from([
        (
            "BUZZ_AGENT_PROVIDER".to_string(),
            "lmstudio-native".to_string(),
        ),
        ("LM_STUDIO_MODEL".to_string(), "qwen/test".to_string()),
    ]));
    assert!(agent_readiness(&configured).is_ready());
}

#[test]
fn reserved_env_cannot_bypass_missing_structured_model() {
    let definition: crate::managed_agents::AgentDefinition =
        serde_json::from_value(serde_json::json!({
            "id": "lm-persona",
            "display_name": "LM",
            "system_prompt": "",
            "created_at": "",
            "updated_at": ""
        }))
        .expect("minimal definition");
    let mut record = definition.into_agent_record();
    record.agent_command_override = Some("buzz-lmstudio-agent".to_string());
    record.env_vars.insert(
        "LM_STUDIO_MODEL".to_string(),
        "ambient-must-not-count".to_string(),
    );
    let runtime = known_acp_runtime_exact("buzz-lmstudio-agent");

    let effective =
        resolve_effective_agent_env(&record, &[], runtime, &GlobalAgentConfig::default());

    assert!(!effective.env.contains_key("LM_STUDIO_MODEL"));
    assert!(agent_readiness(&effective)
        .requirements()
        .contains(&Requirement::NormalizedField {
            field: "model".to_string()
        }));
}
