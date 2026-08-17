//! Tests for the strip-on-save / hydrate-on-load seam.
//!
//! The load-bearing property proven here is the CRITICAL F1 fix: a save NEVER
//! eagerly deletes the old generation. The atomic JSON write is the commit
//! point; if it fails, the on-disk record still points at the OLD generation,
//! so that generation must remain in the keyring and stay hydratable. Old-gen
//! retirement is left entirely to the two-cycle GC.

use super::*;
use crate::managed_agents::secret_projection::{
    agent_auth_tag_key, agent_env_key, agent_provider_config_key, definition_env_key,
    global_env_key, write_secret, WriteOutcome,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

// ── Fake store (mirrors secret_projection_tests::FakeProjectionStore) ──────

struct FakeProjectionStore {
    data: RefCell<HashMap<String, String>>,
}

impl FakeProjectionStore {
    fn new() -> Self {
        Self {
            data: RefCell::new(HashMap::new()),
        }
    }
    fn with_entry(self, key: &str, value: &str) -> Self {
        self.data
            .borrow_mut()
            .insert(key.to_string(), value.to_string());
        self
    }
    fn contains(&self, key: &str) -> bool {
        self.data.borrow().contains_key(key)
    }
}

impl ProjectionStore for FakeProjectionStore {
    fn write_and_verify(&self, key: &str, value: &str) -> Result<(), String> {
        self.data
            .borrow_mut()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }
    fn load_key(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.data.borrow().get(key).cloned())
    }
    fn load_all(&self) -> Result<Option<HashMap<String, String>>, String> {
        Ok(Some(self.data.borrow().clone()))
    }
    fn store_batch(&self, entries: &HashMap<String, String>) -> Result<(), String> {
        for (k, v) in entries {
            self.data.borrow_mut().insert(k.clone(), v.clone());
        }
        Ok(())
    }
    fn remove_batch(&self, keys: &[&str]) -> Result<(), String> {
        for k in keys {
            self.data.borrow_mut().remove(*k);
        }
        Ok(())
    }
}

// ── Fake store that fails loads for keys pointing at a specific gen ─────────
//
// Simulates a keyring outage where an entry's *_ref is present in JSON but the
// blob is unreachable — the exact condition that sets `secrets_unavailable`.
struct FailingLoadStore {
    fail_substr: String,
}

impl FailingLoadStore {
    fn new(fail_substr: &str) -> Self {
        Self {
            fail_substr: fail_substr.to_string(),
        }
    }
}

impl ProjectionStore for FailingLoadStore {
    fn write_and_verify(&self, _key: &str, _value: &str) -> Result<(), String> {
        Ok(())
    }
    fn load_key(&self, key: &str) -> Result<Option<String>, String> {
        if key.contains(&self.fail_substr) {
            Err(format!("simulated keyring outage for {key}"))
        } else {
            Ok(None)
        }
    }
    fn load_all(&self) -> Result<Option<HashMap<String, String>>, String> {
        Ok(Some(HashMap::new()))
    }
    fn store_batch(&self, _entries: &HashMap<String, String>) -> Result<(), String> {
        Ok(())
    }
    fn remove_batch(&self, _keys: &[&str]) -> Result<(), String> {
        Ok(())
    }
}

// ── Record builders ────────────────────────────────────────────────────────

fn instance_record(pubkey: &str) -> ManagedAgentRecord {
    serde_json::from_str(&format!(
        r#"{{
            "pubkey": "{pubkey}",
            "name": "test-agent",
            "private_key_nsec": "nsec1realkey",
            "relay_url": "wss://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }}"#
    ))
    .expect("instance record")
}

fn definition_record(slug: &str) -> ManagedAgentRecord {
    serde_json::from_str(&format!(
        r#"{{
            "pubkey": "",
            "name": "test-def",
            "slug": "{slug}",
            "relay_url": "",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }}"#
    ))
    .expect("definition record")
}

