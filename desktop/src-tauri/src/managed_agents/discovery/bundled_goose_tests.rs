use super::tests::record_with;
use super::*;

#[test]
fn bundled_goose_is_the_only_goose_and_preserves_buzz_default() {
    let bundled = known_acp_runtime("goose-acp").unwrap();
    assert_eq!(bundled.id, "goose");
    assert_eq!(bundled.label, "Goose");
    assert_eq!(bundled.underlying_cli, None);
    assert_eq!(bundled.mcp_command, None);
    assert_eq!(bundled.model_env_var, Some("GOOSE_MODEL"));
    assert_eq!(bundled.provider_env_var, Some("GOOSE_PROVIDER"));
    assert!(bundled.cli_install_commands.is_empty());
    assert_eq!(
        normalize_agent_args("goose-acp", vec!["acp".into()]),
        Vec::<String>::new()
    );
    assert!(normalize_agent_args("goose", vec!["acp".into()]).is_empty());
    // An explicitly pinned external executable retains its CLI argument.
    assert_eq!(normalize_agent_args("/opt/goose", vec![]), vec!["acp"]);
    assert_eq!(known_acp_runtime("goose").unwrap().commands, &["goose-acp"]);
    assert_eq!(default_agent_command(), "buzz-agent");

    let mut record = record_with(Some("goose"), None, None);
    assert_eq!(record_agent_command(&record, &[]), "goose-acp");
    assert_eq!(
        KNOWN_ACP_RUNTIMES
            .iter()
            .filter(|rt| rt.id == "goose")
            .count(),
        1
    );
    assert_eq!(bundled.avatar_url, GOOSE_AVATAR_URL);
    record.provider = Some("anthropic".into());
    record.model = Some("explicit-model".into());
    record
        .env_vars
        .insert("ANTHROPIC_API_KEY".into(), "test-key".into());
    let env = crate::managed_agents::readiness::resolve_effective_agent_env(
        &record,
        &[],
        Some(bundled),
        &Default::default(),
    );
    assert_eq!(
        env.env.get("GOOSE_PROVIDER").map(String::as_str),
        Some("anthropic")
    );
    assert_eq!(
        env.env.get("GOOSE_MODEL").map(String::as_str),
        Some("explicit-model")
    );
    assert!(crate::managed_agents::readiness::agent_readiness(&env).is_ready());
    record
        .env_vars
        .insert("GOOSE_MODEL".into(), "env-model".into());
    let env = crate::managed_agents::readiness::resolve_effective_agent_env(
        &record,
        &[],
        Some(bundled),
        &Default::default(),
    );
    assert_eq!(
        env.env.get("GOOSE_MODEL").map(String::as_str),
        Some("env-model")
    );
}

#[test]
fn bundled_goose_display_defaults_match_launch_precedence() {
    use crate::managed_agents::config_bridge::{reader::read_config_surface, InheritedConfigTiers};
    let runtime = known_acp_runtime("goose").unwrap();
    let tiers = InheritedConfigTiers::default();
    let mut record = record_with(Some("goose"), None, None);
    record.provider = Some("anthropic".into());
    record.model = Some("chosen-model".into());
    let surface = read_config_surface(&record, Some(runtime), None, &tiers, None);
    assert_eq!(
        surface.normalized.model.unwrap().value.as_deref(),
        Some("chosen-model")
    );
    assert_eq!(
        surface.normalized.provider.unwrap().value.as_deref(),
        Some("anthropic")
    );
    record.model = None;
    record.provider = None;
    let tiers = InheritedConfigTiers {
        persona_model: Some("persona-model".into()),
        persona_provider: Some("openai".into()),
        ..Default::default()
    };
    let surface = read_config_surface(&record, Some(runtime), None, &tiers, None);
    assert_eq!(
        surface.normalized.model.unwrap().value.as_deref(),
        Some("persona-model")
    );
    assert_eq!(
        surface.normalized.provider.unwrap().value.as_deref(),
        Some("openai")
    );
}

#[test]
fn pilot_selections_load_as_goose() {
    use crate::managed_agents::types::ManagedAgentRecord;
    let record = record_with(Some("goose-bundled"), None, None);
    let loaded: ManagedAgentRecord =
        serde_json::from_value(serde_json::to_value(record).unwrap()).unwrap();
    assert_eq!(loaded.runtime.as_deref(), Some("goose"));
    assert_eq!(try_record_agent_command(&loaded, &[]).unwrap(), "goose-acp");
    let persona = super::tests::persona_with_runtime("pilot", Some("goose-bundled"));
    let loaded: crate::managed_agents::types::AgentDefinition =
        serde_json::from_value(serde_json::to_value(persona).unwrap()).unwrap();
    assert_eq!(loaded.runtime.as_deref(), Some("goose"));
    let global: crate::managed_agents::GlobalAgentConfig =
        serde_json::from_value(serde_json::json!({"preferred_runtime":"goose-bundled"})).unwrap();
    assert_eq!(global.preferred_runtime.as_deref(), Some("goose"));
}

