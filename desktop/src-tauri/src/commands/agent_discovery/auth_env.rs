use std::collections::BTreeMap;

use tauri::AppHandle;

use crate::managed_agents::{
    baked_build_env, known_acp_runtime, load_global_agent_config, load_managed_agent_configs,
    load_personas, merged_user_env, record_agent_command, resolve_effective_agent_env,
    AgentDefinition, GlobalAgentConfig, ManagedAgentRecord,
};

/// Effective child environments that can supply pre-probe auth evidence.
///
/// Runtime discovery is global rather than agent-specific, so a runtime is
/// authenticated when the desktop process or any configured child environment
/// can authenticate it. Readiness still evaluates one exact child environment.
pub(super) fn load(app: &AppHandle) -> Vec<BTreeMap<String, String>> {
    let personas = load_personas(app).unwrap_or_else(|error| {
        tracing::warn!(%error, "runtime auth discovery could not load personas");
        vec![]
    });
    let records = load_managed_agent_configs(app).unwrap_or_else(|error| {
        tracing::warn!(%error, "runtime auth discovery could not load agent configs");
        vec![]
    });
    let global = load_global_agent_config(app).unwrap_or_else(|error| {
        tracing::warn!(%error, "runtime auth discovery could not load global config");
        GlobalAgentConfig::default()
    });
    configured(&personas, &records, &global)
}

fn configured(
    personas: &[AgentDefinition],
    records: &[ManagedAgentRecord],
    global: &GlobalAgentConfig,
) -> Vec<BTreeMap<String, String>> {
    let mut base = baked_build_env();
    base.extend(merged_user_env(&BTreeMap::new(), &global.env_vars));

    let persona_envs = personas.iter().cloned().map(|persona| {
        let record = persona.into_agent_record();
        let command = record_agent_command(&record, &[]);
        resolve_effective_agent_env(&record, &[], known_acp_runtime(&command), global).env
    });
    let agent_envs = records.iter().map(|record| {
        let command = record_agent_command(record, personas);
        resolve_effective_agent_env(record, personas, known_acp_runtime(&command), global).env
    });

    std::iter::once(base)
        .chain(persona_envs)
        .chain(agent_envs)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn persona(env_vars: BTreeMap<String, String>) -> AgentDefinition {
        AgentDefinition {
            id: "persona".into(),
            display_name: "Persona".into(),
            avatar_url: None,
            system_prompt: "Help".into(),
            runtime: Some("codex".into()),
            model: None,
            provider: None,
            name_pool: vec![],
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            env_vars,
            respond_to: None,
            respond_to_allowlist: vec![],
            parallelism: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn collects_global_persona_and_agent_env_with_last_wins_precedence() {
        let global = GlobalAgentConfig {
            env_vars: BTreeMap::from([
                ("GLOBAL_KEY".into(), "global".into()),
                ("SHARED_KEY".into(), "global".into()),
            ]),
            ..Default::default()
        };
        let persona = persona(BTreeMap::from([
            ("PERSONA_KEY".into(), "persona".into()),
            ("SHARED_KEY".into(), "persona".into()),
        ]));
        let mut agent = persona.clone().into_agent_record();
        agent.persona_id = Some(persona.id.clone());
        agent.env_vars = BTreeMap::from([
            ("AGENT_KEY".into(), "agent".into()),
            ("SHARED_KEY".into(), String::new()),
        ]);

        let envs = configured(std::slice::from_ref(&persona), &[agent], &global);

        assert!(envs.iter().any(|env| env.get("GLOBAL_KEY") == Some(&"global".into())));
        assert!(envs.iter().any(|env| env.get("PERSONA_KEY") == Some(&"persona".into())));
        let agent_env = envs.last().expect("agent environment");
        assert_eq!(agent_env.get("AGENT_KEY").map(String::as_str), Some("agent"));
        assert_eq!(agent_env.get("SHARED_KEY").map(String::as_str), Some(""));
    }
}
