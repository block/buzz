use super::*;
use crate::managed_agents::retention::{get_pending_sync, get_retained_event, mark_synced};
use nostr::ToBech32;
use std::collections::BTreeMap;
use tempfile::TempDir;

fn sample_record(pubkey: &str, name: &str) -> ManagedAgentRecord {
    serde_json::from_str(&format!(
        r#"{{
            "pubkey": "{pubkey}",
            "name": "{name}",
            "relay_url": "wss://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": "You are a test agent.",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }}"#
    ))
    .unwrap()
}

fn write_store(dir: &TempDir, records: &[ManagedAgentRecord]) {
    std::fs::write(
        dir.path().join("managed-agents.json"),
        serde_json::to_vec_pretty(records).unwrap(),
    )
    .unwrap();
}

#[test]
fn private_config_conversion_encrypts_secrets_and_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    let mut record = sample_record(&pubkey, "private-agent");
    record.private_key_nsec = agent_keys.secret_key().to_bech32().unwrap();
    record.env_vars = BTreeMap::from([("API_TOKEN".to_string(), "very-secret".to_string())]);
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();

    assert!(retain_agent_record(&conn, &owner_keys, &record).unwrap());
    let row = get_retained_event(
        &conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &owner_keys.public_key().to_hex(),
        &pubkey,
    )
    .unwrap()
    .unwrap();
    assert!(!row.raw_event.contains("very-secret"));
    assert!(!row.raw_event.contains("nsec1"));

    let event = nostr::Event::from_json(&row.raw_event).unwrap();
    let (_, payload) = private_managed_agent::validate_and_decrypt(&event, &owner_keys).unwrap();
    assert_eq!(payload.config.name, "private-agent");
    assert_eq!(payload.config.env_vars["API_TOKEN"], "very-secret");
    assert_eq!(payload.generation, 1);
    assert_eq!(payload.previous_event_id, None);

    mark_synced(
        &conn,
        row.kind,
        &row.pubkey,
        &row.d_tag,
        row.created_at,
        &row.content,
    )
    .unwrap();
    assert!(!retain_agent_record(&conn, &owner_keys, &record).unwrap());
    assert!(get_pending_sync(&conn)
        .unwrap()
        .iter()
        .all(|pending| pending.kind != KIND_PRIVATE_MANAGED_AGENT));
}

