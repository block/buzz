//! `env_unavailable` marker tests: hydrate sets it on a keyring outage, the
//! save path preserves the live ref rather than erasing it, and the rename
//! path is refused. Extracted from `custom_harnesses_tests.rs` via `#[path]`
//! so that file stays under the desktop file-size ratchet; `super::*` resolves
//! the parent test helpers (`FakeProjectionStore`, `env_harness_def`, …) and
//! `super::super::*` the module under test.

use super::super::*;
use super::{env_harness_def, make_def, FakeProjectionStore};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Projection store whose writes succeed but every read fails — models a
/// keyring that accepts writes but cannot be read back (the outage that leaves
/// a live `env_ref` unhydratable, setting `env_unavailable`).
struct FailingReadStore;

impl ProjectionStore for FailingReadStore {
    fn write_and_verify(&self, _key: &str, _value: &str) -> Result<(), String> {
        Ok(())
    }
    fn load_key(&self, _key: &str) -> Result<Option<String>, String> {
        Err("simulated keyring read failure".to_string())
    }
    fn load_all(&self) -> Result<Option<HashMap<String, String>>, String> {
        Err("simulated keyring read failure".to_string())
    }
    fn store_batch(&self, _entries: &HashMap<String, String>) -> Result<(), String> {
        Ok(())
    }
    fn remove_batch(&self, _keys: &[&str]) -> Result<(), String> {
        Ok(())
    }
}

// ── env_unavailable marker: hydrate sets it; save preserves the ref ───────
//
// A harness whose `env_ref` points at a keyring generation that cannot be read
// back is a keyring OUTAGE, not a user-cleared env. `hydrate_harness_env` marks
// it `env_unavailable`; the save path must preserve that live ref rather than
// erase it (the TS form round-trips neither the ref nor the marker and seeds
// `env` from the empty catalog entry, so a naive empty-env save would strand
// the pointer). The rename path has no safe outcome and is refused.

