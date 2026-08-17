//! Unit tests for `managed_agents/custom_harnesses.rs`.
//!
//! Kept in a sibling file so `custom_harnesses.rs` stays closer to the
//! 1000-line gate; `#[path]`-included from there.

use super::*;
use std::fs;

// ── ID validation ────────────────────────────────────────────────────────

#[test]
fn valid_id_lowercase_with_hyphen() {
    assert!(is_valid_harness_id("my-agent"));
}

#[test]
fn valid_id_underscore_start() {
    assert!(is_valid_harness_id("_my_agent"));
}

#[test]
fn valid_id_alphanumeric() {
    assert!(is_valid_harness_id("agent42"));
}

#[test]
fn invalid_id_uppercase() {
    assert!(!is_valid_harness_id("MyAgent"));
}

#[test]
fn invalid_id_starts_with_hyphen() {
    assert!(!is_valid_harness_id("-bad-id"));
}

#[test]
fn invalid_id_empty() {
    assert!(!is_valid_harness_id(""));
}

#[test]
fn invalid_id_path_traversal() {
    assert!(!is_valid_harness_id("../etc/passwd"));
}

// ── Collision check ──────────────────────────────────────────────────────

#[test]
fn builtin_ids_are_rejected() {
    // Tier-1 hard-coded IDs must always be reserved.
    for id in &["goose", "claude", "codex", "buzz-agent"] {
        assert!(check_id_collision(id).is_err(), "{id} should be rejected");
    }
    // Tier-2 preset IDs must also be reserved (derived from PRESET_HARNESSES).
    for id in crate::managed_agents::discovery::preset_harness_ids() {
        assert!(check_id_collision(id).is_err(), "{id} should be rejected");
    }
}

#[test]
fn unknown_id_passes_collision_check() {
    assert!(check_id_collision("my-custom-agent").is_ok());
}

// ── File loading ─────────────────────────────────────────────────────────

#[test]
fn load_valid_json_returns_definition() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("my-agent.json"),
        r#"{"id":"my-agent","label":"My Agent","command":"my-agent-bin"}"#,
    )
    .unwrap();

    let defs = load_custom_harnesses(dir.path());
    assert_eq!(defs.len(), 1);
    assert_eq!(defs[0].id, "my-agent");
    assert_eq!(defs[0].label, "My Agent");
    assert_eq!(defs[0].command, "my-agent-bin");
}

#[test]
fn load_skips_non_json_files() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("my-agent.toml"), r#"id = "my-agent""#).unwrap();

    let defs = load_custom_harnesses(dir.path());
    assert_eq!(defs.len(), 0, "non-JSON file should be ignored");
}

#[test]
fn load_skips_invalid_json_without_panicking() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("bad.json"), "{ not valid json").unwrap();

    // Must not panic or propagate an error.
    let defs = load_custom_harnesses(dir.path());
    assert_eq!(defs.len(), 0);
}

#[test]
fn load_skips_definition_with_invalid_id() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("Bad.json"),
        r#"{"id":"Bad-Id","label":"Bad","command":"bad"}"#,
    )
    .unwrap();

    let defs = load_custom_harnesses(dir.path());
    assert_eq!(
        defs.len(),
        0,
        "invalid id should cause the entry to be skipped"
    );
}

#[test]
fn load_skips_definition_with_empty_command() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("empty-cmd.json"),
        r#"{"id":"empty-cmd","label":"Empty","command":""}"#,
    )
    .unwrap();

    let defs = load_custom_harnesses(dir.path());
    assert_eq!(
        defs.len(),
        0,
        "empty command should cause the entry to be skipped"
    );
}

#[test]
fn load_skips_definition_with_non_http_install_url() {
    // installInstructionsUrl must start with https:// or http://.
    // A bare path, javascript: URI, or other scheme is rejected.
    let dir = tempfile::tempdir().unwrap();
    fs::write(
            dir.path().join("bad-url.json"),
            r#"{"id":"bad-url","label":"Bad","command":"bad-bin","installInstructionsUrl":"file:///etc/passwd"}"#,
        )
        .unwrap();

    let defs = load_custom_harnesses(dir.path());
    assert_eq!(
        defs.len(),
        0,
        "non-http install URL should cause the entry to be skipped"
    );
}