#[test]
fn private_config_preserves_unknown_fields_without_generation_churn() {
    let dir = TempDir::new().unwrap();
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    let mut record = sample_record(&pubkey, "private-agent");
    record.private_key_nsec = agent_keys.secret_key().to_bech32().unwrap();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();

    let mut newer_payload =
        private_payload_from_record(&record, &owner_keys.public_key().to_hex(), 1, None).unwrap();
    newer_payload.extensions.insert(
        "future.example:feature".into(),
        serde_json::json!({"enabled": true}),
    );
    newer_payload
        .extra
        .insert("future_top_level".into(), serde_json::json!([1, 2, 3]));
    newer_payload
        .config
        .extra
        .insert("future_config".into(), serde_json::json!({"mode": "new"}));
    let first_event = private_managed_agent::build_event(&owner_keys, &newer_payload, 1).unwrap();
    retain_event(
        &conn,
        &RetainedEvent {
            kind: KIND_PRIVATE_MANAGED_AGENT,
            pubkey: owner_keys.public_key().to_hex(),
            d_tag: pubkey.clone(),
            content: first_event.content.clone(),
            created_at: first_event.created_at.as_secs() as i64,
            raw_event: first_event.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();

    record.system_prompt = Some("edited by an older client".into());
    assert!(retain_agent_record(&conn, &owner_keys, &record).unwrap());
    let row = get_retained_event(
        &conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &owner_keys.public_key().to_hex(),
        &pubkey,
    )
    .unwrap()
    .unwrap();
    let rebuilt_event = nostr::Event::from_json(&row.raw_event).unwrap();
    let (_, rebuilt) =
        private_managed_agent::validate_and_decrypt(&rebuilt_event, &owner_keys).unwrap();
    assert_eq!(rebuilt.generation, 2);
    assert_eq!(rebuilt.previous_event_id, Some(first_event.id.to_hex()));
    assert_eq!(rebuilt.extensions, newer_payload.extensions);
    assert_eq!(rebuilt.extra, newer_payload.extra);
    assert_eq!(rebuilt.config.extra, newer_payload.config.extra);

    assert!(!retain_agent_record(&conn, &owner_keys, &record).unwrap());
    let unchanged = get_retained_event(
        &conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &owner_keys.public_key().to_hex(),
        &pubkey,
    )
    .unwrap()
    .unwrap();
    assert_eq!(unchanged.raw_event, row.raw_event);
}

#[test]
fn private_config_change_advances_generation_and_links_previous_event() {
    let dir = TempDir::new().unwrap();
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    let mut record = sample_record(&pubkey, "private-agent");
    record.private_key_nsec = agent_keys.secret_key().to_bech32().unwrap();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();

    retain_agent_record(&conn, &owner_keys, &record).unwrap();
    let first = get_retained_event(
        &conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &owner_keys.public_key().to_hex(),
        &pubkey,
    )
    .unwrap()
    .unwrap();
    let first_event = nostr::Event::from_json(&first.raw_event).unwrap();

    record.env_vars.insert("TOKEN".into(), "rotated".into());
    retain_agent_record(&conn, &owner_keys, &record).unwrap();
    let second = get_retained_event(
        &conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &owner_keys.public_key().to_hex(),
        &pubkey,
    )
    .unwrap()
    .unwrap();
    let second_event = nostr::Event::from_json(&second.raw_event).unwrap();
    let (_, payload) =
        private_managed_agent::validate_and_decrypt(&second_event, &owner_keys).unwrap();
    assert_eq!(payload.generation, 2);
    assert_eq!(payload.previous_event_id, Some(first_event.id.to_hex()));
}

#[test]
fn missing_store_is_noop() {
    let dir = TempDir::new().unwrap();
    let keys = nostr::Keys::generate();
    assert_eq!(reconcile_agents_in_dir(dir.path(), &keys).unwrap(), 0);
}

/// Boot reconcile on a default `system-keyring` build reads the agent nsec
/// from the keyring, not the JSON. Hydration through the [`KeyStore`] seam
/// must fill it in so the first 30179 gets published; without it, the
/// empty-nsec skip fires and only the 30177 lands. The `None`-store control
/// pins the other side: no store, no 30179 — which is also why the plain
/// [`reconcile_agents_in_dir`] test helper can never hit the live OS keyring
/// (a macOS Keychain ACL prompt blocks a headless test binary forever).
#[test]
fn keyring_resident_nsec_is_hydrated_for_private_config() {
    use crate::managed_agents::storage::{agent_keyring_name, tests::FakeKeyStore};
    use nostr::ToBech32;

    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    let nsec = agent_keys.secret_key().to_bech32().unwrap();
    // JSON carries an empty nsec — keyring-resident, as on a default build.
    let record = sample_record(&pubkey, "keyring-agent");
    assert!(record.private_key_nsec.is_empty());

    // Control: no key store → empty-nsec skip → no 30179 head.
    let dir = TempDir::new().unwrap();
    write_store(&dir, std::slice::from_ref(&record));
    let db_path = dir.path().join("retention.db");
    reconcile_agents_in_dir_with(
        dir.path(),
        &owner_keys,
        &db_path,
        None::<&crate::secret_store::SecretStore>,
    )
    .unwrap();
    let conn = open_retention_db(&db_path).unwrap();
    assert!(get_retained_event(
        &conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &owner_keys.public_key().to_hex(),
        &pubkey,
    )
    .unwrap()
    .is_none());
    drop(conn);

    // With the key in the (fake) keyring, hydration fills the nsec and the
    // first 30179 is retained — and decrypts back to that exact key.
    let dir = TempDir::new().unwrap();
    write_store(&dir, &[record]);
    let db_path = dir.path().join("retention.db");
    let store = FakeKeyStore::reachable().with_key(&agent_keyring_name(&pubkey), &nsec);
    reconcile_agents_in_dir_with(dir.path(), &owner_keys, &db_path, Some(&store)).unwrap();
    let conn = open_retention_db(&db_path).unwrap();
    let row = get_retained_event(
        &conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &owner_keys.public_key().to_hex(),
        &pubkey,
    )
    .unwrap()
    .expect("hydrated nsec must publish the first 30179");
    let event = nostr::Event::from_json(&row.raw_event).unwrap();
    let (_, payload) = private_managed_agent::validate_and_decrypt(&event, &owner_keys).unwrap();
    assert_eq!(payload.identity.private_key_nsec, nsec);
}

/// A record whose legacy `relay_url` pin is empty (legal since #2122 — the
/// pin is ignored at read time) must not stop the boot reconcile: the records
/// after it in file order still get their first 30179.
#[test]
fn empty_relay_url_pin_does_not_abort_boot_reconcile() {
    use nostr::ToBech32;

    let owner_keys = nostr::Keys::generate();
    let pinless_keys = nostr::Keys::generate();
    let later_keys = nostr::Keys::generate();
    let mut pinless = sample_record(&pinless_keys.public_key().to_hex(), "pinless");
    pinless.relay_url.clear();
    pinless.private_key_nsec = pinless_keys.secret_key().to_bech32().unwrap();
    let mut later = sample_record(&later_keys.public_key().to_hex(), "later");
    later.private_key_nsec = later_keys.secret_key().to_bech32().unwrap();

    let dir = TempDir::new().unwrap();
    write_store(&dir, &[pinless, later]);
    let db_path = dir.path().join("retention.db");
    reconcile_agents_in_dir_with(
        dir.path(),
        &owner_keys,
        &db_path,
        None::<&crate::secret_store::SecretStore>,
    )
    .unwrap();

    let conn = open_retention_db(&db_path).unwrap();
    let owner = owner_keys.public_key().to_hex();
    for keys in [&pinless_keys, &later_keys] {
        let agent = keys.public_key().to_hex();
        assert!(
            get_retained_event(&conn, KIND_MANAGED_AGENT, &owner, &agent)
                .unwrap()
                .is_some(),
            "30177 for {agent}"
        );
        let row = get_retained_event(&conn, KIND_PRIVATE_MANAGED_AGENT, &owner, &agent)
            .unwrap()
            .unwrap_or_else(|| panic!("first 30179 for {agent}"));
        let event = nostr::Event::from_json(&row.raw_event).unwrap();
        private_managed_agent::validate_and_decrypt(&event, &owner_keys).unwrap();
    }
}

/// A record that genuinely cannot be published (here: an nsec that does not
/// derive its pubkey) is logged and skipped; the records before and after it
/// in file order still get both heads, and the bad coordinate gets neither.
#[test]
fn unpublishable_record_does_not_abort_boot_reconcile() {
    use nostr::ToBech32;

    let owner_keys = nostr::Keys::generate();
    let first_keys = nostr::Keys::generate();
    let bad_keys = nostr::Keys::generate();
    let last_keys = nostr::Keys::generate();
    let mut first = sample_record(&first_keys.public_key().to_hex(), "first");
    first.private_key_nsec = first_keys.secret_key().to_bech32().unwrap();
    let mut bad = sample_record(&bad_keys.public_key().to_hex(), "bad");
    bad.private_key_nsec = nostr::Keys::generate().secret_key().to_bech32().unwrap();
    let mut last = sample_record(&last_keys.public_key().to_hex(), "last");
    last.private_key_nsec = last_keys.secret_key().to_bech32().unwrap();

    let dir = TempDir::new().unwrap();
    write_store(&dir, &[first, bad, last]);
    let db_path = dir.path().join("retention.db");
    let reconciled = reconcile_agents_in_dir_with(
        dir.path(),
        &owner_keys,
        &db_path,
        None::<&crate::secret_store::SecretStore>,
    )
    .unwrap();
    assert_eq!(reconciled, 2);

    let conn = open_retention_db(&db_path).unwrap();
    let owner = owner_keys.public_key().to_hex();
    for keys in [&first_keys, &last_keys] {
        let agent = keys.public_key().to_hex();
        for kind in [KIND_MANAGED_AGENT, KIND_PRIVATE_MANAGED_AGENT] {
            assert!(
                get_retained_event(&conn, kind, &owner, &agent)
                    .unwrap()
                    .is_some(),
                "{kind} for {agent}"
            );
        }
    }
    let bad_agent = bad_keys.public_key().to_hex();
    for kind in [KIND_MANAGED_AGENT, KIND_PRIVATE_MANAGED_AGENT] {
        assert!(
            get_retained_event(&conn, kind, &owner, &bad_agent)
                .unwrap()
                .is_none(),
            "{kind} must not be retained for the unpublishable record"
        );
    }
}

#[test]
fn fresh_record_is_retained_pending() {
    let dir = TempDir::new().unwrap();
    let keys = nostr::Keys::generate();
    write_store(&dir, &[sample_record("a".repeat(64).as_str(), "agent-one")]);

    assert_eq!(reconcile_agents_in_dir(dir.path(), &keys).unwrap(), 1);

    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    let pending = get_pending_sync(&conn).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].kind, KIND_MANAGED_AGENT);
    assert_eq!(pending[0].d_tag, "a".repeat(64));
    // The retained content is the opt-IN projection — never secrets.
    assert!(!pending[0].raw_event.contains("nsec"));
}