fn env_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// ── F1: old generation survives a save (no eager delete) ────────────────────

#[test]
fn test_instance_env_save_keeps_old_generation_for_failed_json_commit() {
    // Old generation is live on disk (record.env_vars_ref = "gen_old").
    let pubkey = "abc";
    let store = FakeProjectionStore::new()
        .with_entry(&agent_env_key(pubkey, "gen_old"), r#"{"OLD":"old-secret"}"#);

    // The on-disk record as it would be re-read after a FAILED JSON write:
    // it still points at the old generation because the new commit never landed.
    let mut on_disk = instance_record(pubkey);
    on_disk.env_vars_ref = Some("gen_old".to_string());

    // A save runs: new env is written to a new generation.
    let mut saving = instance_record(pubkey);
    saving.env_vars = env_map(&[("NEW", "new-secret")]);
    saving.env_vars_ref = Some("gen_old".to_string());
    strip_and_persist_agent_secrets_with(&store, &mut saving);

    // The old generation must NOT have been deleted by the save.
    assert!(
        store.contains(&agent_env_key(pubkey, "gen_old")),
        "old generation must survive the save — JSON commit could still fail"
    );

    // Simulate the JSON write failing: the on-disk record (old ref) is what
    // survives. Hydrating it must still resolve the old secret.
    let errors = hydrate_agent_secrets_with(&store, &mut on_disk);
    assert!(errors.is_empty(), "old ref must still hydrate: {errors:?}");
    assert_eq!(on_disk.env_vars, env_map(&[("OLD", "old-secret")]));
}

#[test]
fn test_instance_auth_tag_save_keeps_old_generation_for_failed_json_commit() {
    let pubkey = "abc";
    let store = FakeProjectionStore::new()
        .with_entry(&agent_auth_tag_key(pubkey, "gen_old"), "old-auth-tag");

    let mut on_disk = instance_record(pubkey);
    on_disk.auth_tag_ref = Some("gen_old".to_string());

    let mut saving = instance_record(pubkey);
    saving.auth_tag = Some("new-auth-tag".to_string());
    saving.auth_tag_ref = Some("gen_old".to_string());
    strip_and_persist_agent_secrets_with(&store, &mut saving);

    assert!(
        store.contains(&agent_auth_tag_key(pubkey, "gen_old")),
        "old auth_tag generation must survive the save"
    );

    let errors = hydrate_agent_secrets_with(&store, &mut on_disk);
    assert!(errors.is_empty(), "old auth ref must hydrate: {errors:?}");
    assert_eq!(on_disk.auth_tag.as_deref(), Some("old-auth-tag"));
}

#[test]
fn test_instance_provider_config_save_keeps_old_generation_for_failed_json_commit() {
    let pubkey = "abc";
    let old_config = r#"{"host":"old.example.com"}"#;
    let store = FakeProjectionStore::new()
        .with_entry(&agent_provider_config_key(pubkey, "gen_old"), old_config);

    let mut on_disk = instance_record(pubkey);
    on_disk.backend = BackendKind::Provider {
        id: "anthropic".to_string(),
        config: serde_json::Value::Null,
    };
    on_disk.provider_config_ref = Some("gen_old".to_string());

    let mut saving = instance_record(pubkey);
    saving.backend = BackendKind::Provider {
        id: "anthropic".to_string(),
        config: serde_json::json!({"host": "new.example.com"}),
    };
    saving.provider_config_ref = Some("gen_old".to_string());
    strip_and_persist_agent_secrets_with(&store, &mut saving);

    assert!(
        store.contains(&agent_provider_config_key(pubkey, "gen_old")),
        "old provider_config generation must survive the save"
    );

    let errors = hydrate_agent_secrets_with(&store, &mut on_disk);
    assert!(errors.is_empty(), "old pc ref must hydrate: {errors:?}");
    if let BackendKind::Provider { config, .. } = &on_disk.backend {
        assert_eq!(config, &serde_json::json!({"host": "old.example.com"}));
    } else {
        panic!("expected provider backend");
    }
}

#[test]
fn test_definition_env_save_keeps_old_generation_for_failed_json_commit() {
    let slug = "my-def";
    let store = FakeProjectionStore::new().with_entry(
        &definition_env_key(slug, "gen_old"),
        r#"{"OLD":"old-secret"}"#,
    );

    let mut on_disk = definition_record(slug);
    on_disk.env_vars_ref = Some("gen_old".to_string());

    let mut saving = definition_record(slug);
    saving.env_vars = env_map(&[("NEW", "new-secret")]);
    saving.env_vars_ref = Some("gen_old".to_string());
    strip_and_persist_definition_secrets_with(&store, &mut saving);

    assert!(
        store.contains(&definition_env_key(slug, "gen_old")),
        "old definition env generation must survive the save"
    );

    let errors = hydrate_definition_secrets_with(&store, &mut on_disk);
    assert!(errors.is_empty(), "old def ref must hydrate: {errors:?}");
    assert_eq!(on_disk.env_vars, env_map(&[("OLD", "old-secret")]));
}

#[test]
fn test_global_env_write_keeps_old_generation_for_failed_json_commit() {
    // Global config save has no seam fn (it lives in global_config::mod), but
    // its persistence uses the same write_secret primitive. Prove the
    // primitive does not disturb the prior generation: a new write creates a
    // NEW gen and the old gen stays intact and loadable.
    let store =
        FakeProjectionStore::new().with_entry(&global_env_key("gen_old"), r#"{"OLD":"old"}"#);

    let outcome = write_secret(
        &store,
        global_env_key,
        Some(r#"{"NEW":"new"}"#),
        "global env",
    );
    let new_gen = match outcome {
        WriteOutcome::Persisted { gen } => gen,
        other => panic!("expected Persisted, got {other:?}"),
    };
    assert_ne!(new_gen, "gen_old");

    // Old generation intact (JSON commit could still fail after this write).
    assert!(
        store.contains(&global_env_key("gen_old")),
        "old global env generation must survive the write"
    );
    // And still hydratable via its ref.
    let loaded = load_secret(&store, Some("gen_old"), global_env_key, "global env");
    assert_eq!(loaded, Ok(Some(r#"{"OLD":"old"}"#.to_string())));
}

// ── F3b: fail-closed — a failed hydrate never orphans the live generation ───
//
// Load path: an env_vars_ref present in JSON whose blob is unreachable must
// leave the field empty AND set `secrets_unavailable` — never silently drop
// the ref. Save path: persisting that unavailable record must PRESERVE the
// ref (empty-projection guard) so the still-live generation is not orphaned.

#[test]
fn test_instance_failed_env_hydrate_sets_secrets_unavailable() {
    let pubkey = "abc";
    let store = FailingLoadStore::new(&agent_env_key(pubkey, "gen_live"));
    let mut record = instance_record(pubkey);
    record.env_vars_ref = Some("gen_live".to_string());

    let errors = hydrate_agent_secrets_with(&store, &mut record);

    assert!(!errors.is_empty(), "a failed hydrate must surface an error");
    assert!(
        record.env_vars.is_empty(),
        "field stays empty on outage — no silent partial value"
    );
    // hydrate_all_secrets_for_records is the caller that sets the flag; assert
    // there so the propagation contract is covered end-to-end.
    let mut records = vec![{
        let mut r = instance_record(pubkey);
        r.env_vars_ref = Some("gen_live".to_string());
        r
    }];
    let unavailable = hydrate_all_secrets_for_records(&store, &mut records);
    assert!(
        records[0].secrets_unavailable,
        "outage must mark unavailable"
    );
    assert_eq!(unavailable, vec![pubkey.to_string()]);
}

#[test]
fn test_instance_save_preserves_ref_when_secrets_unavailable() {
    // The data-loss vector: a record whose env_vars_ref failed to hydrate holds
    // an empty env map. A naive save would write nothing and CLEAR the ref,
    // orphaning the live generation forever. The guard must keep the ref.
    let pubkey = "abc";
    let store =
        FakeProjectionStore::new().with_entry(&agent_env_key(pubkey, "gen_live"), r#"{"K":"v"}"#);

    let mut record = instance_record(pubkey);
    record.env_vars_ref = Some("gen_live".to_string());
    record.env_vars.clear(); // failed-hydrate state: ref present, map empty
    record.secrets_unavailable = true;

    strip_and_persist_agent_secrets_with(&store, &mut record);

    assert_eq!(
        record.env_vars_ref.as_deref(),
        Some("gen_live"),
        "ref must be preserved so the live generation is not orphaned"
    );
    assert!(
        store.contains(&agent_env_key(pubkey, "gen_live")),
        "the live generation must still exist"
    );
}

#[test]
fn test_instance_save_preserves_auth_and_provider_refs_when_unavailable() {
    let pubkey = "abc";
    let store = FakeProjectionStore::new()
        .with_entry(&agent_auth_tag_key(pubkey, "gen_a"), "live-auth")
        .with_entry(&agent_provider_config_key(pubkey, "gen_p"), r#"{"h":"x"}"#);

    let mut record = instance_record(pubkey);
    record.auth_tag_ref = Some("gen_a".to_string());
    record.backend = BackendKind::Provider {
        id: "anthropic".to_string(),
        config: serde_json::Value::Null,
    };
    record.provider_config_ref = Some("gen_p".to_string());
    record.secrets_unavailable = true;

    strip_and_persist_agent_secrets_with(&store, &mut record);

    assert_eq!(
        record.auth_tag_ref.as_deref(),
        Some("gen_a"),
        "auth_tag ref must survive an unavailable save"
    );
    assert_eq!(
        record.provider_config_ref.as_deref(),
        Some("gen_p"),
        "provider_config ref must survive an unavailable save"
    );
}

#[test]
fn test_available_record_clear_still_clears_ref() {
    // Regression guard for the guard: on an AVAILABLE record (not unavailable),
    // a genuinely-cleared field must still clear its ref — the fix must not
    // pin stale refs for a real user edit.
    let pubkey = "abc";
    let store = FakeProjectionStore::new();
    let mut record = instance_record(pubkey);
    record.env_vars_ref = Some("gen_old".to_string());
    record.env_vars.clear();
    record.secrets_unavailable = false; // available: this is a real clear

    strip_and_persist_agent_secrets_with(&store, &mut record);

    assert_eq!(
        record.env_vars_ref, None,
        "an available record's cleared field must clear its ref"
    );
}

#[test]
fn test_definition_failed_hydrate_then_save_preserves_ref() {
    // End-to-end for the definition tier: outage on load → secrets_unavailable
    // set by hydrate_all_secrets_for_records → save preserves the ref.
    let slug = "my-def";
    let mut records = vec![{
        let mut d = definition_record(slug);
        d.env_vars_ref = Some("gen_live".to_string());
        d
    }];

    let outage = FailingLoadStore::new(&definition_env_key(slug, "gen_live"));
    let unavailable = hydrate_all_secrets_for_records(&outage, &mut records);
    assert!(
        unavailable.is_empty(),
        "definitions are slug-keyed, not pushed to the pubkey summary"
    );
    assert!(
        records[0].secrets_unavailable,
        "definition outage must set secrets_unavailable"
    );
    assert!(records[0].env_vars.is_empty());

    // Now save against a store where the live gen still exists.
    let store = FakeProjectionStore::new()
        .with_entry(&definition_env_key(slug, "gen_live"), r#"{"K":"v"}"#);
    strip_and_persist_definition_secrets_with(&store, &mut records[0]);

    assert_eq!(
        records[0].env_vars_ref.as_deref(),
        Some("gen_live"),
        "definition ref must survive an unavailable save"
    );
    assert!(store.contains(&definition_env_key(slug, "gen_live")));
}

// ── F4: dev-migration conflict makes a coordinate unavailable through the ──
//       hydration boundary and refuses spawn (not just a withheld marker) ───
//
// The pass-1 fix only withheld the completion marker; the conflicted value
// stayed hydratable and could spawn during the retry window. These tests prove
// the conflict marker now propagates a real refusal: hydrate → secrets_
// unavailable → spawn_key_refusal.

#[test]
fn test_instance_env_conflict_marker_sets_secrets_unavailable_and_refuses_spawn() {
    use crate::managed_agents::secret_projection::conflict_marker_key;
    use crate::managed_agents::storage::spawn_key_refusal;

    let pubkey = "abc";
    let coord = agent_env_key(pubkey, "gen_live");
    // The generation IS present in the blob (destination has a value), but a
    // conflict marker for its coordinate means that value cannot be trusted.
    let store = FakeProjectionStore::new()
        .with_entry(&coord, r#"{"ANTHROPIC_API_KEY":"dest-value"}"#)
        .with_entry(&conflict_marker_key(&coord), "1");

    let mut records = vec![{
        let mut r = instance_record(pubkey);
        r.env_vars_ref = Some("gen_live".to_string());
        r
    }];

    let unavailable = hydrate_all_secrets_for_records(&store, &mut records);
    assert_eq!(
        unavailable,
        vec![pubkey.to_string()],
        "a conflicted coordinate must surface the instance as unavailable"
    );
    assert!(
        records[0].secrets_unavailable,
        "conflict marker must set secrets_unavailable through hydration"
    );
    assert!(
        records[0].env_vars.is_empty(),
        "the conflicted (untrusted) value must NOT be hydrated into the record"
    );
    // The launch boundary refuses — the conflicted value can never spawn.
    assert!(
        spawn_key_refusal(&records[0]).is_some(),
        "spawn must refuse an instance with an unresolved conflict"
    );
}

#[test]
fn test_definition_env_conflict_marker_sets_secrets_unavailable() {
    use crate::managed_agents::secret_projection::conflict_marker_key;

    let slug = "my-def";
    let coord = definition_env_key(slug, "gen_live");
    let store = FakeProjectionStore::new()
        .with_entry(&coord, r#"{"K":"dest"}"#)
        .with_entry(&conflict_marker_key(&coord), "1");

    let mut records = vec![{
        let mut d = definition_record(slug);
        d.env_vars_ref = Some("gen_live".to_string());
        d
    }];

    let _ = hydrate_all_secrets_for_records(&store, &mut records);
    assert!(
        records[0].secrets_unavailable,
        "a conflicted definition coordinate must set secrets_unavailable — the \
         linked-instance spawn/deploy gate then refuses"
    );
    assert!(records[0].env_vars.is_empty());
}

#[test]
fn test_cleared_conflict_marker_restores_availability_and_allows_spawn() {
    use crate::managed_agents::storage::spawn_key_refusal;

    // Once the migration clears the conflict marker (conflict resolved), the
    // same coordinate hydrates normally and spawn is no longer refused for it.
    let pubkey = "abc";
    let coord = agent_env_key(pubkey, "gen_live");
    let store =
        FakeProjectionStore::new().with_entry(&coord, r#"{"ANTHROPIC_API_KEY":"agreed-value"}"#);
    // No conflict marker present.

    let mut records = vec![{
        let mut r = instance_record(pubkey);
        r.env_vars_ref = Some("gen_live".to_string());
        r
    }];

    let unavailable = hydrate_all_secrets_for_records(&store, &mut records);
    assert!(
        unavailable.is_empty(),
        "with the conflict cleared, the coordinate hydrates cleanly"
    );
    assert!(!records[0].secrets_unavailable);
    assert_eq!(
        records[0]
            .env_vars
            .get("ANTHROPIC_API_KEY")
            .map(String::as_str),
        Some("agreed-value"),
        "the resolved value must hydrate once the conflict is cleared"
    );
    assert!(
        spawn_key_refusal(&records[0]).is_none(),
        "spawn must be allowed once the conflict is resolved"
    );
}

// ── W1: boot-migration transition never clears a committed ref ──────────────
//
// The strip-on-save seam reads an empty inline field on an available record as
// a deliberate user-clear and drops the ref. The boot migration re-reads
// ALREADY-PROJECTED records off disk (empty inline + live ref) on EVERY launch;
// routing those through the strip seam wiped every committed ref on the second
// launch (W1). These tests pin the migration seam's distinct contract: project
// a non-empty inline value, preserve an existing ref when inline is absent, and
// never clear a ref it did not write.

#[test]
fn test_migrate_inline_field_projects_nonempty_inline() {
    let store = FakeProjectionStore::new();
    let outcome = migrate_inline_field(
        &store,
        |gen| agent_env_key("abc", gen),
        Some(r#"{"K":"v"}"#),
        None,
        "agent:abc env_vars",
    );
    let gen = match outcome {
        FieldMigration::Projected { gen } => gen,
        other => panic!("expected Projected, got {other:?}"),
    };
    assert!(
        store.contains(&agent_env_key("abc", &gen)),
        "a projected value must be written under its new generation"
    );
}

#[test]
fn test_migrate_inline_field_preserves_ref_when_inline_absent() {
    // The W1 shape: an already-projected record (no inline, live ref). The
    // migration must PRESERVE the ref, never treat empty as a clear.
    let store =
        FakeProjectionStore::new().with_entry(&agent_env_key("abc", "gen_live"), r#"{"K":"v"}"#);
    let outcome = migrate_inline_field(
        &store,
        |gen| agent_env_key("abc", gen),
        None,
        Some("gen_live"),
        "agent:abc env_vars",
    );
    assert_eq!(
        outcome,
        FieldMigration::Preserved,
        "an absent inline with a live ref must preserve, not clear"
    );
    assert!(
        store.contains(&agent_env_key("abc", "gen_live")),
        "the preserved generation must remain in the keyring"
    );
}

#[test]
fn test_migrate_inline_field_cleared_when_no_inline_and_no_ref() {
    // Genuinely empty field: nothing inline, no ref. The migration does not
    // fabricate a ref — Cleared means "leave the field exactly as-is."
    let store = FakeProjectionStore::new();
    let outcome = migrate_inline_field(
        &store,
        |gen| agent_env_key("abc", gen),
        None,
        None,
        "agent:abc env_vars",
    );
    assert_eq!(outcome, FieldMigration::Cleared);
}

#[test]
fn test_migrate_two_launches_preserves_every_ref_for_instance_and_definition() {
    // The end-to-end W1 regression at the seam level: a first launch projects
    // inline env/auth/provider (instance) + env (definition) into fresh
    // generations; a second launch over the now-projected records (empty
    // inline, live refs) is a no-op that leaves EVERY ref intact. The old
    // strip-seam path cleared them all on launch two.
    let store = FakeProjectionStore::new();

    let mut instance = instance_record("abc");
    instance.env_vars = env_map(&[("ANTHROPIC_API_KEY", "sk-secret")]);
    instance.auth_tag = Some("auth-secret".to_string());
    instance.backend = BackendKind::Provider {
        id: "anthropic".to_string(),
        config: serde_json::json!({"api_key": "provider-secret"}),
    };
    let mut definition = definition_record("my-def");
    definition.env_vars = env_map(&[("DEF_KEY", "def-secret")]);
    let mut records = vec![instance, definition];

    // Launch 1: inline present → all fields projected.
    let changed1 = migrate_all_secrets_for_records(&store, &mut records);
    assert!(changed1, "first launch projects inline secrets");
    let env_ref = records[0].env_vars_ref.clone().expect("instance env ref");
    let auth_ref = records[0].auth_tag_ref.clone().expect("instance auth ref");
    let pc_ref = records[0]
        .provider_config_ref
        .clone()
        .expect("instance provider ref");
    let def_ref = records[1].env_vars_ref.clone().expect("definition env ref");
    assert!(
        records[0].env_vars.is_empty(),
        "inline env cleared on launch 1"
    );
    assert!(
        records[0].auth_tag.is_none(),
        "inline auth cleared on launch 1"
    );

    // Launch 2: records are already projected (empty inline, live refs).
    let changed2 = migrate_all_secrets_for_records(&store, &mut records);
    assert!(
        !changed2,
        "second launch is a no-op — no ref changes on already-projected records"
    );
    assert_eq!(
        records[0].env_vars_ref,
        Some(env_ref.clone()),
        "env ref survives launch 2"
    );
    assert_eq!(
        records[0].auth_tag_ref,
        Some(auth_ref.clone()),
        "auth ref survives launch 2"
    );
    assert_eq!(
        records[0].provider_config_ref,
        Some(pc_ref.clone()),
        "provider ref survives launch 2"
    );
    assert_eq!(
        records[1].env_vars_ref,
        Some(def_ref.clone()),
        "definition ref survives launch 2"
    );

    // The generations themselves must still be present and hydratable.
    assert!(store.contains(&agent_env_key("abc", &env_ref)));
    assert!(store.contains(&agent_auth_tag_key("abc", &auth_ref)));
    assert!(store.contains(&agent_provider_config_key("abc", &pc_ref)));
    assert!(store.contains(&definition_env_key("my-def", &def_ref)));
    let errors = hydrate_all_secrets_for_records(&store, &mut records);
    assert!(
        errors.is_empty(),
        "every ref must still hydrate after two launches: {errors:?}"
    );
    assert_eq!(
        records[0]
            .env_vars
            .get("ANTHROPIC_API_KEY")
            .map(String::as_str),
        Some("sk-secret")
    );
    assert_eq!(
        records[1].env_vars.get("DEF_KEY").map(String::as_str),
        Some("def-secret")
    );
}

#[test]
fn test_migrate_does_not_clear_ref_on_keyring_write_failure() {
    // Distinct from Preserved: an inline value present but the keyring write
    // fails (KeptInline). The migration must return Cleared (caller leaves the
    // field as-is, value stays inline for retry) and must NOT set a ref it did
    // not write. The record keeps its inline value; no ref is fabricated.
    struct FailingWriteStore;
    impl ProjectionStore for FailingWriteStore {
        fn write_and_verify(&self, _key: &str, _value: &str) -> Result<(), String> {
            Err("simulated keyring write failure".to_string())
        }
        fn load_key(&self, _key: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn load_all(&self) -> Result<Option<HashMap<String, String>>, String> {
            Ok(Some(HashMap::new()))
        }
        fn store_batch(&self, _entries: &HashMap<String, String>) -> Result<(), String> {
            Ok(())
        }
        fn remove_batch(&self, _keys: &[&str]) -> Result<(), String> {
            Ok(())
        }
    }
    let store = FailingWriteStore;
    let mut record = instance_record("abc");
    record.env_vars = env_map(&[("K", "v")]);
    // No pre-existing ref.
    migrate_agent_secrets_with(&store, &mut record);
    assert_eq!(
        record.env_vars,
        env_map(&[("K", "v")]),
        "a failed keyring write must keep the value inline for retry"
    );
    assert_eq!(
        record.env_vars_ref, None,
        "the migration must not fabricate a ref it did not write"
    );
}

#[test]
fn test_migrate_two_launches_survive_both_gc_cycles() {
    // The full W1 regression through the real GC: after a first launch projects
    // inline secrets into fresh generations and commits the refs to JSON, a
    // complete two-cycle GC sweep (delete→mark, twice, the boot order) must NOT
    // reclaim any of those generations — they are all referenced by live JSON —
    // and a second launch's migration must leave every ref intact. Before the
    // W1 fix the second launch cleared the refs, orphaning the generations, and
    // this GC would then delete them.
    use crate::managed_agents::secret_projection::{
        collect_live_refs, delete_gc_candidates, mark_gc_candidates,
    };

    let store = FakeProjectionStore::new();

    let mut instance = instance_record("abc");
    instance.env_vars = env_map(&[("ANTHROPIC_API_KEY", "sk-secret")]);
    instance.auth_tag = Some("auth-secret".to_string());
    instance.backend = BackendKind::Provider {
        id: "anthropic".to_string(),
        config: serde_json::json!({"api_key": "provider-secret"}),
    };
    let mut definition = definition_record("my-def");
    definition.env_vars = env_map(&[("DEF_KEY", "def-secret")]);
    let mut records = vec![instance, definition];

    // Launch 1: project inline → refs.
    migrate_all_secrets_for_records(&store, &mut records);
    let env_ref = records[0].env_vars_ref.clone().expect("env ref");
    let auth_ref = records[0].auth_tag_ref.clone().expect("auth ref");
    let pc_ref = records[0]
        .provider_config_ref
        .clone()
        .expect("provider ref");
    let def_ref = records[1].env_vars_ref.clone().expect("def ref");

    // Commit the projected records to disk exactly as the migration does, so GC
    // reads the same JSON production would. Global config is empty.
    let dir = tempfile::tempdir().expect("tempdir");
    let agents_path = dir.path().join("managed-agents.json");
    let global_path = dir.path().join("global-agent-config.json");
    std::fs::write(
        &agents_path,
        serde_json::to_string(&records).expect("serialize records"),
    )
    .expect("write agents json");
    std::fs::write(&global_path, "{}").expect("write global json");

    // Sanity: the committed JSON is a clean projection — every ref resolves to
    // a live coordinate present in the blob, no inline+ref conflict.
    let agents_json = std::fs::read_to_string(&agents_path).unwrap();
    let live = collect_live_refs(&agents_json, "{}").expect("clean live refs");
    for gen in [&env_ref, &auth_ref, &pc_ref, &def_ref] {
        assert!(live.gen_ids.contains(gen), "{gen} must be a live ref");
    }

    // Two full boot-order GC cycles (delete before mark) over the committed
    // JSON. A referenced generation is never a deletion candidate.
    for _ in 0..2 {
        delete_gc_candidates(&store, &agents_path, &global_path);
        mark_gc_candidates(&store, &agents_path, &global_path);
    }

    // Launch 2: migrate again over the already-projected records (empty inline,
    // live refs). Must be a no-op that preserves every ref.
    let changed = migrate_all_secrets_for_records(&store, &mut records);
    assert!(!changed, "second launch must not change any ref");

    // Every generation must have survived both GC cycles and still hydrate.
    assert!(store.contains(&agent_env_key("abc", &env_ref)));
    assert!(store.contains(&agent_auth_tag_key("abc", &auth_ref)));
    assert!(store.contains(&agent_provider_config_key("abc", &pc_ref)));
    assert!(store.contains(&definition_env_key("my-def", &def_ref)));
    let errors = hydrate_all_secrets_for_records(&store, &mut records);
    assert!(
        errors.is_empty(),
        "every ref must still hydrate after two launches + two GC cycles: {errors:?}"
    );
    assert_eq!(
        records[0]
            .env_vars
            .get("ANTHROPIC_API_KEY")
            .map(String::as_str),
        Some("sk-secret")
    );
    assert_eq!(
        records[1].env_vars.get("DEF_KEY").map(String::as_str),
        Some("def-secret")
    );
}