#[test]
fn load_accepts_empty_or_https_install_url() {
    let dir = tempfile::tempdir().unwrap();
    // Empty URL is fine (optional field).
    fs::write(
        dir.path().join("no-url.json"),
        r#"{"id":"no-url","label":"No URL","command":"no-url-bin"}"#,
    )
    .unwrap();
    // https:// is accepted.
    fs::write(
            dir.path().join("good-url.json"),
            r#"{"id":"good-url","label":"Good URL","command":"good-bin","installInstructionsUrl":"https://example.com/install"}"#,
        )
        .unwrap();

    let defs = load_custom_harnesses(dir.path());
    assert_eq!(
        defs.len(),
        2,
        "empty and https:// URLs must both be accepted"
    );
}

#[test]
fn load_missing_dir_returns_empty_vec() {
    let dir = tempfile::tempdir().unwrap();
    let nonexistent = dir.path().join("does_not_exist");

    let defs = load_custom_harnesses(&nonexistent);
    assert_eq!(defs.len(), 0);
}

#[test]
fn load_continues_after_one_bad_entry() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("bad.json"), "!!!").unwrap();
    fs::write(
        dir.path().join("good.json"),
        r#"{"id":"good-one","label":"Good","command":"good-binary"}"#,
    )
    .unwrap();

    let defs = load_custom_harnesses(dir.path());
    assert_eq!(defs.len(), 1, "bad entry skipped, good entry loaded");
    assert_eq!(defs[0].id, "good-one");
}

#[test]
fn load_applies_id_collision_check() {
    // A custom file whose id shadows a built-in ("goose") must be dropped
    // BY THE LOADER — `load_custom_harnesses` is the enforcement boundary
    // shared by both the warm path and discovery. This exercises the real
    // loader against a real file, not just the helper predicate.
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("goose.json"),
        r#"{"id":"goose","label":"Not Goose","command":"goose","args":["--evil"]}"#,
    )
    .unwrap();
    assert!(
        load_custom_harnesses(dir.path()).is_empty(),
        "loader must drop a file shadowing a builtin id"
    );
    assert!(check_id_collision("goose").is_err());
    assert!(check_id_collision("custom-goose").is_ok());
}

#[test]
fn load_dedups_duplicate_ids_first_file_wins() {
    // Two files carrying the same custom id: the loader must keep exactly
    // one definition (directory-order first wins; the duplicate is dropped).
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("a.json"),
        r#"{"id":"custom-dup","label":"First","command":"first-cmd"}"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("b.json"),
        r#"{"id":"custom-dup","label":"Second","command":"second-cmd"}"#,
    )
    .unwrap();
    let loaded = load_custom_harnesses(dir.path());
    assert_eq!(
        loaded.len(),
        1,
        "loader must dedup duplicate ids within the directory"
    );
    assert_eq!(loaded[0].id, "custom-dup");
}

// ── Round-trip via save_custom_harness_to_dir (B-4) ─────────────────────
//
// These tests exercise the REAL persistence helper, not raw fs::write.
// They prove: create, same-ID edit (backup-swap), rename (old file removed),
// backup file cleaned up on success.

fn make_def(id: &str, label: &str) -> HarnessDefinition {
    HarnessDefinition {
        id: id.to_string(),
        label: label.to_string(),
        command: format!("{id}-bin"),
        args: vec![],
        env: BTreeMap::new(),
        env_ref: None,
        env_unavailable: false,
        install_instructions_url: String::new(),
        install_hint: String::new(),
    }
}

