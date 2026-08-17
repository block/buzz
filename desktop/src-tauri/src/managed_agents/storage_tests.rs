//! Unit tests for `managed_agents/storage.rs`.
//!
//! Kept in a sibling file so `storage.rs` stays closer to the 1000-line gate;
//! `#[path]`-included from there.

use std::cell::RefCell;
use std::collections::HashMap;

use super::{
    agent_keyring_name, hydrate_keys_with, migrate_inline_key, persist_agent_keys_with,
    KeyMigration, KeyStore, KeyringProbe, ManagedAgentRecord,
};

/// In-memory [`KeyStore`] for testing the migrate decision without the OS
/// keyring. `reachable=false` simulates a backend outage; `fail_verify`
/// simulates a write whose read-back does not confirm.
struct FakeKeyStore {
    reachable: bool,
    fail_verify: bool,
    stored: RefCell<HashMap<String, String>>,
    write_count: RefCell<usize>,
    read_count: RefCell<usize>,
}

impl FakeKeyStore {
    fn reachable() -> Self {
        Self {
            reachable: true,
            fail_verify: false,
            stored: RefCell::new(HashMap::new()),
            write_count: RefCell::new(0),
            read_count: RefCell::new(0),
        }
    }
    fn unreachable() -> Self {
        Self {
            reachable: false,
            fail_verify: false,
            stored: RefCell::new(HashMap::new()),
            write_count: RefCell::new(0),
            read_count: RefCell::new(0),
        }
    }
    fn verify_fails() -> Self {
        Self {
            reachable: true,
            fail_verify: true,
            stored: RefCell::new(HashMap::new()),
            write_count: RefCell::new(0),
            read_count: RefCell::new(0),
        }
    }
    /// Seed a key as already present in the keyring.
    fn with_key(self, name: &str, value: &str) -> Self {
        self.stored
            .borrow_mut()
            .insert(name.to_string(), value.to_string());
        self
    }
}

impl KeyStore for FakeKeyStore {
    fn probe(&self, _name: &str) -> KeyringProbe {
        if self.reachable {
            KeyringProbe::ReachableButEmpty
        } else {
            KeyringProbe::Unreachable
        }
    }
    fn load(&self, name: &str) -> Result<Option<String>, String> {
        // An unreachable backend errors on read (outage), distinct from a
        // reachable backend returning `Ok(None)` for an absent entry.
        if !self.reachable {
            return Err("keyring backend unreachable".to_string());
        }
        *self.read_count.borrow_mut() += 1;
        Ok(self.stored.borrow().get(name).cloned())
    }
    fn load_all_readonly(&self) -> Result<Option<HashMap<String, String>>, String> {
        if !self.reachable {
            return Err("keyring backend unreachable".to_string());
        }
        *self.read_count.borrow_mut() += 1;
        let map = self.stored.borrow().clone();
        // Return None when completely empty (simulates no blob written yet).
        if map.is_empty() {
            Ok(None)
        } else {
            Ok(Some(map))
        }
    }
    fn write_and_verify(&self, name: &str, value: &str) -> Result<(), String> {
        if self.fail_verify {
            return Err("read-back verify failed".to_string());
        }
        *self.write_count.borrow_mut() += 1;
        self.stored
            .borrow_mut()
            .insert(name.to_string(), value.to_string());
        Ok(())
    }
    fn store_all(&self, entries: &HashMap<String, String>) -> Result<(), String> {
        if !self.reachable {
            return Err("keyring backend unreachable".to_string());
        }
        if self.fail_verify {
            return Err("read-back verify failed".to_string());
        }
        *self.write_count.borrow_mut() += 1;
        let mut stored = self.stored.borrow_mut();
        for (k, v) in entries {
            stored.insert(k.clone(), v.clone());
        }
        Ok(())
    }
}

fn record_with_key(nsec: &str) -> ManagedAgentRecord {
    record_with_pubkey_and_key("agent-pubkey", nsec)
}

