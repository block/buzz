use super::*;
use crate::managed_agents as agents;

fn record() -> ManagedAgentRecord {
    let mut record =
        agents::runtime::test_fixtures::fixture(agents::RespondTo::OwnerOnly, vec![], None);
    let keys = nostr::Keys::generate();
    record.pubkey = keys.public_key().to_hex();
    record.private_key_nsec = keys.secret_key().to_secret_hex();
    record
}

fn attest(record: &mut ManagedAgentRecord, owner: &nostr::Keys) -> String {
    record.auth_tag = Some(
        buzz_sdk_pkg::nip_oa::compute_auth_tag(
            owner,
            &nostr::PublicKey::from_hex(&record.pubkey).unwrap(),
            "",
        )
        .unwrap(),
    );
    owner.public_key().to_hex()
}

fn config(host: &str) -> RuntimeConfiguration {
    RuntimeConfiguration {
        id: uuid::Uuid::new_v4().to_string(),
        revision: uuid::Uuid::new_v4().to_string(),
        name: "Focused".into(),
        host: host.into(),
        runtime: "buzz-agent".into(),
        model: "fixture-model".into(),
        provider: Some("openai".into()),
        workspace: None,
        credential_refs: BTreeMap::new(),
    }
}

fn save(
    record: &mut ManagedAgentRecord,
    owner: &str,
    community: &str,
    entry: RuntimeConfiguration,
) -> RuntimeConfiguration {
    let host = entry.host.clone();
    record
        .runtime_configurations
        .replace(
            owner,
            community,
            &host,
            RuntimeConfigurations {
                selected: Some(entry.id.clone()),
                entries: vec![entry],
            },
        )
        .unwrap();
    record
        .runtime_configurations
        .get(owner, community)
        .entries
        .remove(0)
}

#[test]
fn migration_keeps_default_and_launch_projection_is_not_persisted() {
    let original = record();
    let mut json = serde_json::to_value(&original).unwrap();
    json.as_object_mut()
        .unwrap()
        .remove("runtime_configurations");
    let mut migrated: ManagedAgentRecord = serde_json::from_value(json).unwrap();
    assert!(selected_reference(&migrated, Some("owner"), "community")
        .unwrap()
        .is_none());
    migrated.runtime_configurations.launch = Some(config("host"));
    let resolved =
        agents::effective_config::resolve_effective_config(&migrated, &[], &Default::default())
            .require_resolved()
            .unwrap();
    assert_eq!(resolved.model.value.as_deref(), Some("fixture-model"));
    assert_eq!(
        resolved.model.source,
        agents::effective_config::ConfigSource::RuntimeConfiguration
    );
    let restored: ManagedAgentRecord =
        serde_json::from_value(serde_json::to_value(&migrated).unwrap()).unwrap();
    assert!(selected(&restored).unwrap().is_none());
    assert_eq!(restored.pubkey, original.pubkey);
    assert_eq!(restored.private_key_nsec, original.private_key_nsec);
}

#[test]
fn agent_in_two_communities_preserves_private_sets_and_rejects_foreign_references() {
    let mut record = record();
    let owner = attest(&mut record, &nostr::Keys::generate());
    let first = save(&mut record, &owner, "one", config("host-one"));
    let second = save(&mut record, &owner, "two", config("host-two"));
    let saved_two = record.runtime_configurations.get(&owner, "two");
    let mut changed = first.clone();
    changed.name = "Edited in one".into();
    let changed = save(&mut record, &owner, "one", changed);
    assert_ne!(changed.revision, first.revision);
    assert_eq!(record.runtime_configurations.get(&owner, "two"), saved_two);
    assert_eq!(
        selected_reference(&record, Some(&owner), "two").unwrap(),
        Some(second.reference())
    );
    let view = record.runtime_configurations.get(&owner, "one");
    assert!(!serde_json::to_string(&view).unwrap().contains(&second.id));
    assert!(!serde_json::to_string(&catalog(
        &record,
        &[],
        &Default::default(),
        "host-one",
        &owner,
        "one"
    ))
    .unwrap()
    .contains(&second.id));
    assert!(record
        .runtime_configurations
        .get("other-owner", "one")
        .entries
        .is_empty());
    for (reference, host, requested_owner, community) in [
        (&second, "host-two", owner.as_str(), "one"),
        (&first, "host-two", owner.as_str(), "one"),
        (&changed, "host-one", "other-owner", "one"),
    ] {
        assert!(prepare(
            &record,
            Some(&reference.reference()),
            &[],
            &Default::default(),
            host,
            requested_owner,
            community
        )
        .is_err());
    }
    assert!(catalog(
        &record,
        &[],
        &Default::default(),
        "host-one",
        "other-owner",
        "one"
    )
    .iter()
    .all(|c| !c.eligible));
    let mut bad_selection = view.clone();
    bad_selection.selected = Some(second.id);
    assert!(record
        .runtime_configurations
        .replace(&owner, "one", "host-one", bad_selection)
        .is_err());
    assert_eq!(record.runtime_configurations.get(&owner, "one"), view);
}