#[test]
fn save_to_dir_create_writes_file_and_loads_back() {
    let dir = tempfile::tempdir().unwrap();
    let def = make_def("my-harness", "My Harness");

    let outcome = save_custom_harness_to_dir(dir.path(), &def, None).unwrap();

    assert_eq!(outcome.target_path, dir.path().join("my-harness.json"));
    assert!(outcome.removed_old_path.is_none(), "no old file on create");

    let loaded = load_custom_harnesses(dir.path());
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "my-harness");
    assert_eq!(loaded[0].label, "My Harness");
}

#[test]
fn save_to_dir_same_id_edit_replaces_content() {
    let dir = tempfile::tempdir().unwrap();
    let v1 = make_def("my-harness", "V1 Label");
    save_custom_harness_to_dir(dir.path(), &v1, None).unwrap();

    // Same-ID edit: label changes.
    let v2 = make_def("my-harness", "V2 Label");
    let outcome = save_custom_harness_to_dir(dir.path(), &v2, None).unwrap();

    // No old-path reported (id unchanged).
    assert!(outcome.removed_old_path.is_none());

    let loaded = load_custom_harnesses(dir.path());
    assert_eq!(loaded.len(), 1, "same-id edit must not duplicate entries");
    assert_eq!(loaded[0].label, "V2 Label", "v2 content must be present");
}

#[test]
fn save_to_dir_backup_is_cleaned_up_after_same_id_edit() {
    let dir = tempfile::tempdir().unwrap();
    let v1 = make_def("my-harness", "V1");
    save_custom_harness_to_dir(dir.path(), &v1, None).unwrap();

    let v2 = make_def("my-harness", "V2");
    save_custom_harness_to_dir(dir.path(), &v2, None).unwrap();

    // .bak file must be gone after a successful commit.
    let bak = dir.path().join("my-harness.json.bak");
    assert!(
        !bak.exists(),
        ".bak file must be removed after successful same-id edit"
    );
}

#[test]
fn save_to_dir_rename_removes_old_file_and_creates_new() {
    let dir = tempfile::tempdir().unwrap();
    let old_def = make_def("old-id", "Old");
    save_custom_harness_to_dir(dir.path(), &old_def, None).unwrap();

    // Rename: new id, old_id supplied.
    let new_def = make_def("new-id", "New");
    let outcome = save_custom_harness_to_dir(dir.path(), &new_def, Some("old-id")).unwrap();

    // The outcome carries the old path that was removed.
    let expected_old = dir.path().join("old-id.json");
    assert_eq!(
        outcome.removed_old_path,
        Some(expected_old.clone()),
        "removed_old_path must be the old file"
    );

    // Old file gone, new file present.
    assert!(!expected_old.exists(), "old-id.json must be removed");
    let loaded = load_custom_harnesses(dir.path());
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "new-id");
}

#[test]
fn save_to_dir_rename_nonexistent_old_id_is_non_fatal() {
    // rename_old_id pointing to a file that does not exist must succeed
    // (NotFound is silently ignored by the helper).
    let dir = tempfile::tempdir().unwrap();
    let def = make_def("alpha", "Alpha");
    let outcome = save_custom_harness_to_dir(dir.path(), &def, Some("ghost-id")).unwrap();

    // New file created, no old path removed.
    assert_eq!(outcome.target_path, dir.path().join("alpha.json"));
    assert!(
        outcome.removed_old_path.is_none(),
        "NotFound old-id must not be reported as removed"
    );
    assert!(load_custom_harnesses(dir.path()).len() == 1);
}

// ── Env secret projection (save strips → keyring; load hydrates) ─────────
//
// These drive the store-injected `*_with` seams against an in-memory fake so
// they are deterministic and never touch the live OS keyring (the default
// `system-keyring` feature makes the live store real under `cargo test`).
// The fake mirrors `secret_seam_tests::FakeProjectionStore`.

use crate::managed_agents::secret_projection::{deserialize_env_map, ProjectionStore};
use std::cell::RefCell;
use std::collections::HashMap;

/// In-memory projection store: every write succeeds and is recoverable.
struct FakeProjectionStore {
    data: RefCell<HashMap<String, String>>,
}