#[test]
fn unchanged_record_does_not_churn_pending_sync() {
    let dir = TempDir::new().unwrap();
    let keys = nostr::Keys::generate();
    write_store(&dir, &[sample_record("b".repeat(64).as_str(), "agent-two")]);

    assert_eq!(reconcile_agents_in_dir(dir.path(), &keys).unwrap(), 1);

    // Simulate the flush loop confirming the publish.
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    let row = get_retained_event(
        &conn,
        KIND_MANAGED_AGENT,
        &keys.public_key().to_hex(),
        &"b".repeat(64),
    )
    .unwrap()
    .unwrap();
    mark_synced(
        &conn,
        row.kind,
        &row.pubkey,
        &row.d_tag,
        row.created_at,
        &row.content,
    )
    .unwrap();
    drop(conn);

    // Second boot with identical disk state: no re-retain, no pending churn.
    assert_eq!(reconcile_agents_in_dir(dir.path(), &keys).unwrap(), 0);
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    assert!(get_pending_sync(&conn).unwrap().is_empty());
}

#[test]
fn edited_record_is_republished() {
    let dir = TempDir::new().unwrap();
    let keys = nostr::Keys::generate();
    let mut record = sample_record("c".repeat(64).as_str(), "agent-three");
    write_store(&dir, &[record.clone()]);
    assert_eq!(reconcile_agents_in_dir(dir.path(), &keys).unwrap(), 1);

    // Hand-edit a published field between launches.
    record.system_prompt = Some("You are an edited agent.".to_string());
    write_store(&dir, &[record]);

    assert_eq!(reconcile_agents_in_dir(dir.path(), &keys).unwrap(), 1);
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    let row = get_retained_event(
        &conn,
        KIND_MANAGED_AGENT,
        &keys.public_key().to_hex(),
        &"c".repeat(64),
    )
    .unwrap()
    .unwrap();
    assert!(row.content.contains("edited agent"));
    assert!(row.pending_sync);
}