/// Persist an `env`-bearing harness through the fake, then return the on-disk
/// definition (ref present, inline env stripped) and the projected generation.
fn saved_with_projected_env(store: &FakeProjectionStore, dir: &Path) -> HarnessDefinition {
    save_custom_harness_to_dir_with(Some(store), dir, &env_harness_def(), None).unwrap();
    let raw = fs::read_to_string(dir.join("env-harness.json")).unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[test]
fn hydrate_sets_unavailable_when_ref_generation_is_missing() {
    // env_ref points at a generation that was never written — the keyring is
    // reachable (Ok(None)) but the entry is gone. This must fail closed.
    let dir = tempfile::tempdir().unwrap();
    let store = FakeProjectionStore::new();
    let mut on_disk = saved_with_projected_env(&store, dir.path());
    assert!(on_disk.env_ref.is_some(), "guard: a ref must exist");

    // Drop the projected blob so the ref dangles, then rehydrate from empty.
    let empty = FakeProjectionStore::new();
    on_disk.env.clear();
    hydrate_harness_env(&empty, &mut on_disk);

    assert!(
        on_disk.env_unavailable,
        "a ref whose generation is absent from the keyring must set env_unavailable"
    );
    assert!(on_disk.env.is_empty(), "no value could be hydrated");
}

#[test]
fn hydrate_sets_unavailable_when_keyring_read_errors() {
    // The ref exists but the keyring read itself errors (transient outage).
    let dir = tempfile::tempdir().unwrap();
    let store = FakeProjectionStore::new();
    let mut on_disk = saved_with_projected_env(&store, dir.path());
    on_disk.env.clear();

    hydrate_harness_env(&FailingReadStore, &mut on_disk);

    assert!(
        on_disk.env_unavailable,
        "a keyring read error on a live ref must set env_unavailable"
    );
}

#[test]
fn hydrate_sets_unavailable_when_stored_value_is_malformed() {
    // The blob exists but its bytes are not a valid env map — deserialize fails.
    let dir = tempfile::tempdir().unwrap();
    let store = FakeProjectionStore::new();
    let mut on_disk = saved_with_projected_env(&store, dir.path());
    let gen = on_disk.env_ref.clone().unwrap();

    // Corrupt the stored generation, then rehydrate from empty inline.
    store
        .write_and_verify(&harness_env_key("env-harness", &gen), "not json {{{")
        .unwrap();
    on_disk.env.clear();
    hydrate_harness_env(&store, &mut on_disk);

    assert!(
        on_disk.env_unavailable,
        "malformed keyring bytes on a live ref must set env_unavailable"
    );
    assert!(on_disk.env.is_empty(), "a malformed blob yields no env");
}

#[test]
fn hydrate_leaves_available_when_ref_resolves_cleanly() {
    // Positive control: a healthy ref hydrates its env and never marks
    // unavailable — so the negative assertions above are meaningful.
    let dir = tempfile::tempdir().unwrap();
    let store = FakeProjectionStore::new();
    let mut on_disk = saved_with_projected_env(&store, dir.path());

    hydrate_harness_env(&store, &mut on_disk);

    assert!(
        !on_disk.env_unavailable,
        "a healthy ref must never set env_unavailable"
    );
    assert_eq!(
        on_disk.env.get("ANTHROPIC_API_KEY").map(String::as_str),
        Some("sk-ant-secret"),
        "a healthy ref must hydrate its env"
    );
}

#[test]
fn hydrate_leaves_available_for_ref_less_empty_env() {
    // A genuinely-empty harness (no ref) is available, not unavailable — the
    // marker distinguishes an outage from an intentionally-empty env.
    let mut def = make_def("bare", "Bare"); // no env, no ref
    hydrate_harness_env(&FailingReadStore, &mut def);
    assert!(
        !def.env_unavailable,
        "an env-less record has nothing to hydrate and must stay available"
    );
}

#[test]
fn save_preserves_ref_when_persisted_record_is_env_unavailable() {
    // The pointer-loss fix: an on-disk record with a live-but-unhydratable ref
    // must keep that ref across a same-id save whose incoming env is empty
    // (exactly the TS-form round-trip: no ref, no marker, empty catalog env).
    let dir = tempfile::tempdir().unwrap();
    let store = FakeProjectionStore::new();
    let on_disk = saved_with_projected_env(&store, dir.path());
    let live_ref = on_disk.env_ref.clone().expect("a ref must be persisted");

    // The keyring can no longer read the projected generation — the record is
    // now env_unavailable. The incoming save carries an empty env and no ref.
    let mut incoming = env_harness_def();
    incoming.env.clear();
    incoming.env_ref = None;
    save_custom_harness_to_dir_with(Some(&FailingReadStore), dir.path(), &incoming, None).unwrap();

    let after: HarnessDefinition =
        serde_json::from_str(&fs::read_to_string(dir.path().join("env-harness.json")).unwrap())
            .unwrap();
    assert_eq!(
        after.env_ref.as_deref(),
        Some(live_ref.as_str()),
        "an empty-env save over an unavailable record must preserve the live ref"
    );
}

#[test]
fn save_clears_ref_when_persisted_record_is_healthy() {
    // The inverse of the preserve case: a genuine user-clear of a HEALTHY
    // ref-backed record (the on-disk ref hydrates fine) must clear the ref.
    // Break `persisted_unavailable_env_ref` to always return the ref and this
    // fails — proving the preserve path keys on unavailability, not on any ref.
    let dir = tempfile::tempdir().unwrap();
    let store = FakeProjectionStore::new();
    let on_disk = saved_with_projected_env(&store, dir.path());
    assert!(on_disk.env_ref.is_some(), "guard: a healthy ref must exist");

    // Same store (ref resolves cleanly), incoming env empty → true user-clear.
    let mut incoming = env_harness_def();
    incoming.env.clear();
    incoming.env_ref = None;
    save_custom_harness_to_dir_with(Some(&store), dir.path(), &incoming, None).unwrap();

    let after: HarnessDefinition =
        serde_json::from_str(&fs::read_to_string(dir.path().join("env-harness.json")).unwrap())
            .unwrap();
    assert!(
        after.env_ref.is_none(),
        "clearing a healthy record's env must clear the ref, not preserve it"
    );
}

#[test]
fn rename_of_env_unavailable_record_is_refused() {
    // A rename cannot re-read the ref under the new id (the key embeds the id)
    // and carrying it forward would strand it at an unwritten coordinate, so
    // the save refuses rather than silently erasing or mis-pointing the ref.
    let dir = tempfile::tempdir().unwrap();
    let store = FakeProjectionStore::new();
    let on_disk = saved_with_projected_env(&store, dir.path());
    assert!(on_disk.env_ref.is_some(), "guard: old id has a live ref");

    // Rename env-harness → renamed-harness while the keyring cannot read back.
    let mut renamed = env_harness_def();
    renamed.id = "renamed-harness".to_string();
    renamed.env.clear();
    renamed.env_ref = None;
    let result = save_custom_harness_to_dir_with(
        Some(&FailingReadStore),
        dir.path(),
        &renamed,
        Some("env-harness"),
    );
    let Err(err) = result else {
        panic!("a rename of an unavailable record must be refused");
    };
    assert!(
        err.contains("env-harness") && err.contains("unavailable"),
        "the refusal must name the old id and the reason: {err}"
    );
    // The old file must be untouched — no partial rename left behind.
    assert!(
        dir.path().join("env-harness.json").exists(),
        "the refused rename must not remove the old file"
    );
    assert!(
        !dir.path().join("renamed-harness.json").exists(),
        "the refused rename must not create the new file"
    );
}

#[test]
fn rename_of_healthy_record_succeeds_and_reprojects() {
    // A rename of a HEALTHY ref-backed record works: the TS form round-trips
    // the fully-hydrated env, so strip re-projects it under the new id. Guards
    // that the refusal above is scoped to the unavailable case only.
    let dir = tempfile::tempdir().unwrap();
    let store = FakeProjectionStore::new();
    saved_with_projected_env(&store, dir.path());

    // The form re-submits the full env (hydrated) under the new id.
    let mut renamed = env_harness_def();
    renamed.id = "renamed-harness".to_string();
    let outcome =
        save_custom_harness_to_dir_with(Some(&store), dir.path(), &renamed, Some("env-harness"))
            .expect("a healthy rename must succeed");

    assert_eq!(
        outcome.removed_old_path,
        Some(dir.path().join("env-harness.json")),
        "the old file must be removed on a successful rename"
    );
    let after = load_custom_harnesses_with(Some(&store), dir.path());
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].id, "renamed-harness");
    assert_eq!(
        after[0].env.get("ANTHROPIC_API_KEY").map(String::as_str),
        Some("sk-ant-secret"),
        "the renamed record must carry its env under the new id"
    );
}