impl FakeProjectionStore {
    fn new() -> Self {
        Self {
            data: RefCell::new(HashMap::new()),
        }
    }
    fn len(&self) -> usize {
        self.data.borrow().len()
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

/// Projection store whose writes always fail — models a keyring outage that
/// forces the inline `0o600` fallback (`WriteOutcome::KeptInline`).
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

fn env_harness_def() -> HarnessDefinition {
    let mut env = BTreeMap::new();
    env.insert("ANTHROPIC_API_KEY".to_string(), "sk-ant-secret".to_string());
    HarnessDefinition {
        id: "env-harness".to_string(),
        label: "Env Harness".to_string(),
        command: "env-bin".to_string(),
        args: vec!["--flag".to_string()],
        env,
        env_ref: None,
        env_unavailable: false,
        install_instructions_url: "https://example.com".to_string(),
        install_hint: "Install from example.com".to_string(),
    }
}

#[test]
fn save_projects_env_to_keyring_and_leaves_no_plaintext_on_disk() {
    let store = FakeProjectionStore::new();
    let dir = tempfile::tempdir().unwrap();
    let def = env_harness_def();

    let outcome = save_custom_harness_to_dir_with(Some(&store), dir.path(), &def, None).unwrap();

    let raw = fs::read_to_string(&outcome.target_path).unwrap();
    assert!(
        !raw.contains("sk-ant-secret"),
        "the secret value must never be written to disk in plaintext"
    );
    // The on-disk record carries an env_ref and an empty inline env.
    let on_disk: HarnessDefinition = serde_json::from_str(&raw).unwrap();
    assert!(
        on_disk.env.is_empty(),
        "inline env must be stripped on disk"
    );
    let gen = on_disk
        .env_ref
        .expect("env_ref must point at the projected gen");
    // The keyring fake holds the env verbatim under the harness coordinate.
    let stored = store
        .load_key(&harness_env_key("env-harness", &gen))
        .unwrap()
        .expect("keyring must hold the projected env");
    assert_eq!(
        deserialize_env_map(&stored)
            .unwrap()
            .get("ANTHROPIC_API_KEY"),
        Some(&"sk-ant-secret".to_string())
    );
}

#[test]
fn save_then_load_with_same_fake_roundtrips_env_values() {
    let store = FakeProjectionStore::new();
    let dir = tempfile::tempdir().unwrap();
    let def = env_harness_def();

    save_custom_harness_to_dir_with(Some(&store), dir.path(), &def, None).unwrap();

    let loaded = load_custom_harnesses_with(Some(&store), dir.path());
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].args, vec!["--flag"]);
    assert_eq!(
        loaded[0].env.get("ANTHROPIC_API_KEY").map(String::as_str),
        Some("sk-ant-secret"),
        "env must hydrate back from the keyring on load"
    );
}

#[test]
fn save_does_not_mutate_the_caller_definition() {
    // The catalog entry the UI keeps after a save must retain the full env for
    // the edit round-trip — the seam strips a clone, never the caller's def.
    let store = FakeProjectionStore::new();
    let dir = tempfile::tempdir().unwrap();
    let def = env_harness_def();

    save_custom_harness_to_dir_with(Some(&store), dir.path(), &def, None).unwrap();

    assert_eq!(
        def.env.get("ANTHROPIC_API_KEY").map(String::as_str),
        Some("sk-ant-secret"),
        "caller's definition must keep its full env after save"
    );
    assert!(def.env_ref.is_none(), "caller's def must be untouched");
}

#[test]
fn save_empty_env_writes_no_ref_and_no_keyring_entry() {
    let store = FakeProjectionStore::new();
    let dir = tempfile::tempdir().unwrap();
    let def = make_def("bare", "Bare"); // env is empty

    let outcome = save_custom_harness_to_dir_with(Some(&store), dir.path(), &def, None).unwrap();

    assert_eq!(
        store.len(),
        0,
        "an empty env must not mint a keyring generation"
    );
    let on_disk: HarnessDefinition =
        serde_json::from_str(&fs::read_to_string(&outcome.target_path).unwrap()).unwrap();
    assert!(on_disk.env.is_empty());
    assert!(on_disk.env_ref.is_none(), "no env means no env_ref on disk");
}