#[test]
fn excluded_field_edit_is_noop() {
    let dir = TempDir::new().unwrap();
    let keys = nostr::Keys::generate();
    let mut record = sample_record("d".repeat(64).as_str(), "agent-four");
    write_store(&dir, &[record.clone()]);
    assert_eq!(reconcile_agents_in_dir(dir.path(), &keys).unwrap(), 1);

    // env_vars is excluded from the projection — editing it must not republish.
    record.env_vars = BTreeMap::from([("SOME_KEY".to_string(), "value".to_string())]);
    write_store(&dir, &[record]);

    assert_eq!(reconcile_agents_in_dir(dir.path(), &keys).unwrap(), 0);
}

#[test]
fn missing_record_is_never_tombstoned() {
    let dir = TempDir::new().unwrap();
    let keys = nostr::Keys::generate();
    let one = sample_record("e".repeat(64).as_str(), "agent-five");
    let two = sample_record("f".repeat(64).as_str(), "agent-six");
    write_store(&dir, &[one.clone(), two]);
    assert_eq!(reconcile_agents_in_dir(dir.path(), &keys).unwrap(), 2);

    // A truncated store (one of two records) must leave the missing record's
    // retained row untouched — absence never tombstones.
    write_store(&dir, &[one]);
    assert_eq!(reconcile_agents_in_dir(dir.path(), &keys).unwrap(), 0);

    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    let survivor = get_retained_event(
        &conn,
        KIND_MANAGED_AGENT,
        &keys.public_key().to_hex(),
        &"f".repeat(64),
    )
    .unwrap();
    assert!(survivor.is_some(), "missing record must stay retained");
}

