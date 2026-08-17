//! Coverage for [`write_secrets_batched`] — the metadata-save fast path that
//! reuses a live generation when a field's bytes are unchanged and commits
//! every changed field in one blob mutation.
//!
//! Split out of `secret_projection_tests.rs` so each test file stays under the
//! desktop file-size ratchet; both share the `secret_projection` module via
//! `use super::*`.

use super::*;
use std::cell::RefCell;

/// A store that counts blob mutations (`store_batch`) and per-key writes
/// (`write_and_verify`) so tests can assert the exact I/O a save performs.
struct CountingStore {
    data: RefCell<HashMap<String, String>>,
    batch_mutations: std::cell::Cell<usize>,
    single_writes: std::cell::Cell<usize>,
}

impl CountingStore {
    fn new() -> Self {
        Self {
            data: RefCell::new(HashMap::new()),
            batch_mutations: std::cell::Cell::new(0),
            single_writes: std::cell::Cell::new(0),
        }
    }
    fn with_entry(self, key: &str, value: &str) -> Self {
        self.data
            .borrow_mut()
            .insert(key.to_string(), value.to_string());
        self
    }
    fn get(&self, key: &str) -> Option<String> {
        self.data.borrow().get(key).cloned()
    }
    fn keys(&self) -> Vec<String> {
        self.data.borrow().keys().cloned().collect()
    }
}