#[test]
fn save_keyring_write_failure_keeps_env_inline_with_ref_cleared() {
    // A keyring outage must fall back to the inline `0o600` JSON so the harness
    // still resolves — with env_ref cleared (inline is authoritative).
    let store = FailingWriteStore;
    let dir = tempfile::tempdir().unwrap();
    let def = env_harness_def();

    let outcome = save_custom_harness_to_dir_with(Some(&store), dir.path(), &def, None).unwrap();

    let on_disk: HarnessDefinition =
        serde_json::from_str(&fs::read_to_string(&outcome.target_path).unwrap()).unwrap();
    assert_eq!(
        on_disk.env.get("ANTHROPIC_API_KEY").map(String::as_str),
        Some("sk-ant-secret"),
        "on a keyring write failure the env stays inline as the fallback"
    );
    assert!(
        on_disk.env_ref.is_none(),
        "the ref must be cleared so inline wins on the next hydrate"
    );
}

/// Inline env survives even without a store (keyless build): the env is written
/// inline to the `0o600` JSON and hydrates straight from disk.
#[test]
fn save_without_store_keeps_env_inline_and_roundtrips() {
    let dir = tempfile::tempdir().unwrap();
    let def = env_harness_def();

    save_custom_harness_to_dir_with::<FakeProjectionStore>(None, dir.path(), &def, None).unwrap();

    let loaded = load_custom_harnesses_with::<FakeProjectionStore>(None, dir.path());
    assert_eq!(loaded.len(), 1);
    assert_eq!(
        loaded[0].env.get("ANTHROPIC_API_KEY").map(String::as_str),
        Some("sk-ant-secret"),
        "keyless build must keep env inline and round-trip it"
    );
}

#[cfg(unix)]
#[test]
fn save_written_file_is_owner_only_0o600() {
    use std::os::unix::fs::PermissionsExt;
    let store = FakeProjectionStore::new();
    let dir = tempfile::tempdir().unwrap();
    let def = env_harness_def();

    let outcome = save_custom_harness_to_dir_with(Some(&store), dir.path(), &def, None).unwrap();

    let mode = fs::metadata(&outcome.target_path)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(
        mode & 0o777,
        0o600,
        "harness file must be created owner-only before any bytes hit disk"
    );
}

// `env_unavailable` marker tests (hydrate/save/rename) live in a sibling file
// so this module stays under the desktop file-size ratchet.
#[path = "env_unavailable_tests.rs"]
mod env_unavailable_tests;

// ── B-3: env validation boundary (validate_harness_definition_pub integration) ──

#[test]
fn validate_rejects_malformed_key_with_equals_sign() {
    // BUZZ_AUTH_TAG=x is the documented reserved-key bypass shape:
    // the key contains '=' so Command::env would produce
    // `BUZZ_AUTH_TAG=x=forged` in the child env.
    let mut env = BTreeMap::new();
    env.insert("BUZZ_AUTH_TAG=x".to_string(), "forged".to_string());
    let def = HarnessDefinition {
        id: "bad-env".to_string(),
        label: "Bad".to_string(),
        command: "bad-bin".to_string(),
        args: vec![],
        env,
        env_ref: None,
        env_unavailable: false,
        install_instructions_url: String::new(),
        install_hint: String::new(),
    };
    let err = validate_harness_definition_pub(&def).unwrap_err();
    assert!(
        err.contains("env var keys must match"),
        "malformed key must be rejected: {err}"
    );
    assert!(
        err.contains("BUZZ_AUTH_TAG"),
        "error must name the offending key: {err}"
    );
}

