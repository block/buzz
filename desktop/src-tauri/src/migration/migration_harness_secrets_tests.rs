//! Tests for boot-time custom-harness secret extraction.
//!
//! The module under test is generic over [`ProjectionStore`], so the full
//! extract → verify → scrub flow runs against a fake keyring and a tempdir
//! with no `AppHandle` — the precedent `migration_secrets` could only unit-test
//! its helpers because its extraction was not store-injected.
//!
//! Load-bearing properties proven here:
//!   * inline `env` is projected to the keyring and stripped from disk (no
//!     plaintext survives), recoverable through the same coordinate hydrate reads;
//!   * the plaintext-artifact scrub runs ONLY when every projected ref reads
//!     back cleanly — a dangling ref preserves the backup as last-recoverable
//!     plaintext, and a later boot scrubs it once the ref resolves;
//!   * extraction is idempotent across launches (a re-read projected record
//!     mints no new generation).

use super::*;
use crate::managed_agents::secret_projection::{deserialize_env_map, serialize_env_map};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

// ── Fake store (mirrors secret_seam_tests::FakeProjectionStore) ────────────

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
    fn insert(&self, key: &str, value: &str) {
        self.data
            .borrow_mut()
            .insert(key.to_string(), value.to_string());
    }
    fn contains(&self, key: &str) -> bool {
        self.data.borrow().contains_key(key)
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

// ── Builders ───────────────────────────────────────────────────────────────

fn env_of(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn harness_def(
    id: &str,
    env: BTreeMap<String, String>,
    env_ref: Option<String>,
) -> HarnessDefinition {
    HarnessDefinition {
        id: id.to_string(),
        label: format!("{id} label"),
        command: format!("{id}-bin"),
        args: vec![],
        env,
        env_ref,
        env_unavailable: false,
        install_instructions_url: String::new(),
        install_hint: String::new(),
    }
}

/// Write `def` to `dir/<file_stem>.json` and return the path. The stem defaults
/// to the id but may differ to exercise hand-authored filenames.
fn write_harness_file(dir: &Path, file_stem: &str, def: &HarnessDefinition) -> PathBuf {
    let path = dir.join(format!("{file_stem}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(def).unwrap()).unwrap();
    path
}

fn read_back(path: &Path) -> HarnessDefinition {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

// ── Phase 1: extraction ──────────────────────────────────────────────────────

#[test]
fn test_inline_env_projected_and_file_stripped_of_plaintext() {
    let store = FakeProjectionStore::new();
    let dir = tempfile::tempdir().unwrap();
    let env = env_of(&[("ANTHROPIC_API_KEY", "sk-ant-secret")]);
    let path = write_harness_file(
        dir.path(),
        "env-harness",
        &harness_def("env-harness", env.clone(), None),
    );

    migrate_harness_secrets_in_dir(&store, dir.path());

    // Disk: env stripped, ref set, no plaintext byte left behind.
    let on_disk = read_back(&path);
    assert!(
        on_disk.env.is_empty(),
        "inline env must be stripped from disk"
    );
    let gen = on_disk
        .env_ref
        .expect("env_ref must be set after projection");
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(
        !raw.contains("sk-ant-secret"),
        "plaintext secret must not survive on disk"
    );

    // Keyring: the coordinate hydrate reads holds the projected env verbatim.
    let key = harness_env_key("env-harness", &gen);
    let stored = store
        .load_key(&key)
        .unwrap()
        .expect("keyring must hold the projected env");
    assert_eq!(deserialize_env_map(&stored).unwrap(), env);
}

#[test]
fn test_empty_env_is_noop_no_projection_no_rewrite() {
    let store = FakeProjectionStore::new();
    let dir = tempfile::tempdir().unwrap();
    let path = write_harness_file(
        dir.path(),
        "bare",
        &harness_def("bare", BTreeMap::new(), None),
    );
    let before = std::fs::read_to_string(&path).unwrap();

    migrate_harness_secrets_in_dir(&store, dir.path());

    assert_eq!(store.len(), 0, "empty env must not mint a generation");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        before,
        "a no-env harness file must be left byte-for-byte unchanged"
    );
}

#[test]
fn test_rewrites_original_path_never_id_derived_name() {
    // A hand-authored filename may differ from the harness id; relocating to
    // <id>.json would orphan the original and duplicate the entry.
    let store = FakeProjectionStore::new();
    let dir = tempfile::tempdir().unwrap();
    let env = env_of(&[("K", "v")]);
    let authored = write_harness_file(
        dir.path(),
        "hand-authored",
        &harness_def("realid", env, None),
    );

    migrate_harness_secrets_in_dir(&store, dir.path());

    assert!(
        authored.exists(),
        "the original hand-authored file must be rewritten in place"
    );
    assert!(
        !dir.path().join("realid.json").exists(),
        "migration must not relocate to an id-derived filename"
    );
    assert!(
        read_back(&authored).env_ref.is_some(),
        "the original file must carry the new ref"
    );
}

// ── Phase 2: plaintext-artifact scrub, gated on verified extraction ──────────

#[test]
fn test_parseable_backup_scrubbed_when_extraction_verified() {
    let store = FakeProjectionStore::new();
    let dir = tempfile::tempdir().unwrap();
    write_harness_file(
        dir.path(),
        "h",
        &harness_def("h", env_of(&[("K", "v")]), None),
    );
    // A backup left by the save path still carries plaintext env.
    let bak = dir.path().join("h.json.bak");
    std::fs::write(
        &bak,
        r#"{"id":"h","label":"h","command":"h-bin","env":{"ANTHROPIC_API_KEY":"sk-plaintext"}}"#,
    )
    .unwrap();

    migrate_harness_secrets_in_dir(&store, dir.path());

    assert!(
        bak.exists(),
        "a parseable backup must survive as a scrubbed file"
    );
    let scrubbed = std::fs::read_to_string(&bak).unwrap();
    assert!(
        !scrubbed.contains("sk-plaintext"),
        "backup plaintext env must be scrubbed"
    );
    assert!(
        scrubbed.contains("\"command\""),
        "non-secret backup fields must survive"
    );
}

#[test]
fn test_unparseable_backup_deleted_when_verified() {
    let store = FakeProjectionStore::new();
    let dir = tempfile::tempdir().unwrap();
    write_harness_file(
        dir.path(),
        "h",
        &harness_def("h", env_of(&[("K", "v")]), None),
    );
    let bak = dir.path().join("h.json.bak");
    std::fs::write(&bak, b"not valid json holding sk-plaintext").unwrap();

    migrate_harness_secrets_in_dir(&store, dir.path());

    assert!(
        !bak.exists(),
        "an unparseable backup may hold plaintext and must be deleted"
    );
}

#[test]
fn test_atomic_write_temp_deleted_when_verified() {
    let store = FakeProjectionStore::new();
    let dir = tempfile::tempdir().unwrap();
    write_harness_file(
        dir.path(),
        "h",
        &harness_def("h", env_of(&[("K", "v")]), None),
    );
    let temp = dir.path().join("deadbeefcafe0123");
    std::fs::write(&temp, b"interrupted write with sk-plaintext").unwrap();

    migrate_harness_secrets_in_dir(&store, dir.path());

    assert!(
        !temp.exists(),
        "an atomic-write temp sibling must be deleted"
    );
}

#[test]
fn test_scrub_skipped_when_ref_unreachable_backup_survives_intact() {
    // A live file already projected on a prior boot (empty inline + ref) whose
    // keyring entry is now unreachable: the backup may be the last recoverable
    // plaintext, so it must NOT be scrubbed this boot.
    let store = FakeProjectionStore::new(); // does NOT hold the referenced gen
    let dir = tempfile::tempdir().unwrap();
    write_harness_file(
        dir.path(),
        "h",
        &harness_def("h", BTreeMap::new(), Some("lostgen".to_string())),
    );
    let bak = dir.path().join("h.json.bak");
    std::fs::write(
        &bak,
        r#"{"id":"h","label":"h","command":"h-bin","env":{"ANTHROPIC_API_KEY":"sk-plaintext"}}"#,
    )
    .unwrap();

    migrate_harness_secrets_in_dir(&store, dir.path());

    assert!(
        bak.exists(),
        "a dangling ref must leave the plaintext backup in place"
    );
    assert!(
        std::fs::read_to_string(&bak)
            .unwrap()
            .contains("sk-plaintext"),
        "the backup must retain its plaintext while the ref is unrecoverable"
    );
}

#[test]
fn test_scrub_retried_once_ref_becomes_reachable() {
    let store = FakeProjectionStore::new();
    let dir = tempfile::tempdir().unwrap();
    write_harness_file(
        dir.path(),
        "h",
        &harness_def("h", BTreeMap::new(), Some("g1".to_string())),
    );
    let bak = dir.path().join("h.json.bak");
    std::fs::write(
        &bak,
        r#"{"id":"h","label":"h","command":"h-bin","env":{"ANTHROPIC_API_KEY":"sk-plaintext"}}"#,
    )
    .unwrap();

    // Boot 1: keyring unreachable for g1 → scrub skipped, backup preserved.
    migrate_harness_secrets_in_dir(&store, dir.path());
    assert!(
        std::fs::read_to_string(&bak)
            .unwrap()
            .contains("sk-plaintext"),
        "scrub must be skipped while the ref is unreachable"
    );

    // Keyring recovers the entry; Boot 2: verify passes → scrub runs.
    store.insert(
        &harness_env_key("h", "g1"),
        &serialize_env_map(&env_of(&[("K", "v")])).unwrap(),
    );
    migrate_harness_secrets_in_dir(&store, dir.path());
    assert!(
        !std::fs::read_to_string(&bak)
            .unwrap()
            .contains("sk-plaintext"),
        "scrub must run on the boot where the ref finally resolves"
    );
}

// ── Idempotence ──────────────────────────────────────────────────────────────

#[test]
fn test_two_launches_mint_no_second_generation() {
    let store = FakeProjectionStore::new();
    let dir = tempfile::tempdir().unwrap();
    let path = write_harness_file(
        dir.path(),
        "h",
        &harness_def("h", env_of(&[("K", "v")]), None),
    );

    migrate_harness_secrets_in_dir(&store, dir.path());
    let gen1 = read_back(&path).env_ref.expect("gen after first launch");
    let keys_after_1 = store.len();

    migrate_harness_secrets_in_dir(&store, dir.path());
    let gen2 = read_back(&path).env_ref.expect("gen after second launch");

    assert_eq!(
        gen1, gen2,
        "a re-read projected record must keep its generation"
    );
    assert_eq!(
        store.len(),
        keys_after_1,
        "second launch must not mint a new generation"
    );
}

// ── Helper predicates and JSON strip ─────────────────────────────────────────

#[test]
fn test_is_harness_backup_matches_bak_variants() {
    assert!(is_harness_backup("h.json.bak"));
    assert!(is_harness_backup("h.json.bak-20260813-000000"));
    assert!(is_harness_backup("h.json.bak.20260813"));
    assert!(!is_harness_backup("h.json"));
    assert!(!is_harness_backup("h.json.tmp"));
}

#[test]
fn test_is_atomic_write_temp_matches_hex_names_only() {
    assert!(is_atomic_write_temp("deadbeefcafe0123"));
    assert!(!is_atomic_write_temp("h.json")); // has extension
    assert!(!is_atomic_write_temp("abc")); // too short
    assert!(!is_atomic_write_temp("not-hex-name")); // non-hex
}

#[test]
fn test_strip_env_from_json_removes_only_env() {
    let clean =
        strip_env_from_json(r#"{"id":"h","command":"h-bin","env":{"K":"secret"}}"#).unwrap();
    let v: serde_json::Value = serde_json::from_str(&clean).unwrap();
    assert!(v.get("env").is_none(), "env must be removed");
    assert_eq!(v.get("command").and_then(|c| c.as_str()), Some("h-bin"));
    assert_eq!(v.get("id").and_then(|c| c.as_str()), Some("h"));
}

#[test]
fn test_strip_env_from_json_returns_none_on_non_object() {
    assert!(strip_env_from_json("not json").is_none());
    assert!(strip_env_from_json("[1,2,3]").is_none());
}

#[test]
fn test_extraction_verified_true_when_no_projected_refs() {
    // A directory of purely inline-fallback (non-empty env, no ref) files has
    // nothing to hydrate, so verification passes and the scrub may proceed.
    let store = FakeProjectionStore::new();
    let dir = tempfile::tempdir().unwrap();
    write_harness_file(
        dir.path(),
        "a",
        &harness_def("a", env_of(&[("K", "v")]), None),
    );
    assert!(extraction_verified(&store, dir.path()));
}

#[cfg(unix)]
#[test]
fn test_cleanup_skips_symlink_escaping_dir() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("secret.txt");
    std::fs::write(&target, b"outside-secret").unwrap();

    // A backup-shaped symlink pointing outside the harness dir.
    symlink(&target, dir.path().join("escape.json.bak")).unwrap();

    cleanup_harness_artifacts(dir.path());

    assert!(
        target.exists(),
        "cleanup must not follow a symlink escaping the dir"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "outside-secret");
}

// A pre-populated fake keeps the with_entry builder exercised for future use.
#[test]
fn test_verify_passes_for_reachable_pre_projected_ref() {
    let dir = tempfile::tempdir().unwrap();
    write_harness_file(
        dir.path(),
        "h",
        &harness_def("h", BTreeMap::new(), Some("g1".to_string())),
    );
    let store = FakeProjectionStore::new().with_entry(
        &harness_env_key("h", "g1"),
        &serialize_env_map(&env_of(&[("K", "v")])).unwrap(),
    );
    assert!(store.contains(&harness_env_key("h", "g1")));
    assert!(extraction_verified(&store, dir.path()));
}