#[test]
fn keyless_record_is_skipped() {
    let dir = TempDir::new().unwrap();
    let keys = nostr::Keys::generate();
    write_store(&dir, &[sample_record("", "keyless-agent")]);
    assert_eq!(reconcile_agents_in_dir(dir.path(), &keys).unwrap(), 0);
}

#[test]
fn malformed_store_errors_and_preserves_invalid_backup() {
    let dir = TempDir::new().unwrap();
    let keys = nostr::Keys::generate();
    let store_path = dir.path().join("managed-agents.json");
    std::fs::write(&store_path, b"[{ this is not json").unwrap();

    let err = reconcile_agents_in_dir(dir.path(), &keys).unwrap_err();
    assert!(err.contains("failed to parse"), "unexpected error: {err}");

    let backup = dir.path().join("managed-agents.json.invalid");
    assert!(backup.exists(), "malformed store must be preserved");
    assert_eq!(
        std::fs::read(&backup).unwrap(),
        b"[{ this is not json".to_vec()
    );
    // Original stays in place so the next boot fails loudly again.
    assert!(store_path.exists());
}

#[test]
fn monotonic_bump_supersedes_future_dated_head() {
    let dir = TempDir::new().unwrap();
    let keys = nostr::Keys::generate();
    let mut record = sample_record("1".repeat(64).as_str(), "agent-seven");
    write_store(&dir, &[record.clone()]);
    assert_eq!(reconcile_agents_in_dir(dir.path(), &keys).unwrap(), 1);

    // Future-date the retained head (clock skew / interactive same-second bump).
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    let owner = keys.public_key().to_hex();
    let head = get_retained_event(&conn, KIND_MANAGED_AGENT, &owner, &"1".repeat(64))
        .unwrap()
        .unwrap();
    let future = RetainedEvent {
        created_at: head.created_at + 3600,
        ..head
    };
    crate::managed_agents::retention::retain_event(&conn, &future).unwrap();
    drop(conn);

    record.system_prompt = Some("New prompt after skew.".to_string());
    write_store(&dir, &[record]);

    // The changed body must land despite the future-dated head.
    assert_eq!(reconcile_agents_in_dir(dir.path(), &keys).unwrap(), 1);
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    let row = get_retained_event(&conn, KIND_MANAGED_AGENT, &owner, &"1".repeat(64))
        .unwrap()
        .unwrap();
    assert!(row.content.contains("New prompt after skew"));
}