#[test]
fn validate_rejects_reserved_key_buzz_managed_agent() {
    // BUZZ_MANAGED_AGENT and BUZZ_MANAGED_AGENT_START_NONCE are the
    // ownership markers — supplying them in a definition must be rejected.
    let mut env = BTreeMap::new();
    env.insert(
        "BUZZ_MANAGED_AGENT".to_string(),
        "fake-instance".to_string(),
    );
    let def = HarnessDefinition {
        id: "bad-marker".to_string(),
        label: "Bad".to_string(),
        command: "bad-bin".to_string(),
        args: vec![],
        env,
        env_ref: None,
        env_unavailable: false,
        install_instructions_url: String::new(),
        install_hint: String::new(),
    };
    let err = validate_harness_definition_pub(&def).unwrap_err();
    assert!(
        err.contains("reserved by Buzz"),
        "ownership marker key must be rejected: {err}"
    );
}

#[test]
fn validate_rejects_reserved_key_case_insensitive() {
    // BUZZ_PRIVATE_KEY in any casing must be blocked.
    let mut env = BTreeMap::new();
    env.insert("buzz_private_key".to_string(), "secret".to_string());
    let def = HarnessDefinition {
        id: "ci-marker".to_string(),
        label: "CI".to_string(),
        command: "ci-bin".to_string(),
        args: vec![],
        env,
        env_ref: None,
        env_unavailable: false,
        install_instructions_url: String::new(),
        install_hint: String::new(),
    };
    let err = validate_harness_definition_pub(&def).unwrap_err();
    assert!(
        err.contains("reserved by Buzz"),
        "reserved key must be blocked case-insensitively: {err}"
    );
}

#[test]
fn validate_rejects_nul_byte_in_value() {
    // A NUL in a value would cause Command::env to panic at spawn time.
    let mut env = BTreeMap::new();
    env.insert("MY_KEY".to_string(), "val\x00ue".to_string());
    let def = HarnessDefinition {
        id: "nul-val".to_string(),
        label: "NUL".to_string(),
        command: "nul-bin".to_string(),
        args: vec![],
        env,
        env_ref: None,
        env_unavailable: false,
        install_instructions_url: String::new(),
        install_hint: String::new(),
    };
    let err = validate_harness_definition_pub(&def).unwrap_err();
    assert!(
        err.contains("NUL bytes"),
        "NUL value must be rejected at validation: {err}"
    );
}

#[test]
fn validate_rejects_value_over_per_value_size_limit() {
    use crate::managed_agents::env_vars::MAX_ENV_VALUE_BYTES;
    let mut env = BTreeMap::new();
    // One byte over the per-value cap.
    env.insert("BIG_VAL".to_string(), "x".repeat(MAX_ENV_VALUE_BYTES + 1));
    let def = HarnessDefinition {
        id: "big-val".to_string(),
        label: "Big".to_string(),
        command: "big-bin".to_string(),
        args: vec![],
        env,
        env_ref: None,
        env_unavailable: false,
        install_instructions_url: String::new(),
        install_hint: String::new(),
    };
    let err = validate_harness_definition_pub(&def).unwrap_err();
    assert!(
        err.contains("per-value limit"),
        "oversized value must be rejected: {err}"
    );
}

#[test]
fn validate_accepts_well_formed_env() {
    let mut env = BTreeMap::new();
    env.insert("ANTHROPIC_API_KEY".to_string(), "sk-test-123".to_string());
    env.insert("MODEL_VERSION".to_string(), "claude-3".to_string());
    let def = HarnessDefinition {
        id: "good-env".to_string(),
        label: "Good".to_string(),
        command: "good-bin".to_string(),
        args: vec![],
        env,
        env_ref: None,
        env_unavailable: false,
        install_instructions_url: String::new(),
        install_hint: String::new(),
    };
    assert!(
        validate_harness_definition_pub(&def).is_ok(),
        "well-formed definition must pass validation"
    );
}

// ── Comma-in-args validation (transport-lossiness guard) ─────────────────

