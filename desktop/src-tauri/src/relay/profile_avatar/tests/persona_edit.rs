use super::*;

// The real edit writes agent keys and schedules nest work. Confine its process,
// key store, app data and background nest directory on every supported platform.
#[test]
fn persona_avatar_edit_transfers_both_assets_and_reports_failure() {
    const CHILD: &str = "BUZZ_PERSONA_AVATAR_TEST_HOME";
    if let Some(home) = std::env::var_os(CHILD) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(edit_in_confined_child(std::path::Path::new(&home)));
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "relay::profile_avatar::tests::persona_edit::persona_avatar_edit_transfers_both_assets_and_reports_failure", "--nocapture"])
        .env(CHILD,home.path()).env("HOME",home.path()).env("XDG_DATA_HOME",home.path())
        .env("APPDATA",home.path()).env("LOCALAPPDATA",home.path())
        .env_remove("BUZZ_PRIVATE_KEY").env_remove("BUZZ_AUTH_TAG").env_remove("BUZZ_RELAY_URL").env_remove("BUZZ_NETWORK_TRACE")
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "child failed: {stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn edit_in_confined_child(home: &std::path::Path) {
    use crate::managed_agents::{
        load_managed_agents, load_personas, save_managed_agents, save_personas,
    };
    use tauri::Manager;
    crate::managed_agents::storage::NO_KEYCHAIN_FOR_TEST
        .store(true, std::sync::atomic::Ordering::SeqCst);
    crate::managed_agents::pin_nest_dir_for_test(home.join("nest"));
    let target = server().await;
    target.state.data.lock().unwrap().enforce_possession = true;
    let external = server().await;
    let owner = Keys::generate();
    let agent = Keys::generate();
    let state = state(&[&target]);
    *state.keys.lock().unwrap() = owner.clone();
    *state.relay_url_override.lock().unwrap() = Some(target.state.base.clone());
    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().identifier = home.join("app-data").to_str().unwrap().to_owned();
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(context)
        .unwrap();
    assert!(crate::managed_agents::managed_agents_base_dir(app.handle())
        .unwrap()
        .starts_with(home));
    let persona: crate::managed_agents::AgentDefinition = serde_json::from_value(serde_json::json!({
        "id":"edited", "display_name":"Edited Agent", "system_prompt":"Be helpful",
        "avatar_url":null,"created_at":"2026-11-06T00:00:00Z", "updated_at":"2026-11-06T00:00:00Z"
    })).unwrap();
    let record: crate::managed_agents::ManagedAgentRecord = serde_json::from_value(serde_json::json!({
        "pubkey":agent.public_key().to_hex(),"private_key_nsec":agent.secret_key().to_secret_hex(),
        "persona_id":"edited","name":"Edited Agent","relay_url":"", "acp_command":"", "agent_command":"",
        "agent_args":[],"mcp_command":"","turn_timeout_seconds":0,"system_prompt":null,
        "created_at":"2026-11-06T00:00:00Z", "updated_at":"2026-11-06T00:00:00Z",
        "last_started_at":null,"last_stopped_at":null,"last_exit_code":null,"last_error":null
    })).unwrap();
    let mut unavailable = record.clone();
    unavailable.pubkey = Keys::generate().public_key().to_hex();
    unavailable.private_key_nsec.clear();
    unavailable.name = "Unavailable key".into();
    save_personas(app.handle(), &[persona]).unwrap();
    save_managed_agents(app.handle(), &[record, unavailable]).unwrap();
    let request = |avatar: &str, name: &str| {
        serde_json::from_value(serde_json::json!({
            "id":"edited","displayName":name,"systemPrompt":"Be helpful","avatarUrl":avatar
        }))
        .unwrap()
    };
    let poster = seed(&target, image(31));
    let animation = seed(&target, image(32));
    let avatar = format!(
        "{poster}#buzz-anim={}",
        percent_encoding::utf8_percent_encode(&animation, percent_encoding::NON_ALPHANUMERIC)
    );
    let before_personas = serde_json::to_value(load_personas(app.handle()).unwrap()).unwrap();
    let before_records = serde_json::to_value(load_managed_agents(app.handle()).unwrap()).unwrap();
    let error = crate::update_persona_with(
        request(&avatar, "Edited Agent"),
        app.handle().clone(),
        |_, _, _, _| Ok(()),
    )
    .await
    .unwrap_err();
    assert!(
        error.contains("Unavailable key") && error.contains("Restore") && error.contains("unlink")
    );
    assert_eq!(
        serde_json::to_value(load_personas(app.handle()).unwrap()).unwrap(),
        before_personas
    );
    assert_eq!(
        serde_json::to_value(load_managed_agents(app.handle()).unwrap()).unwrap(),
        before_records
    );
    assert!(
        target.state.data.lock().unwrap().requests.is_empty(),
        "refusal happens before any transfer"
    );
    let recovered = Keys::generate();
    let mut records = load_managed_agents(app.handle()).unwrap();
    records[1].private_key_nsec = recovered.secret_key().to_secret_hex();
    records[1].pubkey = recovered.public_key().to_hex();
    save_managed_agents(app.handle(), &records).unwrap();
    crate::update_persona_with(
        request(&avatar, "Edited Agent"),
        app.handle().clone(),
        |_, _, _, _| Ok(()),
    )
    .await
    .unwrap();
    assert_eq!(
        load_personas(app.handle())
            .unwrap()
            .into_iter()
            .find(|p| p.id == "edited")
            .unwrap()
            .avatar_url
            .as_deref(),
        Some(avatar.as_str())
    );
    assert_eq!(
        load_managed_agents(app.handle()).unwrap()[0]
            .avatar_url
            .as_deref(),
        Some(avatar.as_str())
    );
    {
        let data = target.state.data.lock().unwrap();
        for (method, signer) in [("read", owner.public_key()), ("upload", agent.public_key())] {
            let requests: Vec<_> = data
                .requests
                .iter()
                .filter(|(verb, _)| verb == method)
                .collect();
            assert_eq!(requests.len(), 4);
            assert!(requests.iter().all(|(_, event)| event.pubkey == signer
                || (method == "upload" && event.pubkey == recovered.public_key())));
        }
        assert!(data
            .profiles
            .iter()
            .any(|event| event.pubkey == agent.public_key()));
    }
    // An unchanged selection can rename the profile with agent authority only.
    app.state::<AppState>()
        .identity_lost
        .store(true, std::sync::atomic::Ordering::Release);
    let mut records = load_managed_agents(app.handle()).unwrap();
    records[1].private_key_nsec.clear();
    save_managed_agents(app.handle(), &records).unwrap();
    target.state.data.lock().unwrap().requests.clear();
    crate::update_persona_with(
        request(&avatar, "Renamed Agent"),
        app.handle().clone(),
        |_, _, _, _| Ok(()),
    )
    .await
    .unwrap();
    assert!(target
        .state
        .data
        .lock()
        .unwrap()
        .requests
        .iter()
        .all(|(_, event)| event.pubkey == agent.public_key()));
    app.state::<AppState>()
        .identity_lost
        .store(false, std::sync::atomic::Ordering::Release);
    let mut records = load_managed_agents(app.handle()).unwrap();
    records[1].private_key_nsec = recovered.secret_key().to_secret_hex();
    save_managed_agents(app.handle(), &records).unwrap();
    let next_avatar = seed(&target, image(33));
    let before_personas = serde_json::to_value(load_personas(app.handle()).unwrap()).unwrap();
    let before_records = serde_json::to_value(load_managed_agents(app.handle()).unwrap()).unwrap();
    target.state.data.lock().unwrap().fail_upload = true;
    let error = crate::update_persona_with(
        request(&next_avatar, "Renamed Agent"),
        app.handle().clone(),
        |_, _, _, _| Ok(()),
    )
    .await
    .unwrap_err();
    assert!(error.contains("changes were not saved") && error.contains("503"));
    assert_eq!(
        serde_json::to_value(load_personas(app.handle()).unwrap()).unwrap(),
        before_personas
    );
    assert_eq!(
        serde_json::to_value(load_managed_agents(app.handle()).unwrap()).unwrap(),
        before_records
    );
    target.state.data.lock().unwrap().fail_upload = false;
    for switch_workspace in [false, true] {
        let state = app.state::<AppState>();
        *state.keys.lock().unwrap() = owner.clone();
        *state.relay_url_override.lock().unwrap() = Some(target.state.base.clone());
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        target.state.data.lock().unwrap().pause_read = Some((entered.clone(), release.clone()));
        let edit = crate::update_persona_with(
            request(&next_avatar, "Renamed Agent"),
            app.handle().clone(),
            |_, _, _, _| Ok(()),
        );
        tokio::pin!(edit);
        tokio::select! { _=entered.notified()=>{}, result=&mut edit=>panic!("edit completed before transfer pause: {result:?}") }
        if switch_workspace {
            *state.relay_url_override.lock().unwrap() = Some(external.state.base.clone());
            *state.keys.lock().unwrap() = Keys::generate();
        } else {
            let mut records = load_managed_agents(app.handle()).unwrap();
            records[0].name = "Concurrent edit".into();
            records[0].avatar_url = Some("https://example.org/concurrent.png".into());
            save_managed_agents(app.handle(), &records).unwrap();
        }
        target.state.data.lock().unwrap().pause_read = None;
        release.notify_one();
        assert!(edit
            .await
            .unwrap_err()
            .contains("changed during avatar transfer"));
        assert_eq!(
            serde_json::to_value(load_personas(app.handle()).unwrap()).unwrap(),
            before_personas
        );
        assert_eq!(
            load_managed_agents(app.handle()).unwrap()[0].name,
            "Concurrent edit"
        );
        assert!(external.state.data.lock().unwrap().requests.is_empty());
    }
    // A workspace switch in the last synchronous retention window must not
    // retarget the captured enqueue authority.
    let state = app.state::<AppState>();
    *state.keys.lock().unwrap() = owner.clone();
    *state.relay_url_override.lock().unwrap() = Some(target.state.base.clone());
    let expected_relay = target.state.base.clone();
    let (_, retained_scope) = crate::update_persona_with(
        request(&avatar, "Scoped Rename"),
        app.handle().clone(),
        move |_, state, _, scope| {
            *state.relay_url_override.lock().unwrap() = Some("https://switched.example".into());
            *state.keys.lock().unwrap() = Keys::generate();
            let scope = scope.unwrap();
            assert_eq!(scope.relay_url, expected_relay);
            Ok(scope.clone())
        },
    )
    .await
    .unwrap();
    assert_eq!(retained_scope.owner_keys.public_key(), owner.public_key());
    let state = app.state::<AppState>();
    *state.keys.lock().unwrap() = owner;
    *state.relay_url_override.lock().unwrap() = Some(target.state.base.clone());
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    target.state.data.lock().unwrap().pause_read = Some((entered.clone(), release.clone()));
    let edit = crate::update_persona_with(
        request(&next_avatar, "Runtime-safe edit"),
        app.handle().clone(),
        |_, _, _, _| Ok(()),
    );
    tokio::pin!(edit);
    tokio::select! { _=entered.notified()=>{}, result=&mut edit=>panic!("edit completed before pause: {result:?}") }
    let mut records = load_managed_agents(app.handle()).unwrap();
    records[0].runtime_pid = Some(12345);
    records[1].last_started_at = Some("2026-11-06T20:00:00Z".into());
    save_managed_agents(app.handle(), &records).unwrap();
    target.state.data.lock().unwrap().pause_read = None;
    release.notify_one();
    edit.await.unwrap();
    assert_eq!(
        load_managed_agents(app.handle()).unwrap()[0].runtime_pid,
        Some(12345)
    );
}