/// The slimming transition: a definition-linked record whose retained row
/// holds the legacy fat projection republishes ONCE (the slimmed shape), and
/// the second boot is a true no-op — the republish wave is one-time.
#[test]
fn slimming_republish_wave_is_one_time() {
    let dir = TempDir::new().unwrap();
    let keys = nostr::Keys::generate();
    let mut record = sample_record("e".repeat(64).as_str(), "agent-five");
    record.persona_id = Some("persona-1".to_string());
    record.persona_source_version = Some("abc123".to_string());
    write_store(&dir, &[record]);

    // Seed a SYNCED legacy-fat retained row — the pre-upgrade state — so the
    // first-boot republish below is distinctly the fat→slim content change,
    // not the ordinary fresh-record retain.
    let fat_content = serde_json::json!({
        "name": "agent-five",
        "persona_id": "persona-1",
        "system_prompt": "You are a test agent.",
        "persona_source_version": "abc123",
        "parallelism": 1,
        "respond_to": "owner-only"
    })
    .to_string();
    {
        let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_MANAGED_AGENT,
                pubkey: keys.public_key().to_hex(),
                d_tag: "e".repeat(64),
                content: fat_content,
                created_at: 1,
                raw_event: String::new(),
                pending_sync: false,
            },
        )
        .unwrap();
    }

    // First boot after upgrade: projection content changed (fat -> slim) so
    // the agent republishes.
    assert_eq!(reconcile_agents_in_dir(dir.path(), &keys).unwrap(), 1);
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    let row = get_retained_event(
        &conn,
        KIND_MANAGED_AGENT,
        &keys.public_key().to_hex(),
        &"e".repeat(64),
    )
    .unwrap()
    .unwrap();
    assert!(
        !row.content.contains("system_prompt"),
        "definition-linked retained content must be the slimmed shape"
    );
    assert!(!row.content.contains("\"model\""), "model must be slimmed");
    assert!(
        !row.content.contains("\"provider\""),
        "provider must be slimmed"
    );
    assert!(
        !row.content.contains("persona_source_version"),
        "persona_source_version must be slimmed"
    );
    assert!(
        !row.content.contains("abc123"),
        "source version value must be absent"
    );
    assert!(row.pending_sync, "slimmed rewrite must queue for publish");
    drop(conn);

    // Second boot: identical projection — a true no-op, no republish loop.
    assert_eq!(
        reconcile_agents_in_dir(dir.path(), &keys).unwrap(),
        0,
        "second boot must be a no-op (idempotence)"
    );
}

// ── retain_agent_record (interactive-edit engine) ────────────────────────────
//
// #2423: renaming an agent must re-retain its kind:30177 identity record
// IMMEDIATELY, not at the next boot-time reconcile. These tests pin the shared
// engine both the boot reconcile and the interactive edit paths
// (`retain_managed_agent_pending`, persona-rename propagation) run on.

/// A rename re-retains the identity record under the SAME coordinate (the
/// agent pubkey) with the new name, queued for publish, with a created_at
/// strictly past the retained head so the relay's replaceable-event rule
/// accepts it. Without this, the relay keeps the old name→pubkey binding
/// until the next restart — the identity desync in #2423.
#[test]
fn rename_re_retains_identity_record_with_new_name() {
    let dir = TempDir::new().unwrap();
    let keys = nostr::Keys::generate();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    let owner = keys.public_key().to_hex();
    let pubkey = "9".repeat(64);
    let mut record = sample_record(&pubkey, "Fizz");

    assert!(retain_agent_record(&conn, &keys, &record).unwrap());
    let first = get_retained_event(&conn, KIND_MANAGED_AGENT, &owner, &pubkey)
        .unwrap()
        .unwrap();
    // Simulate the flush loop confirming the initial publish.
    mark_synced(
        &conn,
        first.kind,
        &first.pubkey,
        &first.d_tag,
        first.created_at,
        &first.content,
    )
    .unwrap();

    record.name = "Spark".to_string();
    assert!(
        retain_agent_record(&conn, &keys, &record).unwrap(),
        "a renamed record must re-retain its identity record"
    );

    let row = get_retained_event(&conn, KIND_MANAGED_AGENT, &owner, &pubkey)
        .unwrap()
        .unwrap();
    assert_eq!(row.d_tag, pubkey, "coordinate stays keyed by agent pubkey");
    assert!(
        row.content.contains("Spark"),
        "retained identity record must carry the new name"
    );
    assert!(
        !row.content.contains("Fizz"),
        "the stale name must not survive the rename"
    );
    assert!(row.pending_sync, "a rename must queue a republish");
    assert!(
        row.created_at > first.created_at,
        "created_at must bump past the retained head (replaceable-event rule)"
    );
}

