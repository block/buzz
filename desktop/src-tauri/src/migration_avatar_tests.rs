use super::*;

#[test]
fn refresh_builtin_agent_avatars_updates_seeded_values_and_preserves_customizations() {
    use sha2::{Digest as _, Sha256};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("managed-agents.json");
    let old_diego = "data:image/png;base64,old-diego";
    let old_murietta = "data:image/png;base64,old-murietta";
    let diego_hash = hex::encode(Sha256::digest(old_diego.as_bytes()));
    let murietta_hash = hex::encode(Sha256::digest(old_murietta.as_bytes()));
    let legacy_avatars = [
        LegacyBuiltInAvatar {
            persona_id: "builtin:diego",
            data_url_sha256: diego_hash.as_str(),
            sanitized_media_sha256: "",
            persona_content_hash: "",
        },
        LegacyBuiltInAvatar {
            persona_id: "builtin:murietta",
            data_url_sha256: murietta_hash.as_str(),
            sanitized_media_sha256: "",
            persona_content_hash: "",
        },
    ];
    let definition = crate::managed_agents::AgentDefinition {
        id: "builtin:diego".to_string(),
        display_name: "Diego".to_string(),
        avatar_url: Some(old_diego.to_string()),
        system_prompt: "A customized built-in prompt".to_string(),
        runtime: Some("goose".to_string()),
        model: Some("test-model".to_string()),
        provider: Some("test-provider".to_string()),
        name_pool: vec!["Diego".to_string()],
        is_builtin: true,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        env_vars: Default::default(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "before".to_string(),
        updated_at: "before".to_string(),
    };
    let old_persona_version = crate::managed_agents::persona_events::persona_content_hash(
        &crate::managed_agents::persona_events::persona_event_content(&definition),
    );
    let mut definition_record = serde_json::to_value(definition.into_agent_record()).unwrap();
    definition_record["future_definition_field"] = serde_json::json!("preserved");

    let instance =
        |pubkey: &str, persona_id: &str, avatar_url: &str, persona_source_version: &str| {
            serde_json::json!({
                "name": pubkey,
                "pubkey": pubkey,
                "persona_id": persona_id,
                "relay_url": "ws://localhost:3000",
                "avatar_url": avatar_url,
                "acp_command": "buzz-acp",
                "agent_command": "goose",
                "agent_args": [],
                "mcp_command": "",
                "turn_timeout_seconds": 320,
                "parallelism": 4,
                "system_prompt": "A customized built-in prompt",
                "model": "test-model",
                "provider": "test-provider",
                "persona_source_version": persona_source_version,
                "env_vars": {},
                "start_on_app_launch": true,
                "created_at": "before",
                "updated_at": "before",
                "last_started_at": null,
                "last_stopped_at": null,
                "last_exit_code": null,
                "last_error": null
            })
        };
    let mut synced_instance = instance(
        "diego-instance",
        "builtin:diego",
        old_diego,
        &old_persona_version,
    );
    synced_instance["future_instance_field"] = serde_json::json!("preserved");
    let records = serde_json::Value::Array(vec![
        definition_record,
        synced_instance,
        instance(
            "drifted-diego-instance",
            "builtin:diego",
            old_diego,
            "genuinely-drifted-version",
        ),
        instance(
            "murietta-instance",
            "builtin:murietta",
            "data:image/png;base64,user-customized",
            "murietta-version",
        ),
        instance(
            "custom-instance",
            "custom:diego",
            old_diego,
            "custom-version",
        ),
    ]);
    std::fs::write(&path, serde_json::to_vec_pretty(&records).unwrap()).unwrap();

    refresh_builtin_agent_avatars_in_file(&path, &legacy_avatars, "after");

    let migrated: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let new_diego = crate::managed_agents::built_in_persona_avatar_url("builtin:diego").unwrap();
    assert_eq!(migrated[0]["avatar_url"], new_diego);
    assert_eq!(migrated[0]["updated_at"], "after");
    assert_eq!(migrated[0]["future_definition_field"], "preserved");
    let migrated_definition: crate::managed_agents::ManagedAgentRecord =
        serde_json::from_value(migrated[0].clone()).unwrap();
    let new_persona_version = crate::managed_agents::persona_events::persona_content_hash(
        &crate::managed_agents::persona_events::persona_event_content(
            &migrated_definition.to_definition_view().unwrap(),
        ),
    );
    assert_ne!(new_persona_version, old_persona_version);
    assert_eq!(migrated[1]["avatar_url"], new_diego);
    assert_eq!(migrated[1]["updated_at"], "after");
    assert_eq!(migrated[1]["persona_source_version"], new_persona_version);
    assert_eq!(migrated[1]["future_instance_field"], "preserved");
    assert_eq!(migrated[2]["avatar_url"], new_diego);
    assert_eq!(migrated[2]["updated_at"], "after");
    assert_eq!(
        migrated[2]["persona_source_version"],
        "genuinely-drifted-version"
    );
    assert_eq!(
        migrated[3]["avatar_url"],
        "data:image/png;base64,user-customized"
    );
    assert_eq!(migrated[3]["updated_at"], "before");
    assert_eq!(migrated[4]["avatar_url"], old_diego);
    assert_eq!(migrated[4]["updated_at"], "before");

    let once = std::fs::read(&path).unwrap();
    refresh_builtin_agent_avatars_in_file(&path, &legacy_avatars, "later");
    assert_eq!(std::fs::read(&path).unwrap(), once);
}

#[test]
fn current_builtin_agent_avatars_do_not_match_legacy_hashes() {
    use sha2::{Digest as _, Sha256};

    for legacy in LEGACY_BUILTIN_AVATARS {
        let current =
            crate::managed_agents::built_in_persona_avatar_url(legacy.persona_id).unwrap();
        assert_ne!(
            hex::encode(Sha256::digest(current.as_bytes())),
            legacy.data_url_sha256
        );
    }
}

#[test]
fn refresh_builtin_agent_avatars_updates_versions_without_stored_definitions() {
    use sha2::{Digest as _, Sha256};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("managed-agents.json");
    let old_diego = "data:image/png;base64,old-diego";
    let diego_hash = hex::encode(Sha256::digest(old_diego.as_bytes()));
    let mut old_definition =
        crate::managed_agents::built_in_persona_definition("builtin:diego", "before").unwrap();
    old_definition.avatar_url = Some(old_diego.to_string());
    let old_version = crate::managed_agents::persona_events::persona_content_hash(
        &crate::managed_agents::persona_events::persona_event_content(&old_definition),
    );
    let legacy_avatars = [LegacyBuiltInAvatar {
        persona_id: "builtin:diego",
        data_url_sha256: diego_hash.as_str(),
        sanitized_media_sha256: "",
        persona_content_hash: old_version.as_str(),
    }];
    let records = serde_json::json!([
        {
            "name": "synced-diego",
            "pubkey": "synced-diego",
            "persona_id": "builtin:diego",
            "avatar_url": old_diego,
            "persona_source_version": old_version,
            "updated_at": "before"
        },
        {
            "name": "drifted-diego",
            "pubkey": "drifted-diego",
            "persona_id": "builtin:diego",
            "avatar_url": old_diego,
            "persona_source_version": "genuinely-drifted-version",
            "updated_at": "before"
        }
    ]);
    std::fs::write(&path, serde_json::to_vec_pretty(&records).unwrap()).unwrap();

    refresh_builtin_agent_avatars_in_file(&path, &legacy_avatars, "after");

    let migrated: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let current_definition =
        crate::managed_agents::built_in_persona_definition("builtin:diego", "after").unwrap();
    let current_version = crate::managed_agents::persona_events::persona_content_hash(
        &crate::managed_agents::persona_events::persona_event_content(&current_definition),
    );
    let new_diego = crate::managed_agents::built_in_persona_avatar_url("builtin:diego").unwrap();
    assert_eq!(migrated[0]["avatar_url"], new_diego);
    assert_eq!(migrated[0]["persona_source_version"], current_version);
    assert_eq!(migrated[0]["updated_at"], "after");
    assert_eq!(migrated[1]["avatar_url"], new_diego);
    assert_eq!(
        migrated[1]["persona_source_version"],
        "genuinely-drifted-version"
    );
    assert_eq!(migrated[1]["updated_at"], "after");
}

#[test]
fn refresh_builtin_agent_avatars_updates_uploaded_media_urls() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("managed-agents.json");
    let media_sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let different_sha256 = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let old_persona_version = "legacy-persona-version";
    let legacy_avatars = [LegacyBuiltInAvatar {
        persona_id: "builtin:diego",
        data_url_sha256: "not-a-data-url-hash",
        sanitized_media_sha256: media_sha256,
        persona_content_hash: old_persona_version,
    }];
    let matching_url = format!("https://relay.example/media/{media_sha256}.png?download=1");
    let custom_url = format!("https://relay.example/media/{different_sha256}.png");
    let embedded_hash_url = format!("https://relay.example/media/avatar-{media_sha256}.png");
    let records = serde_json::json!([
        {
            "name": "synced-diego",
            "pubkey": "synced-diego",
            "persona_id": "builtin:diego",
            "avatar_url": matching_url,
            "persona_source_version": old_persona_version,
            "updated_at": "before"
        },
        {
            "name": "drifted-diego",
            "pubkey": "drifted-diego",
            "persona_id": "builtin:diego",
            "avatar_url": matching_url,
            "persona_source_version": "genuinely-drifted-version",
            "updated_at": "before"
        },
        {
            "name": "custom-diego",
            "pubkey": "custom-diego",
            "persona_id": "builtin:diego",
            "avatar_url": custom_url,
            "persona_source_version": old_persona_version,
            "updated_at": "before"
        },
        {
            "name": "embedded-hash-diego",
            "pubkey": "embedded-hash-diego",
            "persona_id": "builtin:diego",
            "avatar_url": embedded_hash_url,
            "persona_source_version": old_persona_version,
            "updated_at": "before"
        }
    ]);
    std::fs::write(&path, serde_json::to_vec_pretty(&records).unwrap()).unwrap();

    refresh_builtin_agent_avatars_in_file(&path, &legacy_avatars, "after");

    let migrated: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let current_definition =
        crate::managed_agents::built_in_persona_definition("builtin:diego", "after").unwrap();
    let current_version = crate::managed_agents::persona_events::persona_content_hash(
        &crate::managed_agents::persona_events::persona_event_content(&current_definition),
    );
    let new_diego = crate::managed_agents::built_in_persona_avatar_url("builtin:diego").unwrap();
    assert_eq!(migrated[0]["avatar_url"], new_diego);
    assert_eq!(migrated[0]["persona_source_version"], current_version);
    assert_eq!(migrated[0]["updated_at"], "after");
    assert_eq!(migrated[1]["avatar_url"], new_diego);
    assert_eq!(
        migrated[1]["persona_source_version"],
        "genuinely-drifted-version"
    );
    assert_eq!(migrated[1]["updated_at"], "after");
    assert_eq!(migrated[2]["avatar_url"], custom_url);
    assert_eq!(migrated[2]["updated_at"], "before");
    assert_eq!(migrated[3]["avatar_url"], embedded_hash_url);
    assert_eq!(migrated[3]["updated_at"], "before");
}