fn record_with_pubkey_and_key(pubkey: &str, nsec: &str) -> ManagedAgentRecord {
    serde_json::from_str(&format!(
        r#"{{
            "pubkey": "{pubkey}",
            "name": "test-agent",
            "private_key_nsec": "{nsec}",
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
    .expect("sample record")
}

#[test]
fn migrate_persists_and_signals_stripping_when_keyring_reachable() {
    // Item 2: an inline key (residue from a prior keyring-unreachable save)
    // is written to the keyring and verified when the backend is reachable,
    // so the next save can drop it from JSON.
    let store = FakeKeyStore::reachable();
    let record = record_with_key("nsec1realkey");

    let outcome = migrate_inline_key(&store, &record);

    assert_eq!(outcome, KeyMigration::Persisted);
    assert_eq!(
        store
            .stored
            .borrow()
            .get(&agent_keyring_name("agent-pubkey"))
            .map(String::as_str),
        Some("nsec1realkey")
    );
}

#[test]
fn migrate_keeps_inline_when_keyring_unreachable() {
    // No-resurrection guard: a transient outage must NOT migrate; the key
    // stays inline (file fallback) so it is not lost.
    let store = FakeKeyStore::unreachable();
    let record = record_with_key("nsec1realkey");

    let outcome = migrate_inline_key(&store, &record);

    assert_eq!(outcome, KeyMigration::KeptInline);
    assert!(store.stored.borrow().is_empty());
}

#[test]
fn migrate_keeps_inline_when_verify_fails() {
    // A write whose read-back does not confirm must keep the key inline —
    // never drop plaintext on an unverified write.
    let store = FakeKeyStore::verify_fails();
    let record = record_with_key("nsec1realkey");

    assert_eq!(
        migrate_inline_key(&store, &record),
        KeyMigration::KeptInline
    );
}

#[test]
fn migrate_reports_nothing_for_empty_key() {
    // A record whose key already lives in the keyring (empty inline) has
    // nothing to migrate. It must NOT be reported as `Persisted` — an
    // empty key after a keyring outage means the secret is unavailable,
    // not verified present (Wes storage.rs:158).
    let store = FakeKeyStore::reachable();
    let record = record_with_key("");

    assert_eq!(migrate_inline_key(&store, &record), KeyMigration::Nothing);
    assert!(store.stored.borrow().is_empty());
}

#[test]
fn hydrate_fills_key_from_keyring_when_reachable() {
    // The normal keyring-backed case: an empty inline key is filled from
    // the keyring on load.
    let store =
        FakeKeyStore::reachable().with_key(&agent_keyring_name("agent-pubkey"), "nsec1stored");
    let mut records = vec![record_with_key("")];

    hydrate_keys_with(&store, &mut records);

    assert_eq!(records[0].private_key_nsec, "nsec1stored");
}

#[test]
fn hydrate_leaves_key_empty_on_keyring_outage() {
    // Outage edge (Wes storage.rs:158): when the keyring read ERRORS, the
    // key must be left empty — never silently treated as resolved — so the
    // spawn path refuses rather than launching the agent with no identity.
    let store = FakeKeyStore::unreachable();
    let mut records = vec![record_with_key("")];

    hydrate_keys_with(&store, &mut records);

    assert!(
        records[0].private_key_nsec.is_empty(),
        "an unreadable key must stay empty, not be fabricated"
    );
}

#[test]
fn spawn_refused_when_private_key_empty() {
    // The spawn path MUST refuse a record left empty by an outage/absence
    // before injecting an empty BUZZ_PRIVATE_KEY / NOSTR_PRIVATE_KEY — never
    // launch an agent with no identity (Wes storage.rs:158).
    let record = record_with_key("");
    assert!(
        super::spawn_key_refusal(&record).is_some(),
        "an agent with no private key must be refused"
    );
}

#[test]
fn spawn_allowed_when_private_key_present() {
    // A record carrying a key must not be blocked by the refusal guard.
    let record = record_with_key("nsec1realkey");
    assert!(super::spawn_key_refusal(&record).is_none());
}

#[test]
fn spawn_refused_when_secrets_unavailable() {
    // A record whose keyring ref exists but the entry is unavailable must be
    // refused at spawn time — same semantics as a missing private key.
    let mut record = record_with_key("nsec1realkey");
    record.secrets_unavailable = true;
    assert!(
        super::spawn_key_refusal(&record).is_some(),
        "an agent with unavailable secrets must be refused at spawn"
    );
}

#[test]
fn spawn_allowed_when_key_present_and_no_unavailable_secrets() {
    // A fully hydrated record must not be blocked.
    let mut record = record_with_key("nsec1realkey");
    record.secrets_unavailable = false;
    assert!(
        super::spawn_key_refusal(&record).is_none(),
        "an agent with key and reachable secrets must be allowed"
    );
}

#[test]
fn persist_agent_keys_issues_zero_writes_when_inline_keys_already_cleared() {
    // This is the dominant prompt-storm scenario: after the first successful
    // persist all inline copies are cleared, so subsequent saves (e.g. a
    // model change) must issue zero keychain writes. `migrate_inline_key`
    // returns `Nothing` for empty-key records, and `persist_agent_keys_with`
    // must propagate that guarantee — write_count stays at 0.
    let store = FakeKeyStore::reachable();
    // Records whose inline key is already blank (key lives in the keyring).
    let mut records = vec![record_with_key(""), record_with_key("")];

    persist_agent_keys_with(&store, &mut records);

    assert_eq!(
        *store.write_count.borrow(),
        0,
        "a save with no inline keys must issue zero keychain writes"
    );
}

#[test]
fn persist_agent_keys_writes_once_per_record_with_inline_key() {
    // A record carrying an inline key (e.g. first save, or keyring-outage
    // residue) must trigger exactly one write_and_verify per record — and
    // once persisted the inline copy is cleared so the next save is free.
    // Records use distinct pubkeys so each maps to a distinct keyring name,
    // verifying the "per record" behaviour rather than a single-key overwrite.
    let store = FakeKeyStore::reachable();
    let mut records = vec![
        record_with_pubkey_and_key("pubkey-agent-alpha", "nsec1key_a"),
        record_with_pubkey_and_key("pubkey-agent-beta", "nsec1key_b"),
    ];

    persist_agent_keys_with(&store, &mut records);

    assert_eq!(
        *store.write_count.borrow(),
        2,
        "each record with an inline key must trigger exactly one write"
    );
    // Verify the correct keyring name was used for each agent.
    assert_eq!(
        store
            .stored
            .borrow()
            .get(&agent_keyring_name("pubkey-agent-alpha"))
            .map(String::as_str),
        Some("nsec1key_a"),
    );
    assert_eq!(
        store
            .stored
            .borrow()
            .get(&agent_keyring_name("pubkey-agent-beta"))
            .map(String::as_str),
        Some("nsec1key_b"),
    );
    // After persist the inline copies are cleared — next save is zero-write.
    assert!(records[0].private_key_nsec.is_empty());
    assert!(records[1].private_key_nsec.is_empty());
}

/// The keyringless fallback write must land `0o600` from the write itself —
/// not a post-write `chmod` — so a crash in the umask window can never leave
/// plaintext agent nsecs world-readable (Wes storage.rs:239, SECURITY.md:90).
#[cfg(unix)]
#[test]
fn restricted_write_lands_owner_only_without_post_write_chmod() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("managed-agents.json");

    super::atomic_write_json_restricted(&path, br#"[{"private_key_nsec":"nsec1secret"}]"#)
        .expect("restricted write");

    let mode = std::fs::metadata(&path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "secret-bearing write must be owner-only");
    assert_eq!(
        std::fs::read_to_string(&path).expect("read back"),
        r#"[{"private_key_nsec":"nsec1secret"}]"#
    );
}

// ── keyring-dev-migration tests ────────────────────────────────────────

#[test]
fn copy_agent_keys_copies_keys_present_in_src_to_dst() {
    // Keys in src but not in dst must be copied in a single bulk write,
    // and the migration-complete marker must be set.
    let src = FakeKeyStore::reachable()
        .with_key(&agent_keyring_name("agent-alpha"), "nsec1alpha")
        .with_key(&agent_keyring_name("agent-beta"), "nsec1beta");
    let dst = FakeKeyStore::reachable();

    super::copy_agent_keys_between_stores(
        &["agent-alpha".to_string(), "agent-beta".to_string()],
        &src,
        &dst,
    );

    assert_eq!(
        dst.stored
            .borrow()
            .get(&agent_keyring_name("agent-alpha"))
            .map(String::as_str),
        Some("nsec1alpha"),
        "agent-alpha must be copied from src to dst"
    );
    assert_eq!(
        dst.stored
            .borrow()
            .get(&agent_keyring_name("agent-beta"))
            .map(String::as_str),
        Some("nsec1beta"),
        "agent-beta must be copied from src to dst"
    );
    assert_eq!(
        dst.stored
            .borrow()
            .get(super::DEV_MIGRATION_MARKER)
            .map(String::as_str),
        Some("done"),
        "migration-complete marker must be set after first migration"
    );
    // Bulk write: exactly 1 store_all call.
    assert_eq!(
        *dst.write_count.borrow(),
        1,
        "must perform exactly one bulk write"
    );
    // Src accessed exactly once (bulk blob read).
    assert_eq!(
        *src.read_count.borrow(),
        1,
        "src must be read exactly once (bulk)"
    );
}

#[test]
fn copy_agent_keys_skips_keys_already_in_dst() {
    // Idempotency: a key already present in dst must NOT be overwritten
    // — the agent may have rotated their key in the dev service.
    let src = FakeKeyStore::reachable().with_key(&agent_keyring_name("agent-alpha"), "nsec1old");
    let dst = FakeKeyStore::reachable().with_key(&agent_keyring_name("agent-alpha"), "nsec1new");

    super::copy_agent_keys_between_stores(&["agent-alpha".to_string()], &src, &dst);

    // dst value must remain unchanged — src must not overwrite it.
    assert_eq!(
        dst.stored
            .borrow()
            .get(&agent_keyring_name("agent-alpha"))
            .map(String::as_str),
        Some("nsec1new"),
        "key already in dst must not be overwritten by migration"
    );
    // Marker must still be written even though no new keys were copied.
    assert_eq!(
        dst.stored
            .borrow()
            .get(super::DEV_MIGRATION_MARKER)
            .map(String::as_str),
        Some("done"),
        "marker must be set even when all keys are already present"
    );
    assert_eq!(*src.read_count.borrow(), 0);
}

#[test]
fn copy_agent_keys_skips_keys_absent_from_src() {
    // A pubkey with no entry in src (new agent that will mint a fresh key)
    // must be silently skipped — no agent key written to dst.
    let src = FakeKeyStore::reachable(); // empty
    let dst = FakeKeyStore::reachable();

    super::copy_agent_keys_between_stores(&["new-agent".to_string()], &src, &dst);

    assert!(
        dst.stored
            .borrow()
            .get(&agent_keyring_name("new-agent"))
            .is_none(),
        "absent src key must produce no agent key write to dst"
    );
    // Marker must still be written.
    assert_eq!(
        dst.stored
            .borrow()
            .get(super::DEV_MIGRATION_MARKER)
            .map(String::as_str),
        Some("done"),
        "marker must be set even when no keys were present in src"
    );
}

#[test]
fn copy_agent_keys_skips_all_when_dst_unreachable() {
    // When dst keyring is unreachable the migration must be a no-op — never
    // data-loss (failing to write is fine; the agent will re-mint on next
    // onboarding run).
    let src = FakeKeyStore::reachable().with_key(&agent_keyring_name("agent-alpha"), "nsec1alpha");
    let dst = FakeKeyStore::unreachable();

    super::copy_agent_keys_between_stores(&["agent-alpha".to_string()], &src, &dst);

    // No writes attempted to an unreachable dst.
    assert_eq!(*dst.write_count.borrow(), 0);
    // Src must not have been accessed (failed on dst read, returned early).
    assert_eq!(
        *src.read_count.borrow(),
        0,
        "src must not be accessed when dst is unreachable"
    );
}

#[test]
fn copy_agent_keys_skips_entirely_when_marker_present() {
    // After the first migration, the marker is in dst. Subsequent calls
    // must return immediately — the prod keyring (src) must never be read.
    let src = FakeKeyStore::reachable().with_key(&agent_keyring_name("agent-alpha"), "nsec1alpha");
    let dst = FakeKeyStore::reachable()
        .with_key(super::DEV_MIGRATION_MARKER, "done")
        .with_key(&agent_keyring_name("agent-alpha"), "nsec1dev");

    super::copy_agent_keys_between_stores(&["agent-alpha".to_string()], &src, &dst);

    // Src must not have been accessed at all.
    assert_eq!(
        *src.read_count.borrow(),
        0,
        "src must not be read when migration-complete marker is present"
    );
    // Dst must not have been written.
    assert_eq!(
        *dst.write_count.borrow(),
        0,
        "dst must not be written when migration-complete marker is present"
    );
    // Dev key must remain unchanged.
    assert_eq!(
        dst.stored
            .borrow()
            .get(&agent_keyring_name("agent-alpha"))
            .map(String::as_str),
        Some("nsec1dev"),
        "dev key must not be overwritten on subsequent boots"
    );
}

#[test]
fn copy_agent_keys_writes_marker_even_with_empty_agent_list() {
    // An empty pubkey list (no agents yet) must still write the marker so
    // future boots skip the prod read.
    let src = FakeKeyStore::reachable();
    let dst = FakeKeyStore::reachable();

    super::copy_agent_keys_between_stores(&[], &src, &dst);

    assert_eq!(
        dst.stored
            .borrow()
            .get(super::DEV_MIGRATION_MARKER)
            .map(String::as_str),
        Some("done"),
        "marker must be set even when pubkey list is empty"
    );
    assert_eq!(*src.read_count.borrow(), 0);
}

#[test]
fn try_delete_agent_key_returns_result() {
    // Verify the result-returning seam exists and has the correct signature.
    // We cannot call it in default builds (system-keyring feature is on,
    // which accesses the real OS keychain and blocks in headless/CI). The
    // real keychain paths are integration-tested through the #[ignore]
    // tests in secret_store.rs; the rollback aggregation is tested in
    // team_snapshot::tests::rollback_aggregates_multiple_errors.
    let _: fn(&str) -> Result<(), String> = super::try_delete_agent_key;
}

/// Regression test: after secret extraction, the serialized store must not
/// contain any secret-shaped values.  This is the grep-empty criterion from
/// the v5 spec acceptance criteria, exercised at the type level.
///
/// The test constructs records with inline secrets, runs the strip path via
/// the secret seam, and verifies the resulting JSON is free of the known
/// secret values.
#[test]
fn serialized_store_is_empty_of_secret_values_after_strip() {
    use crate::managed_agents::secret_seam::strip_and_persist_agent_secrets_with;
    use std::cell::RefCell;
    use std::collections::HashMap;

    // ── FakeProjectionStore (minimal, for this test) ──────────────────
    struct FakePS {
        data: RefCell<HashMap<String, String>>,
    }
    impl crate::managed_agents::secret_projection::ProjectionStore for FakePS {
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
            let mut d = self.data.borrow_mut();
            for k in keys {
                d.remove(*k);
            }
            Ok(())
        }
    }

    let store = FakePS {
        data: RefCell::new(HashMap::new()),
    };

    let secret_env_value = "sk-ant-api03-very-secret-key";
    let secret_auth_tag = "auth-tag-secret";

    // Build a record via JSON deserialization (avoids Default dependency).
    let mut record: ManagedAgentRecord = serde_json::from_str(&format!(
        r#"{{
            "pubkey": "testpubkey123",
            "name": "test-agent",
            "env_vars": {{"ANTHROPIC_API_KEY": "{secret_env_value}"}},
            "auth_tag": "{secret_auth_tag}",
            "backend": {{"type": "provider", "id": "anthropic", "config": {{"api_key": "provider-secret"}}}},
            "relay_url": "wss://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "created_at": "2026-01-01",
            "updated_at": "2026-01-01"
        }}"#
    ))
    .expect("sample record with inline secrets");

    // Strip: moves secrets from inline fields into the fake keyring.
    strip_and_persist_agent_secrets_with(&store, &mut record);

    // After strip: inline fields must be empty/null.
    assert!(
        record.env_vars.is_empty(),
        "env_vars must be cleared after strip"
    );
    assert!(
        record.auth_tag.is_none(),
        "auth_tag must be cleared after strip"
    );
    if let crate::managed_agents::types::BackendKind::Provider { config, .. } = &record.backend {
        assert!(
            config.is_null(),
            "provider config must be cleared after strip"
        );
    }

    // Serialize to JSON and verify no secret bytes appear.
    let json = serde_json::to_string(&record).expect("serialize");
    assert!(
        !json.contains(secret_env_value),
        "serialized JSON must not contain env_vars secret"
    );
    assert!(
        !json.contains(secret_auth_tag),
        "serialized JSON must not contain auth_tag secret"
    );
    assert!(
        !json.contains("provider-secret"),
        "serialized JSON must not contain provider config secret"
    );
    // Refs must be present (the keyring round-trip worked).
    assert!(
        record.env_vars_ref.is_some(),
        "env_vars_ref must be set after successful strip"
    );
    assert!(
        record.auth_tag_ref.is_some(),
        "auth_tag_ref must be set after successful strip"
    );
    assert!(
        record.provider_config_ref.is_some(),
        "provider_config_ref must be set after successful strip"
    );
}