/// A definition whose args contain a literal comma must be rejected at the
/// validation boundary — the comma-delimited `BUZZ_ACP_AGENT_ARGS`
/// transport would silently split it into two args at spawn time.
#[test]
fn validate_rejects_comma_in_args() {
    let mut def = make_def("comma-args", "Comma");
    def.args = vec!["--name".to_string(), "a,b".to_string()];
    let err = validate_harness_definition_pub(&def).unwrap_err();
    assert!(
        err.contains("comma"),
        "error must explain the comma transport limit, got: {err}"
    );
}

/// Comma-free args pass — including args with spaces and special chars.
#[test]
fn validate_accepts_comma_free_args() {
    let mut def = make_def("clean-args", "Clean");
    def.args = vec!["acp".to_string(), "--flag=x y".to_string()];
    assert!(validate_harness_definition_pub(&def).is_ok());
}

/// The loader shares the same validator: a hand-authored file with a comma
/// in args is skipped, so the invariant holds regardless of how the
/// definition arrives (UI save or hand-edited JSON).
#[test]
fn load_skips_definition_with_comma_in_args() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("comma.json"),
        r#"{"id":"comma-file","label":"Comma","command":"comma-bin","args":["a,b"]}"#,
    )
    .unwrap();
    assert!(
        load_custom_harnesses(dir.path()).is_empty(),
        "comma-in-args definition must be skipped at the loader boundary"
    );
}

// ── Discovery publish under persist_mutex (stale-snapshot regression) ────

/// Discovery's registry publish must re-read the directory at publish time
/// (under the persist mutex), not push a snapshot taken before the auth
/// probes ran. Regression shape: discovery scans dir → user saves harness X
/// (save_and_warm warms the registry with X) → discovery finishes. If
/// discovery published its pre-save snapshot, X would be on disk but
/// unresolvable at spawn until the next discover.
///
/// Deterministic interleaving: we simulate it by calling the publish seam
/// (`warm_harness_registry_locked`) after a save that happened "during"
/// discovery — the fresh-read semantics mean the just-saved definition
/// survives the publish.
#[test]
fn discovery_publish_after_concurrent_save_keeps_saved_harness() {
    let _lock = registry_test_lock();
    let dir = tempfile::tempdir().unwrap();

    // Discovery "scans" the dir while it is empty (stale snapshot would be []).
    let stale_snapshot = load_custom_harnesses(dir.path());
    assert!(stale_snapshot.is_empty());

    // A save lands mid-discovery (save_and_warm: write + warm).
    let def = make_def("mid-save", "Mid Save");
    save_and_warm(dir.path(), &def, None).unwrap();
    assert!(lookup_loaded_harness_by_id("mid-save").is_some());

    // Discovery publishes — the locked warm re-reads the directory, so the
    // just-saved harness must survive (a stale-snapshot publish would
    // clobber it).
    warm_harness_registry_locked(Some(dir.path()));
    assert!(
        lookup_loaded_harness_by_id("mid-save").is_some(),
        "publish must re-read the directory, not clobber the mid-discovery save"
    );
}

/// Same shape for delete: a delete landing mid-discovery must not be
/// resurrected by the discovery publish.
#[test]
fn discovery_publish_after_concurrent_delete_keeps_harness_gone() {
    let _lock = registry_test_lock();
    let dir = tempfile::tempdir().unwrap();

    let def = make_def("mid-delete", "Mid Delete");
    save_and_warm(dir.path(), &def, None).unwrap();

    // Discovery "scans" while the file exists (stale snapshot would contain it).
    let stale_snapshot = load_custom_harnesses(dir.path());
    assert_eq!(stale_snapshot.len(), 1);

    // Delete lands mid-discovery.
    delete_and_warm(dir.path(), "mid-delete").unwrap();
    assert!(lookup_loaded_harness_by_id("mid-delete").is_none());

    // Discovery publishes — fresh read keeps it gone.
    warm_harness_registry_locked(Some(dir.path()));
    assert!(
        lookup_loaded_harness_by_id("mid-delete").is_none(),
        "publish must not resurrect a harness deleted mid-discovery"
    );
}

// ── Registry warm path ───────────────────────────────────────────────────

