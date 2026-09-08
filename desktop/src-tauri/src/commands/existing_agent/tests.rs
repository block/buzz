use super::*;
use nostr::{EventBuilder, Kind, Tag};

fn fixture() -> (AddExistingAgentRequest, Vec<Event>, Vec<Event>) {
    let owner = Keys::parse(&"01".repeat(32)).unwrap();
    let agent = Keys::parse(&"02".repeat(32)).unwrap();
    let auth = buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "").unwrap();
    let profile = EventBuilder::new(
        Kind::Metadata,
        r#"{"name":"Exact agent","picture":"https://fixture.example/avatar.png"}"#,
    )
    .tags([buzz_sdk_pkg::nip_oa::parse_auth_tag(&auth).unwrap()])
    .sign_with_keys(&agent)
    .unwrap();
    let input = AddExistingAgentRequest {
        owner: owner.public_key().to_hex(),
        community: "wss://fixture.example".into(),
        pubkey: agent.public_key().to_hex(),
        private_key: agent.secret_key().to_bech32().unwrap(),
    };
    let policy = EventBuilder::new(
        Kind::Custom(30177),
        serde_json::json!({
            "name": "Exact agent", "persona_id": "existing-persona", "parallelism": 1,
            "respond_to": "owner-only"
        })
        .to_string(),
    )
    .tags([Tag::parse(["d", &input.pubkey]).unwrap()])
    .sign_with_keys(&owner)
    .unwrap();
    (input, vec![profile], vec![policy])
}

#[test]
fn exact_key_and_persona_no_autostart() {
    let (input, profiles, policies) = fixture();
    let record = verified_record(&input, &profiles, &policies).unwrap();
    assert_eq!(record.pubkey, input.pubkey);
    assert_eq!(
        record.avatar_url.as_deref(),
        Some("https://fixture.example/avatar.png")
    );
    assert_eq!(record.persona_id.as_deref(), Some("existing-persona"));
    assert_eq!(record.private_key_nsec, input.private_key);
    assert_eq!(record.relay_url, input.community);
    assert!(!record.start_on_app_launch);
    assert!(!record.auto_restart_on_config_change);
    assert!(record.runtime_pid.is_none());
}

#[test]
fn wrong_key_foreign_owner_and_unsigned_policy_are_refused() {
    let (mut input, profiles, mut policies) = fixture();
    input.private_key = "03".repeat(32);
    assert!(verified_record(&input, &profiles, &policies).is_err());
    let (mut input, _, _) = fixture();
    input.owner = Keys::parse(&"03".repeat(32)).unwrap().public_key().to_hex();
    assert!(verified_record(&input, &profiles, &policies).is_err());
    let (input, _, _) = fixture();
    policies[0].content.push(' ');
    assert!(verified_record(&input, &profiles, &policies).is_err());
    assert!(verified_record(&input, &[], &policies).is_err());
}