// ── W2: instance-side save preserves the definition half raw ───────────────
//
// `save_managed_agents` re-reads the definition half RAW under the txn lock
// (never through the hydrating loader) before committing the unified store, so
// an instance-only save can never re-inline a definition's projected secrets
// into plaintext JSON, and a store parse error propagates instead of silently
// collapsing the definition half to empty.

/// In-memory store implementing BOTH seams `save_managed_agents_at` requires:
/// [`KeyStore`] (nsec persistence) and `ProjectionStore` (env/auth/provider
/// projection). A single backing map so a written secret reads back verified.
struct FakeCombinedStore {
    data: RefCell<HashMap<String, String>>,
}

impl FakeCombinedStore {
    fn new() -> Self {
        Self {
            data: RefCell::new(HashMap::new()),
        }
    }
}

impl KeyStore for FakeCombinedStore {
    fn probe(&self, _name: &str) -> KeyringProbe {
        KeyringProbe::ReachableButEmpty
    }
    fn load(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self.data.borrow().get(name).cloned())
    }
    fn load_all_readonly(&self) -> Result<Option<HashMap<String, String>>, String> {
        Ok(Some(self.data.borrow().clone()))
    }
    fn write_and_verify(&self, name: &str, value: &str) -> Result<(), String> {
        self.data
            .borrow_mut()
            .insert(name.to_string(), value.to_string());
        Ok(())
    }
    fn store_all(&self, entries: &HashMap<String, String>) -> Result<(), String> {
        for (k, v) in entries {
            self.data.borrow_mut().insert(k.clone(), v.clone());
        }
        Ok(())
    }
}