#[test]
fn other_host_entries_are_preserved_not_a_global_validation_failure() {
    let mut record = record();
    let first = save(&mut record, "owner", "one", config("host-one"));
    let mut set = record.runtime_configurations.get("owner", "one");
    let second = config("host-two");
    set.entries.push(second.clone());
    set.selected = None;
    record
        .runtime_configurations
        .replace("owner", "one", "host-two", set)
        .unwrap();
    let mut set = record.runtime_configurations.get("owner", "one");
    set.selected = Some(first.id.clone());
    record
        .runtime_configurations
        .replace("owner", "one", "host-one", set.clone())
        .unwrap();
    set.selected = Some(second.id.clone());
    assert!(record
        .runtime_configurations
        .replace("owner", "one", "host-one", set)
        .is_err());
    let mut set = record.runtime_configurations.get("owner", "one");
    set.entries.retain(|c| c.id == first.id);
    assert!(record
        .runtime_configurations
        .replace("owner", "one", "host-one", set)
        .is_err());
    let mut set = record.runtime_configurations.get("owner", "one");
    set.entries[1].model = "tamper".into();
    assert!(record
        .runtime_configurations
        .replace("owner", "one", "host-one", set)
        .is_err());
}

#[test]
fn malformed_or_stale_reference_is_rejected_before_launch_resolution() {
    let mut record = record();
    let owner = attest(&mut record, &nostr::Keys::generate());
    let mut named = save(&mut record, &owner, "one", config("host"));
    named.revision = uuid::Uuid::new_v4().to_string();
    assert!(prepare(
        &record,
        Some(&named.reference()),
        &[],
        &Default::default(),
        "host",
        &owner,
        "one"
    )
    .is_err());
    named
        .credential_refs
        .insert("BUZZ_PRIVATE_KEY".into(), "SECRET".into());
    assert!(RuntimeConfigurations {
        selected: None,
        entries: vec![named]
    }
    .validate()
    .is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn shared_spawn_registers_exact_plan_and_rejects_edits_or_lost_identity() {
    use std::os::unix::fs::PermissionsExt;
    use tauri::Manager;
    let _guard = agents::lock_path_mutex();
    let temp = tempfile::tempdir().unwrap();
    struct Restore(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            for (key, value) in self.0.drain(..) {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
            agents::clear_resolve_cache();
        }
    }
    let _restore = Restore(
        ["HOME", "XDG_DATA_HOME", "PATH"]
            .map(|key| (key, std::env::var_os(key)))
            .to_vec(),
    );
    std::env::set_var("HOME", temp.path());
    std::env::set_var("XDG_DATA_HOME", temp.path());
    std::env::set_var("PATH", format!("{}:/usr/bin:/bin", temp.path().display()));
    agents::clear_resolve_cache();
    let capture = temp.path().join("capture");
    // A fixture child at the real spawn boundary, not an ACP/session or live-model claim.
    for (name, script) in [("buzz-agent", "#!/bin/sh\nexit 0\n".to_string()), ("buzz-dev-mcp", "#!/bin/sh\nexit 0\n".to_string()), ("buzz-acp", format!("#!/bin/sh\nprintf '%s\\n' \"$BUZZ_ACP_MODEL\" \"$BUZZ_AGENT_MODEL\" \"$BUZZ_AGENT_PROVIDER\" \"$BUZZ_ACP_REQUIRED_MODEL\" \"$PWD\" \"$BUZZ_ACP_TEAM_INSTRUCTIONS\" \"$BUZZ_ACP_SESSION_POLICY\" \"$BUZZ_ACP_MCP_COMMAND\" \"$BUZZ_ACP_REQUIRE_MODEL\" > '{}'\n", capture.display()))] {
        let path = temp.path().join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let app = tauri::test::mock_builder()
        .manage(crate::app_state::build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let owner = app
        .state::<crate::app_state::AppState>()
        .signing_keys()
        .unwrap()
        .public_key()
        .to_hex();
    let community = "wss://runtime-fixture.example";
    let mut record = record();
    attest(
        &mut record,
        &app.state::<crate::app_state::AppState>()
            .signing_keys()
            .unwrap(),
    );
    record.acp_command = temp.path().join("buzz-acp").display().to_string();
    record.env_vars.insert(
        "OPENAI_COMPAT_API_KEY".into(),
        "fixture-not-a-secret".into(),
    );
    // Resolve the app-scoped host independently of a selected named configuration.
    let base = prepare_for_app(&app.handle().clone(), &record, None, &owner, community);
    // Default may lack a provider: obtain the same host identity used by prepare_for_app.
    drop(base);
    let scope = agents::retention::RetentionScope {
        db_path: agents::retention::scoped_retention_db_path(
            &agents::managed_agents_base_dir(app.handle()).unwrap(),
            community,
            &owner,
        ),
        relay_url: community.into(),
        owner_keys: app
            .state::<crate::app_state::AppState>()
            .signing_keys()
            .unwrap(),
    };
    let host = crate::commands::desktop_stop::local_id(
        &mut agents::retention::open_retention_db(&scope.db_path).unwrap(),
        &scope,
    )
    .unwrap();
    let mut named = config(&host);
    named.workspace = Some(temp.path().display().to_string());
    let named = save(&mut record, &owner, community, named);
    let mut teams = agents::load_teams(app.handle()).unwrap();
    teams[0].instructions = Some("prepared team instructions".into());
    record.team_id = Some(teams[0].id.clone());
    agents::save_teams(app.handle(), &teams).unwrap();
    for (key, value) in [
        ("BUZZ_ACP_MODEL", "wrong-inherited-model"),
        ("BUZZ_ACP_REQUIRE_MODEL", "false"),
        ("BUZZ_ACP_REQUIRED_MODEL", "wrong-inherited-model"),
        ("BUZZ_ACP_MCP_COMMAND", "wrong-inherited-tool"),
    ] {
        record.env_vars.insert(key.into(), value.into());
    }
    let mut plan = prepare_for_app(
        app.handle(),
        &record,
        Some(&named.reference()),
        &owner,
        community,
    )
    .unwrap();
    assert!(plan.require_preflight().is_err());
    let unpreflighted = agents::spawn_agent_child_prepared(
        app.handle(),
        &record,
        community,
        true,
        Some(&owner),
        None,
        None,
        Some(&plan),
    )
    .unwrap_err();
    assert!(unpreflighted.contains("has not completed provider preflight"));
    preflight_with(&mut plan, &owner, community, false, |_, _| async { Ok(()) })
        .await
        .unwrap();
    // Team content and session partitioning changed while preflight was awaiting.
    // This launch uses the prepared values; a later preparation sees the edits.
    teams[0].instructions = Some("next launch instructions".into());
    agents::save_teams(app.handle(), &teams).unwrap();
    app.state::<crate::app_state::AppState>()
        .thread_scoped_acp_sessions_enabled()
        .store(true, std::sync::atomic::Ordering::Release);
    assert_eq!(
        selected_reference(&record, Some(&owner), community).unwrap(),
        Some(named.reference())
    );
    assert_eq!(plan.configuration(), Some(named.reference()));
    assert!(plan
        .check_scope(Some(&owner), "wss://other.example")
        .is_err());
    let mut edited = record.clone();
    let mut changed = named.clone();
    changed.model = "other-model".into();
    save(&mut edited, &owner, community, changed);
    assert!(plan.revalidate(&edited, &[], &Default::default()).is_err());
    edited = record.clone();
    edited.private_key_nsec.clear();
    assert!(plan.revalidate(&edited, &[], &Default::default()).is_err());
    edited = record.clone();
    edited.parallelism += 1;
    assert!(plan.revalidate(&edited, &[], &Default::default()).is_err());
    edited = record.clone();
    edited.updated_at = "stopped".into();
    edited.last_stopped_at = Some("stopped".into());
    assert!(plan.revalidate(&edited, &[], &Default::default()).is_ok());
    let safe = serde_json::to_string(&catalog(
        &record,
        &[],
        &Default::default(),
        &host,
        &owner,
        community,
    ))
    .unwrap();
    assert!(!safe.contains("credentialRefs"));
    assert!(!safe.contains("workspace"));
    assert!(!safe.contains("fixture-not-a-secret"));
    let next = prepare_for_app(
        app.handle(),
        &record,
        Some(&named.reference()),
        &owner,
        community,
    )
    .unwrap();
    assert_eq!(
        next.app_inputs,
        Some((
            Some("next launch instructions".into()),
            agents::AcpSessionPolicy::Thread
        ))
    );
    // A cached resolution must not hide a tool removed after preparation.
    let mcp_path = required_mcp_command("buzz-agent").unwrap().unwrap();
    assert_eq!(mcp_path, temp.path().join("buzz-dev-mcp"));
    std::fs::set_permissions(&mcp_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(plan.revalidate(&record, &[], &Default::default()).is_err());
    assert!(
        !catalog(&record, &[], &Default::default(), &host, &owner, community)
            .iter()
            .find(|entry| entry.configuration == Some(named.reference()))
            .unwrap()
            .eligible
    );
    assert!(required_mcp_command("claude-agent-acp").unwrap().is_none());
    std::fs::set_permissions(&mcp_path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let relay = crate::relay::bind_expected_relay_scope(None, community.into()).unwrap();
    let mut runtimes = std::collections::HashMap::new();
    agents::start_managed_agent_process_prepared(
        app.handle(),
        &mut record,
        &mut runtimes,
        Some(&owner),
        &relay,
        None,
        None,
        Some(&plan),
    )
    .unwrap();
    let key = agents::ManagedAgentRuntimeKey::new(&record.pubkey, community).unwrap();
    let mut running = runtimes.remove(&key).unwrap();
    assert!(running.child.wait().unwrap().success());
    assert_eq!(
        running.spawn_config.runtime_configuration,
        Some(named.reference())
    );
    assert_eq!(
        std::fs::read_to_string(capture).unwrap(),
        format!(
            "fixture-model\nfixture-model\nopenai\nfixture-model\n{}\nprepared team instructions\nchannel\n{}\n\n",
            temp.path().display(),
            mcp_path.display()
        )
    );
    assert!(agents::read_all_agent_runtime_receipts(app.handle())
        .iter()
        .any(|(_, receipt)| receipt.runtime_configuration == Some(named.reference())));
    agents::remove_agent_runtime_receipt(app.handle(), &key);
}

#[test]
fn final_model_authority_clears_inheritance_and_preserves_claude_a1() {
    use std::ffi::OsStr;
    for required in [None, Some("exact-wire-model")] {
        let mut command = std::process::Command::new("fixture");
        command.env("BUZZ_ACP_REQUIRE_MODEL", "false");
        command.env("BUZZ_ACP_REQUIRED_MODEL", "wrong-inherited-model");
        command.env("ANTHROPIC_MODEL", "exact-wire-model");
        command.env_remove("BUZZ_ACP_MODEL");
        apply_required_model_env(&mut command, required).unwrap();
        let env = command.get_envs().collect::<BTreeMap<_, _>>();
        assert_eq!(env[OsStr::new("BUZZ_ACP_REQUIRE_MODEL")], None);
        assert_eq!(
            env[OsStr::new("BUZZ_ACP_REQUIRED_MODEL")],
            required.map(OsStr::new)
        );
        assert_eq!(env[OsStr::new("BUZZ_ACP_MODEL")], None);
        assert_eq!(
            env[OsStr::new("ANTHROPIC_MODEL")],
            Some(OsStr::new("exact-wire-model"))
        );
    }
    for model in ["", " "] {
        assert!(
            apply_required_model_env(&mut std::process::Command::new("fixture"), Some(model))
                .is_err()
        );
    }
}

#[test]
fn ordinary_selection_fence_includes_default() {
    let mut record = record();
    let owner = attest(&mut record, &nostr::Keys::generate());
    assert!(check_selection(&record, Some(&owner), "one", None).is_ok());
    let named = save(&mut record, &owner, "one", config("host"));
    assert!(check_selection(&record, Some(&owner), "one", None).is_err());
    assert!(check_selection(&record, Some(&owner), "one", Some(&named.reference())).is_ok());
    assert!(check_selection(&record, Some(&owner), "two", None).is_ok());
    record
        .runtime_configurations
        .replace(
            &owner,
            "one",
            "host",
            RuntimeConfigurations {
                selected: None,
                entries: vec![named.clone()],
            },
        )
        .unwrap();
    assert!(check_selection(&record, Some(&owner), "one", Some(&named.reference())).is_err());
}

#[cfg(unix)]
#[path = "orchestration_tests.rs"]
mod orchestration;

#[cfg(all(unix, not(feature = "system-keyring")))]
mod remote_credentials;