#[test]
fn conflicting_oa_and_duplicate_coordinates_are_refused() {
    let (input, profiles, policies) = fixture();
    let agent = Keys::parse(&input.private_key).unwrap();
    let other = Keys::parse(&"03".repeat(32)).unwrap();
    let auth = buzz_sdk_pkg::nip_oa::compute_auth_tag(&other, &agent.public_key(), "").unwrap();
    let mut tags = profiles[0].tags.clone().to_vec();
    tags.push(buzz_sdk_pkg::nip_oa::parse_auth_tag(&auth).unwrap());
    let profile = EventBuilder::new(Kind::Metadata, "{}")
        .tags(tags)
        .sign_with_keys(&agent)
        .unwrap();
    assert!(verified_record(&input, &[profile], &policies).is_err());
    let owner = Keys::parse(&"01".repeat(32)).unwrap();
    let policy = EventBuilder::new(Kind::Custom(30177), &policies[0].content)
        .tags([
            Tag::parse(["d", &input.pubkey]).unwrap(),
            Tag::parse(["d", &input.pubkey]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    assert!(verified_record(&input, &profiles, &[policy]).is_err());
}

/// Compute a NIP-OA attestation exactly as the production commit does.
#[cfg(all(unix, not(feature = "system-keyring")))]
fn owner_tag(owner: &Keys, agent_pubkey: &str) -> String {
    buzz_sdk_pkg::nip_oa::compute_auth_tag(
        owner,
        &nostr::PublicKey::from_hex(agent_pubkey).unwrap(),
        "",
    )
    .unwrap()
}

// Never run filesystem/credential fixtures against the real system keyring.
#[cfg(all(unix, not(feature = "system-keyring")))]
struct TempHomeEnv(Vec<(&'static str, Option<std::ffi::OsString>)>);

#[cfg(all(unix, not(feature = "system-keyring")))]
impl Drop for TempHomeEnv {
    fn drop(&mut self) {
        for (name, value) in &self.0 {
            match value {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
        }
    }
}

/// Point HOME/XDG_DATA_HOME at `temp` and restore them on drop. Pair with
/// `lock_path_mutex` so parallel env-mutating tests stay exclusive.
#[cfg(all(unix, not(feature = "system-keyring")))]
fn isolate_agent_home(temp: &std::path::Path) -> TempHomeEnv {
    TempHomeEnv(
        ["HOME", "XDG_DATA_HOME"]
            .into_iter()
            .map(|name| {
                let old = std::env::var_os(name);
                std::env::set_var(name, temp);
                (name, old)
            })
            .collect(),
    )
}

// Never run filesystem/credential fixtures against the real system keyring.
#[cfg(all(unix, not(feature = "system-keyring")))]
#[test]
fn explicit_file_commit_repair_and_ordinary_save_f6() {
    use agents::storage;
    let _guard = agents::lock_path_mutex();
    let temp = tempfile::tempdir().unwrap();
    let _env = isolate_agent_home(temp.path());
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let (input, profiles, policies) = fixture();
    let mut record = verified_record(&input, &profiles, &policies).unwrap();
    assert!(storage::save_managed_agents(app.handle(), std::slice::from_ref(&record)).is_err());
    // Seed the store exactly as the production commit leaves it: record,
    // credential, and the current owner's attestation in one restricted write.
    record.auth_tag = Some(owner_tag(
        &Keys::parse(&"01".repeat(32)).unwrap(),
        &input.pubkey,
    ));
    storage::import_existing_agent_key(app.handle(), record.clone()).unwrap();
    let path = agents::managed_agents_store_path(app.handle()).unwrap();
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = Keys::parse(&"01".repeat(32)).unwrap();
    *state.relay_url_override.lock().unwrap() = Some(input.community.clone());
    let mut persona = agents::load_personas(app.handle()).unwrap().remove(0);
    persona.id = "existing-persona".into();
    persona.is_active = true;
    agents::save_personas(app.handle(), &[persona]).unwrap();
    let original = std::fs::read(&path).unwrap();
    // Bind duplicate and post-await scope fences to the production commit seam.
    commit_verified(app.handle(), &state, &input, record.clone()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), original);
    *state.relay_url_override.lock().unwrap() = Some("wss://changed.example".into());
    assert!(commit_verified(app.handle(), &state, &input, record.clone()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    *state.relay_url_override.lock().unwrap() = Some(input.community.clone());
    *state.keys.lock().unwrap() = Keys::parse(&"03".repeat(32)).unwrap();
    assert!(commit_verified(app.handle(), &state, &input, record.clone()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    *state.keys.lock().unwrap() = Keys::parse(&"01".repeat(32)).unwrap();
    let mut wrong = record.clone();
    wrong.persona_id = Some("wrong".into());
    assert!(storage::import_existing_agent_key(app.handle(), wrong).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let mut raw = storage::load_agent_store(app.handle()).unwrap();
    let saved = raw.iter_mut().find(|r| r.pubkey == input.pubkey).unwrap();
    saved.private_key_nsec.clear();
    saved.relay_url.clear(); // Legacy records inherit the active community.
    saved.runtime_pid = Some(42);
    saved.agent_command = "preserved".into();
    std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
    storage::save_managed_agents(app.handle(), std::slice::from_ref(&record)).unwrap();
    assert!(storage::load_managed_agents(app.handle()).unwrap()[0]
        .private_key_nsec
        .is_empty());
    // Restore the synthetic running/config state before explicit key-only repair.
    std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
    commit_verified(app.handle(), &state, &input, record.clone()).unwrap();
    let repaired = storage::load_managed_agents(app.handle())
        .unwrap()
        .remove(0);
    assert_eq!(repaired.private_key_nsec, input.private_key);
    assert_eq!(repaired.runtime_pid, Some(42));
    assert_eq!(repaired.agent_command, "preserved");
    // Fail during atomic-file preparation, AFTER all admission and reads.
    // Credential and config bytes remain unchanged, then exact retry succeeds.
    let before = std::fs::read(&path).unwrap();
    let parent = path.parent().unwrap();
    let permissions = std::fs::metadata(parent).unwrap().permissions();
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o500)).unwrap();
    let failed = storage::import_existing_agent_key(app.handle(), record.clone());
    std::fs::set_permissions(parent, permissions).unwrap();
    assert!(failed.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    storage::import_existing_agent_key(app.handle(), record).unwrap();
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

// The saved owner link gates BOTH commit outcomes: a legacy record whose saved
// attestation is absent, foreign, or invalid must be refused with zero writes,
// whatever the key state — never a healthy duplicate or repaired key for a
// record that later refuses to run (runtime_configurations::verify_owner).
#[cfg(all(unix, not(feature = "system-keyring")))]
#[test]
fn saved_owner_link_gates_duplicate_and_repair() {
    use agents::storage;
    let _guard = agents::lock_path_mutex();
    let temp = tempfile::tempdir().unwrap();
    let _env = isolate_agent_home(temp.path());
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let (input, profiles, policies) = fixture();
    let record = verified_record(&input, &profiles, &policies).unwrap();
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = Keys::parse(&"01".repeat(32)).unwrap();
    *state.relay_url_override.lock().unwrap() = Some(input.community.clone());
    let mut persona = agents::load_personas(app.handle()).unwrap().remove(0);
    persona.id = "existing-persona".into();
    persona.is_active = true;
    agents::save_personas(app.handle(), &[persona]).unwrap();
    // Production creation commit: record + credential + owner tag in one write.
    commit_verified(app.handle(), &state, &input, record.clone()).unwrap();
    let path = agents::managed_agents_store_path(app.handle()).unwrap();
    let created = std::fs::read(&path).unwrap();
    // Positive controls first: this exact fixture must admit both outcomes the
    // guard protects, so every refusal below varies only the saved owner link.
    commit_verified(app.handle(), &state, &input, record.clone()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), created); // Healthy duplicate.
    let mut raw = storage::load_agent_store(app.handle()).unwrap();
    raw.iter_mut()
        .find(|r| r.pubkey == input.pubkey)
        .unwrap()
        .private_key_nsec
        .clear();
    std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
    commit_verified(app.handle(), &state, &input, record.clone()).unwrap(); // Key repair.
    assert_eq!(
        storage::load_agent_store(app.handle())
            .unwrap()
            .into_iter()
            .find(|r| r.pubkey == input.pubkey)
            .unwrap()
            .private_key_nsec,
        input.private_key
    );
    let foreign = owner_tag(&Keys::parse(&"03".repeat(32)).unwrap(), &input.pubkey);
    // Structurally valid, current owner embedded, but signed over a different
    // agent's preimage — only signature verification rejects this one.
    let invalid = owner_tag(
        &Keys::parse(&"01".repeat(32)).unwrap(),
        &Keys::parse(&"04".repeat(32)).unwrap().public_key().to_hex(),
    );
    for (name, tag, healthy_key) in [
        ("absent attestation, healthy key", None, true),
        ("absent attestation, missing key", None, false),
        ("foreign attestation, healthy key", Some(&foreign), true),
        ("foreign attestation, missing key", Some(&foreign), false),
        ("invalid attestation, healthy key", Some(&invalid), true),
        ("invalid attestation, missing key", Some(&invalid), false),
    ] {
        let mut raw = storage::load_agent_store(app.handle()).unwrap();
        let saved = raw.iter_mut().find(|r| r.pubkey == input.pubkey).unwrap();
        saved.auth_tag = tag.cloned();
        saved.private_key_nsec = if healthy_key {
            input.private_key.clone()
        } else {
            String::new()
        };
        std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        let before = std::fs::read(&path).unwrap();
        let error = commit_verified(app.handle(), &state, &input, record.clone()).expect_err(name);
        assert_eq!(
            error, "Saved agent ownership is missing, invalid, or belongs to a different identity",
            "{name}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), before, "{name}: no writes");
        let persisted = storage::load_agent_store(app.handle())
            .unwrap()
            .into_iter()
            .find(|r| r.pubkey == input.pubkey)
            .unwrap();
        assert_eq!(
            persisted.private_key_nsec.is_empty(),
            !healthy_key,
            "{name}: key state unchanged"
        );
    }
}