impl ProjectionStore for CountingStore {
    fn write_and_verify(&self, key: &str, value: &str) -> Result<(), String> {
        self.single_writes.set(self.single_writes.get() + 1);
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
        self.batch_mutations.set(self.batch_mutations.get() + 1);
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

/// A store whose blob write always fails — a denied keychain prompt or a
/// transient outage — so the batched save must fail closed to KeptInline.
struct FailingBatchStore;

impl ProjectionStore for FailingBatchStore {
    fn write_and_verify(&self, _key: &str, _value: &str) -> Result<(), String> {
        Err("write failed".to_string())
    }
    fn load_key(&self, _key: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn load_all(&self) -> Result<Option<HashMap<String, String>>, String> {
        Ok(None)
    }
    fn store_batch(&self, _entries: &HashMap<String, String>) -> Result<(), String> {
        Err("blob write failed".to_string())
    }
    fn remove_batch(&self, _keys: &[&str]) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn test_write_secrets_batched_reuses_gen_when_bytes_unchanged() {
    // A field whose live ref already stores these exact bytes must keep its
    // generation and perform NO write — the metadata-only-save fast path.
    let store = CountingStore::new().with_entry(&agent_env_key("abc", "gen1"), r#"{"K":"v"}"#);
    let env_key = |gen: &str| agent_env_key("abc", gen);
    let outcomes = write_secrets_batched(
        &store,
        &[FieldSave {
            coord_key_fn: &env_key,
            value: Some(r#"{"K":"v"}"#),
            existing_ref: Some("gen1"),
            context: "agent:abc env_vars",
        }],
    );
    assert_eq!(
        outcomes[0],
        WriteOutcome::Persisted {
            gen: "gen1".to_string()
        },
        "unchanged bytes must reuse the existing generation"
    );
    assert_eq!(
        store.batch_mutations.get(),
        0,
        "no blob mutation for an unchanged field"
    );
    assert_eq!(store.keys().len(), 1, "no new generation key was created");
}

#[test]
fn test_write_secrets_batched_new_gen_when_bytes_changed() {
    let store = CountingStore::new().with_entry(&agent_env_key("abc", "gen1"), r#"{"K":"old"}"#);
    let env_key = |gen: &str| agent_env_key("abc", gen);
    let outcomes = write_secrets_batched(
        &store,
        &[FieldSave {
            coord_key_fn: &env_key,
            value: Some(r#"{"K":"new"}"#),
            existing_ref: Some("gen1"),
            context: "agent:abc env_vars",
        }],
    );
    let new_gen = match &outcomes[0] {
        WriteOutcome::Persisted { gen } => gen.clone(),
        other => panic!("expected Persisted, got {other:?}"),
    };
    assert_ne!(
        new_gen, "gen1",
        "changed bytes must mint a fresh generation"
    );
    assert_eq!(store.batch_mutations.get(), 1);
    assert_eq!(
        store.get(&agent_env_key("abc", "gen1")),
        Some(r#"{"K":"old"}"#.to_string()),
        "the old generation must survive — a failed JSON commit still needs it"
    );
    assert_eq!(
        store.get(&agent_env_key("abc", &new_gen)),
        Some(r#"{"K":"new"}"#.to_string())
    );
}

#[test]
fn test_write_secrets_batched_single_mutation_for_multiple_changed_fields() {
    // Three changed fields across three namespaces must commit in EXACTLY one
    // blob mutation and zero per-key writes.
    let store = CountingStore::new();
    let env_key = |gen: &str| agent_env_key("abc", gen);
    let auth_key = |gen: &str| agent_auth_tag_key("abc", gen);
    let pc_key = |gen: &str| agent_provider_config_key("abc", gen);
    let outcomes = write_secrets_batched(
        &store,
        &[
            FieldSave {
                coord_key_fn: &env_key,
                value: Some(r#"{"K":"v"}"#),
                existing_ref: None,
                context: "agent:abc env_vars",
            },
            FieldSave {
                coord_key_fn: &auth_key,
                value: Some("auth-tag"),
                existing_ref: None,
                context: "agent:abc auth_tag",
            },
            FieldSave {
                coord_key_fn: &pc_key,
                value: Some(r#"{"host":"x"}"#),
                existing_ref: None,
                context: "agent:abc provider_config",
            },
        ],
    );
    assert!(
        outcomes
            .iter()
            .all(|o| matches!(o, WriteOutcome::Persisted { .. })),
        "all three fields persist"
    );
    assert_eq!(
        store.batch_mutations.get(),
        1,
        "three changed fields must be ONE blob mutation, not three"
    );
    assert_eq!(
        store.single_writes.get(),
        0,
        "the batched path must not fall back to per-key writes"
    );
    assert_eq!(store.keys().len(), 3, "one generation per field");
}

#[test]
fn test_write_secrets_batched_mixed_reuse_and_change_is_one_mutation() {
    // One unchanged field (reuse, no write) + one changed field (new gen):
    // still exactly one mutation, and the unchanged field keeps its gen.
    let store = CountingStore::new()
        .with_entry(&agent_env_key("abc", "gen_env"), r#"{"E":"same"}"#)
        .with_entry(&agent_auth_tag_key("abc", "gen_auth"), "old-auth");
    let env_key = |gen: &str| agent_env_key("abc", gen);
    let auth_key = |gen: &str| agent_auth_tag_key("abc", gen);
    let outcomes = write_secrets_batched(
        &store,
        &[
            FieldSave {
                coord_key_fn: &env_key,
                value: Some(r#"{"E":"same"}"#),
                existing_ref: Some("gen_env"),
                context: "agent:abc env_vars",
            },
            FieldSave {
                coord_key_fn: &auth_key,
                value: Some("new-auth"),
                existing_ref: Some("gen_auth"),
                context: "agent:abc auth_tag",
            },
        ],
    );
    assert_eq!(
        outcomes[0],
        WriteOutcome::Persisted {
            gen: "gen_env".to_string()
        },
        "unchanged env reuses its gen"
    );
    let new_auth = match &outcomes[1] {
        WriteOutcome::Persisted { gen } => gen.clone(),
        other => panic!("expected Persisted, got {other:?}"),
    };
    assert_ne!(new_auth, "gen_auth");
    assert_eq!(
        store.batch_mutations.get(),
        1,
        "only the changed field triggers the single mutation"
    );
}

#[test]
fn test_write_secrets_batched_nothing_on_empty_value() {
    let store = CountingStore::new();
    let env_key = |gen: &str| agent_env_key("abc", gen);
    let outcomes = write_secrets_batched(
        &store,
        &[FieldSave {
            coord_key_fn: &env_key,
            value: None,
            existing_ref: None,
            context: "agent:abc env_vars",
        }],
    );
    assert_eq!(outcomes[0], WriteOutcome::Nothing);
    assert_eq!(
        store.batch_mutations.get(),
        0,
        "an empty field writes nothing"
    );
}

#[test]
fn test_write_secrets_batched_all_kept_inline_on_write_failure() {
    // The single blob write is atomic: if it fails, every staged field becomes
    // KeptInline together — no torn partial state where some fields persisted.
    let store = FailingBatchStore;
    let env_key = |gen: &str| agent_env_key("abc", gen);
    let auth_key = |gen: &str| agent_auth_tag_key("abc", gen);
    let outcomes = write_secrets_batched(
        &store,
        &[
            FieldSave {
                coord_key_fn: &env_key,
                value: Some(r#"{"K":"v"}"#),
                existing_ref: None,
                context: "agent:abc env_vars",
            },
            FieldSave {
                coord_key_fn: &auth_key,
                value: Some("auth"),
                existing_ref: None,
                context: "agent:abc auth_tag",
            },
        ],
    );
    assert!(
        outcomes
            .iter()
            .all(|o| matches!(o, WriteOutcome::KeptInline { .. })),
        "a failed atomic write keeps ALL staged fields inline: {outcomes:?}"
    );
}

#[test]
fn test_write_secrets_batched_changes_when_prior_gen_unreadable() {
    // existing_ref points at a gen whose value is absent (outage/deleted). Reuse
    // must NOT fire on an unverified generation — fall through to a fresh write.
    let store = CountingStore::new();
    let env_key = |gen: &str| agent_env_key("abc", gen);
    let outcomes = write_secrets_batched(
        &store,
        &[FieldSave {
            coord_key_fn: &env_key,
            value: Some(r#"{"K":"v"}"#),
            existing_ref: Some("gen_missing"),
            context: "agent:abc env_vars",
        }],
    );
    match &outcomes[0] {
        WriteOutcome::Persisted { gen } => assert_ne!(gen, "gen_missing"),
        other => panic!("expected fresh Persisted, got {other:?}"),
    }
    assert_eq!(store.batch_mutations.get(), 1);
}

#[test]
fn test_store_batch_verified_default_catches_silent_write_loss() {
    // A store whose store_batch is a no-op (acknowledges without persisting)
    // must be caught by the verify pass, not reported as success.
    struct LyingStore;
    impl ProjectionStore for LyingStore {
        fn write_and_verify(&self, _k: &str, _v: &str) -> Result<(), String> {
            Ok(())
        }
        fn load_key(&self, _k: &str) -> Result<Option<String>, String> {
            Ok(None) // nothing was actually stored
        }
        fn load_all(&self) -> Result<Option<HashMap<String, String>>, String> {
            Ok(None)
        }
        fn store_batch(&self, _entries: &HashMap<String, String>) -> Result<(), String> {
            Ok(()) // lies: acknowledges but persists nothing
        }
        fn remove_batch(&self, _keys: &[&str]) -> Result<(), String> {
            Ok(())
        }
    }
    let mut entries = HashMap::new();
    entries.insert("global:env:gen1".to_string(), "sk-secret".to_string());
    assert!(
        LyingStore.store_batch_verified(&entries).is_err(),
        "verify must reject a write the backend did not persist"
    );
}