/// After `warm_harness_registry_from_dir` the registry contains preset +
/// custom definitions and `lookup_loaded_harness_by_id` resolves them.
#[test]
fn warm_registry_then_lookup_finds_custom_and_preset_entries() {
    let _lock = registry_test_lock();
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("my-custom.json"),
        r#"{"id":"my-custom","label":"My Custom","command":"my-custom-bin"}"#,
    )
    .unwrap();

    warm_harness_registry_from_dir(Some(dir.path()));

    // Custom entry must be findable.
    let found = lookup_loaded_harness_by_id("my-custom");
    assert!(
        found.is_some(),
        "warm registry must contain the custom entry"
    );
    assert_eq!(found.unwrap().command, "my-custom-bin");

    // At least one preset entry must be in the registry (e.g. "cursor").
    let preset = lookup_loaded_harness_by_id("cursor");
    assert!(
        preset.is_some(),
        "warm registry must contain preset entries"
    );
}

/// `warm_harness_registry_from_dir` with `None` still loads presets.
#[test]
fn warm_registry_with_no_custom_dir_loads_presets_only() {
    let _lock = registry_test_lock();
    warm_harness_registry_from_dir(None);
    // At least the "cursor" preset must be present.
    assert!(
        lookup_loaded_harness_by_id("cursor").is_some(),
        "presets must be reachable even without a custom dir"
    );
}

/// `warm_harness_registry_from_dir` followed by `update_loaded_harness_registry`
/// with an empty slice clears the registry (transactional save/delete contract).
#[test]
fn warm_then_clear_registry_empties_lookup() {
    let _lock = registry_test_lock();
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("tmp-agent.json"),
        r#"{"id":"tmp-agent","label":"Tmp","command":"tmp-bin"}"#,
    )
    .unwrap();

    warm_harness_registry_from_dir(Some(dir.path()));
    assert!(lookup_loaded_harness_by_id("tmp-agent").is_some());

    // Simulate delete — re-warm with empty dir.
    let empty_dir = tempfile::tempdir().unwrap();
    warm_harness_registry_from_dir(Some(empty_dir.path()));
    assert!(
        lookup_loaded_harness_by_id("tmp-agent").is_none(),
        "deleted harness must not appear after re-warm"
    );
}

// ── Legacy avatarUrl regression (F1) ─────────────────────────────────────

/// A JSON file that contains a legacy `avatarUrl` field (from pre-BYOH code)
/// must still deserialize without error (unknown-field handling) and the
/// loaded `HarnessDefinition` must NOT carry the URL — the field is absent
/// from the struct so serde drops it.
#[test]
fn legacy_avatar_url_in_json_is_silently_dropped_on_load() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("legacy.json"),
        r#"{
                "id": "legacy-agent",
                "label": "Legacy Agent",
                "command": "legacy-bin",
                "avatarUrl": "https://tracking.example.com/logo.png"
            }"#,
    )
    .unwrap();

    let defs = load_custom_harnesses(dir.path());
    // The file must deserialize successfully (serde ignores unknown fields).
    assert_eq!(defs.len(), 1, "legacy file with avatarUrl must still load");
    assert_eq!(defs[0].id, "legacy-agent");
    // HarnessDefinition has no avatar_url field — prove the URL cannot
    // be routed to a catalog entry by serializing back and checking.
    let json = serde_json::to_string(&defs[0]).unwrap();
    assert!(
        !json.contains("https://tracking.example.com"),
        "serialized HarnessDefinition must not contain the legacy avatar URL"
    );
}

// ── Preset id reservation ────────────────────────────────────────────────

/// All preset ids must be blocked by `check_id_collision`.
#[test]
fn preset_ids_are_reserved_and_cannot_be_used_as_custom_ids() {
    // Derived from PRESET_HARNESSES — no hard-coded copy here so this test
    // automatically covers any future preset additions.
    for id in crate::managed_agents::discovery::preset_harness_ids() {
        assert!(
            check_id_collision(id).is_err(),
            "preset id {id:?} should be rejected by check_id_collision"
        );
    }
}