/// An unchanged record is a true no-op: no rewrite, no `pending_sync` churn.
/// This is what lets every edit path call the engine unconditionally.
#[test]
fn retain_agent_record_is_noop_when_unchanged() {
    let dir = TempDir::new().unwrap();
    let keys = nostr::Keys::generate();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    let pubkey = "8".repeat(64);
    let record = sample_record(&pubkey, "steady-agent");

    assert!(retain_agent_record(&conn, &keys, &record).unwrap());
    let row = get_retained_event(
        &conn,
        KIND_MANAGED_AGENT,
        &keys.public_key().to_hex(),
        &pubkey,
    )
    .unwrap()
    .unwrap();
    mark_synced(
        &conn,
        row.kind,
        &row.pubkey,
        &row.d_tag,
        row.created_at,
        &row.content,
    )
    .unwrap();

    assert!(
        !retain_agent_record(&conn, &keys, &record).unwrap(),
        "an unchanged projection must not re-retain"
    );
    assert!(
        get_pending_sync(&conn).unwrap().is_empty(),
        "no pending_sync churn for an unchanged record"
    );
}

/// `effort_level` is the single persisted effort authority (main #4625). A
/// follower device must run the leader's effort, so the private payload has to
/// carry it and an effort-only edit has to publish a successor head.
#[test]
fn effort_level_is_portable_in_private_config() {
    let dir = TempDir::new().unwrap();
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    let owner_hex = owner_keys.public_key().to_hex();
    let mut record = sample_record(&pubkey, "effort-agent");
    record.private_key_nsec = agent_keys.secret_key().to_bech32().unwrap();
    record.effort_level = Some("high".into());
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();

    let decrypt_head = |conn: &rusqlite::Connection| {
        let row = get_retained_event(conn, KIND_PRIVATE_MANAGED_AGENT, &owner_hex, &pubkey)
            .unwrap()
            .unwrap();
        let event = nostr::Event::from_json(&row.raw_event).unwrap();
        private_managed_agent::validate_and_decrypt(&event, &owner_keys)
            .unwrap()
            .1
    };

    assert!(retain_agent_record(&conn, &owner_keys, &record).unwrap());
    let payload = decrypt_head(&conn);
    assert_eq!(payload.config.effort_level.as_deref(), Some("high"));

    record.effort_level = Some("low".into());
    assert!(
        retain_agent_record(&conn, &owner_keys, &record).unwrap(),
        "an effort-only edit must publish a successor head"
    );
    let payload = decrypt_head(&conn);
    assert_eq!(payload.generation, 2);
    assert_eq!(payload.config.effort_level.as_deref(), Some("low"));
}

mod field_classification_tests;
mod self_authored_overlay_tests;
mod stale_republish_tests;

#[test]
fn public_recreation_does_not_boot_mint_deleted_private_settings() {
    let dir = TempDir::new().unwrap();
    let owner_keys = nostr::Keys::generate();
    let owner = owner_keys.public_key().to_hex();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    let mut record = sample_record(&pubkey, "public recreation");
    record.private_key_nsec = agent_keys.secret_key().to_bech32().unwrap();
    record
        .env_vars
        .insert("API_TOKEN".into(), "deleted private credential".into());
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    retain_event(
        &conn,
        &RetainedEvent {
            kind: 5,
            pubkey: owner.clone(),
            d_tag: crate::managed_agents::retention::tombstone_retention_d_tag(
                KIND_PRIVATE_MANAGED_AGENT,
                &pubkey,
            ),
            content: String::new(),
            created_at: 1,
            raw_event: "{}".into(),
            pending_sync: false,
        },
    )
    .unwrap();
    retain_agent_record_at_boot(&conn, &owner_keys, &record).unwrap();
    assert!(
        get_retained_event(&conn, KIND_MANAGED_AGENT, &owner, &pubkey)
            .unwrap()
            .is_some()
    );
    assert!(
        get_retained_event(&conn, KIND_PRIVATE_MANAGED_AGENT, &owner, &pubkey)
            .unwrap()
            .is_none(),
        "public recreation cannot restore deleted private config from disk at boot"
    );
}