#[test]
fn goose_command_uses_the_bundled_resolver() {
    // Both the catalog command and older bare command pins use the same
    // authoritative artifact, without falling back to an installed CLI.
    assert_eq!(resolve_command("goose"), resolve_bundled_goose());
    assert_eq!(resolve_command("goose-acp"), resolve_bundled_goose());
    assert_eq!(resolve_command_cached("goose"), resolve_bundled_goose());
}

#[test]
fn bundled_goose_file_and_env_precedence() {
    // Exercise the real disk reader, launch resolver and config surface in an
    // isolated process, so neither the user's Goose config nor parallel tests
    // can change the outcome.
    const CHILD: &str = "BUZZ_TEST_GOOSE_PRECEDENCE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let root = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "managed_agents::discovery::bundled_goose_tests::bundled_goose_file_and_env_precedence", "--nocapture"])
            .env(CHILD, "1")
            .env("GOOSE_PATH_ROOT", root.path())
            .env_remove("GOOSE_PROVIDER")
            .env_remove("GOOSE_MODEL")
            .status().unwrap();
        assert!(status.success());
        return;
    }
    use crate::managed_agents::config_bridge::{reader::read_config_surface, InheritedConfigTiers};
    use crate::managed_agents::readiness::resolve_effective_agent_env;
    let runtime = KnownAcpRuntime {
        default_env: &[
            ("GOOSE_MODE", "auto"),
            ("GOOSE_PROVIDER", "databricks_v2"),
            ("GOOSE_MODEL", "bundled-model"),
        ],
        ..*known_acp_runtime("goose").unwrap()
    };
    assert_eq!(
        runtime.process_defaults().collect::<Vec<_>>(),
        vec![("GOOSE_MODE", "auto")]
    );
    let mut record = record_with(Some("goose"), None, None);
    let global = Default::default();
    let tiers = InheritedConfigTiers::default();
    let env = resolve_effective_agent_env(&record, &[], Some(&runtime), &global);
    assert_eq!(env.env["GOOSE_PROVIDER"], "databricks_v2");
    assert_eq!(env.env["GOOSE_MODEL"], "bundled-model");
    let path = std::path::PathBuf::from(std::env::var_os("GOOSE_PATH_ROOT").unwrap())
        .join("config/config.yaml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    for (yaml, expected_provider, expected_model) in [
        (
            "GOOSE_PROVIDER: anthropic\nGOOSE_MODEL: file-model\n",
            "anthropic",
            "file-model",
        ),
        ("GOOSE_MODEL: file-model\n", "databricks_v2", "file-model"),
        (
            "GOOSE_PROVIDER: databricks_v2\n",
            "databricks_v2",
            "bundled-model",
        ),
    ] {
        std::fs::write(&path, yaml).unwrap();
        let env = resolve_effective_agent_env(&record, &[], Some(&runtime), &global);
        if yaml.contains("GOOSE_PROVIDER:") {
            assert!(!env.env.contains_key("GOOSE_PROVIDER"));
        } else {
            assert_eq!(env.env["GOOSE_PROVIDER"], expected_provider);
        }
        if yaml.contains("GOOSE_MODEL:") {
            assert!(!env.env.contains_key("GOOSE_MODEL"));
        } else {
            assert_eq!(env.env["GOOSE_MODEL"], expected_model);
        }
        let surface = read_config_surface(&record, Some(&runtime), None, &tiers, None);
        assert_eq!(
            surface.normalized.provider.unwrap().value.as_deref(),
            Some(expected_provider)
        );
        assert_eq!(
            surface.normalized.model.unwrap().value.as_deref(),
            Some(expected_model)
        );
    }
    // Existing exported settings must remain inherited by the child.
    std::fs::write(&path, "{}\n").unwrap();
    std::env::set_var("GOOSE_PROVIDER", "exported-provider");
    std::env::set_var("GOOSE_MODEL", "exported-model");
    let env = resolve_effective_agent_env(&record, &[], Some(&runtime), &global);
    assert!(!env.env.contains_key("GOOSE_PROVIDER"));
    assert!(!env.env.contains_key("GOOSE_MODEL"));
    std::env::remove_var("GOOSE_PROVIDER");
    std::env::remove_var("GOOSE_MODEL");
    std::fs::write(
        &path,
        "GOOSE_PROVIDER: anthropic\nGOOSE_MODEL: file-model\n",
    )
    .unwrap();

    record.provider = Some("structured-provider".into());
    record.model = Some("structured-model".into());
    for (key, value) in [
        ("GOOSE_PROVIDER", "env-provider"),
        ("GOOSE_MODEL", "env-model"),
    ] {
        record.env_vars.insert(key.into(), value.into());
    }
    let env = resolve_effective_agent_env(&record, &[], Some(&runtime), &global);
    assert_eq!(env.env["GOOSE_PROVIDER"], "env-provider");
    assert_eq!(env.env["GOOSE_MODEL"], "env-model");
    let surface = read_config_surface(&record, Some(&runtime), None, &tiers, None);
    assert_eq!(
        surface.normalized.provider.unwrap().value.as_deref(),
        Some("env-provider")
    );
    assert_eq!(
        surface.normalized.model.unwrap().value.as_deref(),
        Some("env-model")
    );
}