impl crate::managed_agents::secret_projection::ProjectionStore for FakeCombinedStore {
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
        let mut d = self.data.borrow_mut();
        for k in keys {
            d.remove(*k);
        }
        Ok(())
    }
}

/// A key-less definition record carrying an already-projected `env_vars_ref`
/// (no inline env) — the on-disk shape after the definition's secrets were
/// stripped into the keyring on a prior save.
fn projected_definition(slug: &str, env_ref: &str) -> ManagedAgentRecord {
    let mut record: ManagedAgentRecord = serde_json::from_str(&format!(
        r#"{{
            "pubkey": "",
            "name": "def-{slug}",
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
    .expect("definition record");
    record.env_vars_ref = Some(env_ref.to_string());
    record
}

#[test]
fn save_managed_agents_preserves_projected_definition_ref_without_reinlining() {
    // Seed a store whose definition half is already projected: `env_vars_ref`
    // set, NO inline env bytes. Save an UNRELATED instance. The committed store
    // must keep the definition's ref verbatim and must not resurrect its inline
    // env — the W2 regression was that the hydrating re-read re-inlined it.
    use crate::managed_agents::secret_projection::definition_env_key;

    let dir = tempfile::tempdir().expect("tempdir");
    let store_path = dir.path().join("managed-agents.json");

    // The keyring already holds the definition's projected env under its ref.
    let store = FakeCombinedStore::new();
    let def_env_gen = "gendef123";
    store.data.borrow_mut().insert(
        definition_env_key("shared-def", def_env_gen),
        r#"{"DEF_SECRET":"def-secret-value"}"#.to_string(),
    );

    // On-disk starting store: one projected definition, no instances.
    let definition = projected_definition("shared-def", def_env_gen);
    std::fs::write(
        &store_path,
        serde_json::to_string(&[&definition]).expect("serialize seed"),
    )
    .expect("write seed store");

    // Save an unrelated instance (carries its own inline env to project).
    let mut instance = record_with_pubkey_and_key("instance-pubkey", "nsec1instkey");
    instance.env_vars = [("INSTANCE_KEY".to_string(), "inst-secret".to_string())]
        .into_iter()
        .collect();

    super::save_managed_agents_at(&store_path, Some(&store), std::slice::from_ref(&instance))
        .expect("save must succeed");

    // Re-read the committed store RAW (no hydration).
    let committed = std::fs::read_to_string(&store_path).expect("read committed");

    // The definition's plaintext secret must NOT be in the JSON.
    assert!(
        !committed.contains("def-secret-value"),
        "an instance-side save must not re-inline the definition's projected secret"
    );

    // The definition's ref must survive verbatim.
    let records: Vec<ManagedAgentRecord> =
        serde_json::from_str(&committed).expect("parse committed");
    let def = records
        .iter()
        .find(|r| r.pubkey.is_empty() && r.slug.as_deref() == Some("shared-def"))
        .expect("definition must survive the instance save");
    assert_eq!(
        def.env_vars_ref.as_deref(),
        Some(def_env_gen),
        "the definition's projected ref must be preserved unchanged"
    );
    assert!(
        def.env_vars.is_empty(),
        "the definition must carry no inline env after the save"
    );
    // The instance was persisted alongside it.
    assert!(
        records.iter().any(|r| r.pubkey == "instance-pubkey"),
        "the saved instance must be present in the committed store"
    );
}

#[test]
fn save_managed_agents_propagates_store_parse_error_instead_of_dropping_definitions() {
    // A malformed on-disk store must fail the save with an error — NEVER be
    // read as an empty definition half, because the wholesale rewrite would
    // then delete every definition from the live store. W2/F2: the re-read uses
    // `?`, not `unwrap_or_default()`.
    let dir = tempfile::tempdir().expect("tempdir");
    let store_path = dir.path().join("managed-agents.json");
    std::fs::write(&store_path, b"{ this is not valid json ]").expect("write malformed");

    let store = FakeCombinedStore::new();
    let instance = record_with_pubkey_and_key("instance-pubkey", "nsec1instkey");

    let result =
        super::save_managed_agents_at(&store_path, Some(&store), std::slice::from_ref(&instance));

    assert!(
        result.is_err(),
        "a malformed store must fail the save, not silently drop definitions"
    );
    assert!(
        result.unwrap_err().contains("parse"),
        "the error must surface the parse failure"
    );
}

// The concurrent instance/definition-save interleave test lives in the sibling
// `storage_interleave_tests.rs` — a child of this `tests` module so it reuses
// the `FakeCombinedStore` and `record_with_pubkey_and_key` helpers above
// without duplication, while keeping this file under the desktop file-size
// gate. Unix-only + `system-keyring`: it drives `libc::flock` and the real
// cross-process transaction lock.
#[cfg(all(unix, feature = "system-keyring"))]
#[path = "storage_interleave_tests.rs"]
mod interleave;
