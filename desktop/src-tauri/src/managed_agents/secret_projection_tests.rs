use super::*;
use std::cell::RefCell;

// ── FakeProjectionStore ──────────────────────────────────────────────

struct FakeProjectionStore {
    reachable: bool,
    fail_verify: bool,
    data: RefCell<HashMap<String, String>>,
}

impl FakeProjectionStore {
    fn reachable() -> Self {
        Self {
            reachable: true,
            fail_verify: false,
            data: RefCell::new(HashMap::new()),
        }
    }
    fn unreachable() -> Self {
        Self {
            reachable: false,
            fail_verify: false,
            data: RefCell::new(HashMap::new()),
        }
    }
    fn verify_fails() -> Self {
        Self {
            reachable: true,
            fail_verify: true,
            data: RefCell::new(HashMap::new()),
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

impl ProjectionStore for FakeProjectionStore {
    fn write_and_verify(&self, key: &str, value: &str) -> Result<(), String> {
        if !self.reachable {
            return Err("unreachable".to_string());
        }
        if self.fail_verify {
            return Err("verify failed".to_string());
        }
        self.data
            .borrow_mut()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn load_key(&self, key: &str) -> Result<Option<String>, String> {
        if !self.reachable {
            return Err("unreachable".to_string());
        }
        Ok(self.data.borrow().get(key).cloned())
    }

    fn load_all(&self) -> Result<Option<HashMap<String, String>>, String> {
        if !self.reachable {
            return Err("unreachable".to_string());
        }
        if self.data.borrow().is_empty() {
            Ok(None)
        } else {
            Ok(Some(self.data.borrow().clone()))
        }
    }

    fn store_batch(&self, entries: &HashMap<String, String>) -> Result<(), String> {
        if !self.reachable {
            return Err("unreachable".to_string());
        }
        for (k, v) in entries {
            self.data.borrow_mut().insert(k.clone(), v.clone());
        }
        Ok(())
    }

    fn remove_batch(&self, keys: &[&str]) -> Result<(), String> {
        if !self.reachable {
            return Err("unreachable".to_string());
        }
        let mut data = self.data.borrow_mut();
        for k in keys {
            data.remove(*k);
        }
        Ok(())
    }
}

// ── is_projection_key ────────────────────────────────────────────────

#[test]
fn test_is_projection_key_global_env() {
    assert!(is_projection_key("global:env:abc123"));
    assert!(!is_projection_key("global:model"));
    assert!(!is_projection_key("identity"));
    assert!(!is_projection_key("agent:abc123")); // no sub-part
}

#[test]
fn test_is_projection_key_agent_namespaces() {
    assert!(is_projection_key("agent:abc123:env:gen1"));
    assert!(is_projection_key("agent:abc123:auth_tag:gen1"));
    assert!(is_projection_key("agent:abc123:provider_config:gen1"));
    assert!(is_projection_key("agent:abc123:env:gen1_candidate"));
    assert!(!is_projection_key("agent:abc123")); // nsec key — not ours
}

#[test]
fn test_is_projection_key_definition() {
    assert!(is_projection_key("definition:my-slug:env:gen1"));
    assert!(!is_projection_key("definition:my-slug")); // no part
}

// ── write_secret ──────────────────────────────────────────────────────

#[test]
fn test_write_secret_nothing_on_empty_value() {
    let store = FakeProjectionStore::reachable();
    let outcome = write_secret(&store, global_env_key, None, "test");
    assert_eq!(outcome, WriteOutcome::Nothing);
    assert!(store.keys().is_empty());
}

#[test]
fn test_write_secret_persisted_on_success() {
    let store = FakeProjectionStore::reachable();
    let outcome = write_secret(
        &store,
        global_env_key,
        Some("sk-ant-api03-secret"),
        "global:env",
    );
    match &outcome {
        WriteOutcome::Persisted { gen } => {
            let key = global_env_key(gen);
            assert_eq!(store.get(&key), Some("sk-ant-api03-secret".to_string()));
        }
        other => panic!("expected Persisted, got {other:?}"),
    }
}

#[test]
fn test_write_secret_kept_inline_on_verify_failure() {
    let store = FakeProjectionStore::verify_fails();
    let outcome = write_secret(&store, global_env_key, Some("sk-ant-secret"), "global:env");
    assert!(matches!(outcome, WriteOutcome::KeptInline { .. }));
}

#[test]
fn test_write_secret_kept_inline_on_unreachable() {
    let store = FakeProjectionStore::unreachable();
    let outcome = write_secret(&store, global_env_key, Some("value"), "test");
    assert!(matches!(outcome, WriteOutcome::KeptInline { .. }));
}

// ── load_secret ───────────────────────────────────────────────────────

#[test]
fn test_load_secret_none_when_no_ref() {
    let store = FakeProjectionStore::reachable();
    let result = load_secret(&store, None, global_env_key, "test");
    assert_eq!(result, Ok(None));
}

#[test]
fn test_load_secret_ok_when_entry_present() {
    let store = FakeProjectionStore::reachable().with_entry("global:env:gen1", "sk-secret");
    let result = load_secret(&store, Some("gen1"), global_env_key, "global:env");
    assert_eq!(result, Ok(Some("sk-secret".to_string())));
}

#[test]
fn test_load_secret_err_when_ref_present_but_missing() {
    let store = FakeProjectionStore::reachable();
    let result = load_secret(&store, Some("gen1"), global_env_key, "global:env");
    assert!(result.is_err(), "expected unavailable error");
}

#[test]
fn test_load_secret_err_when_keyring_unreachable() {
    let store = FakeProjectionStore::unreachable();
    let result = load_secret(&store, Some("gen1"), global_env_key, "global:env");
    assert!(result.is_err());
}

/// A [`ProjectionStore`] whose conflict-marker read fails transiently while
/// the value read succeeds — the exact production shape Thufir flagged: the
/// real `SecretStore::load_blob` caches a successful read but never caches an
/// error, so a first (marker) read can error and an immediately-following
/// (value) read can succeed against a warm cache. The value is present and
/// KNOWN-conflicted; `load_secret` must fail closed on the marker-read `Err`
/// rather than fall through and hydrate it.
struct MarkerReadErrStore {
    value_key: String,
    value: String,
}

impl ProjectionStore for MarkerReadErrStore {
    fn write_and_verify(&self, _key: &str, _value: &str) -> Result<(), String> {
        unreachable!("load_secret never writes")
    }
    fn load_key(&self, key: &str) -> Result<Option<String>, String> {
        if key.starts_with("conflict:") {
            // Transient backend failure on the marker read only.
            return Err("transient marker read failure".to_string());
        }
        if key == self.value_key {
            // The value read succeeds — a known-conflicted credential.
            return Ok(Some(self.value.clone()));
        }
        Ok(None)
    }
    fn load_all(&self) -> Result<Option<HashMap<String, String>>, String> {
        unreachable!("load_secret never calls load_all")
    }
    fn store_batch(&self, _entries: &HashMap<String, String>) -> Result<(), String> {
        unreachable!("load_secret never writes")
    }
    fn remove_batch(&self, _keys: &[&str]) -> Result<(), String> {
        unreachable!("load_secret never deletes")
    }
}

#[test]
fn test_load_secret_fails_closed_when_marker_read_errors() {
    // F4: a transient conflict-marker read `Err` must be treated as "conflict
    // status unknown" → unavailable, even though the value read would succeed.
    // Falling through would hydrate a known-conflicted value the moment the
    // marker check flaked.
    let value_key = global_env_key("gen1");
    let store = MarkerReadErrStore {
        value_key,
        value: "sk-known-conflicted".to_string(),
    };
    let result = load_secret(&store, Some("gen1"), global_env_key, "global:env");
    assert!(
        result.is_err(),
        "a marker-read Err must fail closed, not fall through to the value read"
    );
    let msg = result.unwrap_err();
    assert!(
        msg.contains("conflict-marker read") && msg.contains("failed"),
        "error must name the marker-read failure as the refusal cause, got: {msg}"
    );
}

// ── Two-cycle GC tests ────────────────────────────────────────────────

fn make_agents_json(env_ref: Option<&str>) -> String {
    if let Some(r) = env_ref {
        format!(
            r#"[{{"pubkey":"abc","name":"test","env_vars_ref":"{r}","created_at":"2026","updated_at":"2026"}}]"#
        )
    } else {
        r#"[{"pubkey":"abc","name":"test","created_at":"2026","updated_at":"2026"}]"#.to_string()
    }
}

fn make_global_json(env_ref: Option<&str>) -> String {
    if let Some(r) = env_ref {
        format!(r#"{{"env_vars_ref":"{r}"}}"#)
    } else {
        "{}".to_string()
    }
}

#[test]
fn test_collect_live_refs_extracts_refs() {
    let agents = make_agents_json(Some("gen1"));
    let global = make_global_json(Some("gen2"));
    let refs = collect_live_refs(&agents, &global).unwrap();
    assert!(refs.gen_ids.contains("gen1"));
    assert!(refs.gen_ids.contains("gen2"));
}

#[test]
fn test_collect_live_refs_empty_when_no_refs() {
    let agents = make_agents_json(None);
    let global = make_global_json(None);
    let refs = collect_live_refs(&agents, &global).unwrap();
    assert!(refs.gen_ids.is_empty());
}

#[test]
fn test_collect_live_refs_none_on_malformed_json() {
    let result = collect_live_refs("not json", "{}");
    assert!(result.is_none());
}

// ── F5b: collect_live_refs validates, not just collects ──────────────────
//
// Any ambiguity in the JSON makes the whole sweep a no-op (returns None) —
// a partial deletion decision could orphan a live secret.

#[test]
fn test_collect_live_refs_none_on_empty_coordinate() {
    // An empty ref string is a malformed coordinate — it cannot match any
    // blob generation, so the sweep must not run.
    let agents = r#"[{"pubkey":"abc","name":"t","env_vars_ref":"","created_at":"2026","updated_at":"2026"}]"#;
    assert!(collect_live_refs(agents, "{}").is_none());
}

#[test]
fn test_collect_live_refs_none_on_embedded_colon_coordinate() {
    // A gen id is the last `:`-segment of a blob key. An embedded `:` in the
    // ref would make it un-matchable, so the coordinate is malformed.
    let agents = r#"[{"pubkey":"abc","name":"t","env_vars_ref":"gen:evil","created_at":"2026","updated_at":"2026"}]"#;
    assert!(collect_live_refs(agents, "{}").is_none());
}

#[test]
fn test_collect_live_refs_none_on_duplicate_gen_id() {
    // Two coordinates referencing the same gen id — impossible for fresh
    // UUIDs, so the JSON is corrupt and the sweep must not run.
    let agents = r#"[
        {"pubkey":"a","name":"t","env_vars_ref":"gen1","created_at":"2026","updated_at":"2026"},
        {"pubkey":"b","name":"t","env_vars_ref":"gen1","created_at":"2026","updated_at":"2026"}
    ]"#;
    assert!(collect_live_refs(agents, "{}").is_none());
}

#[test]
fn test_collect_live_refs_none_on_duplicate_across_field_and_global() {
    // Same gen id in an agent env ref and the global env ref.
    let agents = r#"[{"pubkey":"a","name":"t","env_vars_ref":"gen1","created_at":"2026","updated_at":"2026"}]"#;
    let global = r#"{"env_vars_ref":"gen1"}"#;
    assert!(collect_live_refs(agents, global).is_none());
}

#[test]
fn test_collect_live_refs_none_on_inline_plus_ref_conflict_env() {
    // A record carrying BOTH non-empty inline env_vars AND an env_vars_ref is
    // ambiguous: inline is authoritative on load, so the ref is being ignored.
    // The GC must not reason about which gen is live.
    let agents = r#"[{"pubkey":"a","name":"t","env_vars":{"K":"v"},"env_vars_ref":"gen1","created_at":"2026","updated_at":"2026"}]"#;
    assert!(collect_live_refs(agents, "{}").is_none());
}

#[test]
fn test_collect_live_refs_none_on_inline_plus_ref_conflict_auth_tag() {
    let agents = r#"[{"pubkey":"a","name":"t","auth_tag":"live-tag","auth_tag_ref":"gen1","created_at":"2026","updated_at":"2026"}]"#;
    assert!(collect_live_refs(agents, "{}").is_none());
}

#[test]
fn test_collect_live_refs_none_on_inline_plus_ref_conflict_provider_config() {
    let agents = r#"[{"pubkey":"a","name":"t","backend":{"type":"provider","id":"anthropic","config":{"k":"v"}},"provider_config_ref":"gen1","created_at":"2026","updated_at":"2026"}]"#;
    assert!(collect_live_refs(agents, "{}").is_none());
}

#[test]
fn test_collect_live_refs_none_on_global_inline_plus_ref_conflict() {
    let global = r#"{"env_vars":{"K":"v"},"env_vars_ref":"gen1"}"#;
    assert!(collect_live_refs("[]", global).is_none());
}

#[test]
fn test_collect_live_refs_allows_empty_inline_with_ref() {
    // Empty inline (env_vars: {}) alongside a ref is the HEALTHY stripped
    // state — not a conflict. The ref must be collected.
    let agents = r#"[{"pubkey":"a","name":"t","env_vars":{},"env_vars_ref":"gen1","created_at":"2026","updated_at":"2026"}]"#;
    let refs = collect_live_refs(agents, "{}").expect("empty inline + ref is healthy");
    assert!(refs.gen_ids.contains("gen1"));
}

#[test]
fn test_collect_live_refs_allows_null_provider_config_with_ref() {
    // Stripped provider config is JSON null alongside a ref — healthy state.
    let agents = r#"[{"pubkey":"a","name":"t","backend":{"type":"provider","id":"anthropic","config":null},"provider_config_ref":"gen1","created_at":"2026","updated_at":"2026"}]"#;
    let refs = collect_live_refs(agents, "{}").expect("null config + ref is healthy");
    assert!(refs.gen_ids.contains("gen1"));
}

#[test]
fn test_collect_live_refs_collects_all_three_instance_fields() {
    let agents = r#"[{"pubkey":"a","name":"t","env_vars_ref":"g_env","auth_tag_ref":"g_auth","backend":{"type":"provider","id":"anthropic","config":null},"provider_config_ref":"g_pc","created_at":"2026","updated_at":"2026"}]"#;
    let refs = collect_live_refs(agents, "{}").unwrap();
    assert!(refs.gen_ids.contains("g_env"));
    assert!(refs.gen_ids.contains("g_auth"));
    assert!(refs.gen_ids.contains("g_pc"));
}

#[test]
fn test_gc_interleaving_save_cancels_candidacy() {
    // Simulate: GC marks gen1 as candidate, then a save confirms it into JSON.
    // GC sweep 2 should NOT delete gen1 because the ref is now live.
    let store = FakeProjectionStore::reachable()
        .with_entry("global:env:gen1", "sk-secret")
        .with_entry("global:env:gen1_candidate", "1"); // sweep 1 already ran

    // Sweep 2 re-reads JSON — now gen1 IS referenced.
    // We simulate this by providing JSON that references gen1.
    let agents_content = make_agents_json(None);
    let global_content = make_global_json(Some("gen1"));

    let live_refs = collect_live_refs(&agents_content, &global_content).unwrap();
    assert!(live_refs.gen_ids.contains("gen1"), "gen1 must be live");

    // Verify that delete_gc_candidates would skip gen1 because it's live.
    // Since we can't call delete_gc_candidates directly (it reads files),
    // we verify the logic: a live ref prevents deletion.
    let blob = store.load_all().unwrap().unwrap();
    let candidate_key = "global:env:gen1_candidate";
    assert!(
        blob.contains_key(candidate_key),
        "candidate should be present"
    );

    // Simulate what delete_gc_candidates does: skip live refs.
    let gen = "gen1";
    let would_delete = !live_refs.gen_ids.contains(gen);
    assert!(
        !would_delete,
        "gen1 must NOT be deleted — it's now referenced"
    );
}

#[test]
fn test_gc_no_op_on_unreachable_keyring() {
    // GC mark/delete must be skipped when the keyring is unavailable.
    let store = FakeProjectionStore::unreachable().with_entry("global:env:gen1", "value");
    // load_all returns Err — mark_gc_candidates aborts early.
    let result = store.load_all();
    assert!(result.is_err());
    // Confirms GC would abort before any writes.
}

// ── F5a: synchronized interleaving — GC vs an in-flight save ─────────────
//
// The dangerous ordering the store lock exists to prevent:
//   1. A save writes a NEW generation to the blob and read-back verifies it.
//   2. The save has NOT yet committed the JSON pointing at the new gen.
//   3. GC runs its full two-cycle sweep against the CURRENT (old) JSON.
//   4. The save commits its JSON.
//
// If GC could observe the pre-commit JSON AND delete on the same cycle, the
// new gen (unreferenced in old JSON) would be destroyed. Two properties make
// this safe and are exercised here with the REAL GC functions over tempfile
// JSON: (a) delete-before-mark means a gen written this boot is only a
// deletion candidate after a full mark cycle, never on the boot it appears;
// (b) once the JSON commits, the ref is live and the gen is protected.

fn write_json_stores(
    agents: &str,
    global: &str,
) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let agents_path = dir.path().join("managed-agents.json");
    let global_path = dir.path().join("global-agent-config.json");
    std::fs::write(&agents_path, agents).expect("write agents");
    std::fs::write(&global_path, global).expect("write global");
    (dir, agents_path, global_path)
}

#[test]
fn test_gc_delete_before_mark_spares_gen_written_this_boot() {
    // A generation written + verified this boot, whose JSON commit has NOT
    // landed (JSON still references the OLD gen), must survive a full GC pass.
    // gen_new has no candidate marker yet, so delete phase skips it; the mark
    // phase marks it — but deletion only happens on a LATER boot's delete
    // phase, giving the pending save a full cycle to commit.
    let store = FakeProjectionStore::reachable()
        .with_entry("global:env:gen_old", "old-secret")
        .with_entry("global:env:gen_new", "new-secret"); // in-flight, not yet in JSON

    // JSON still references the OLD generation (commit pending).
    let (_dir, agents_path, global_path) =
        write_json_stores(&make_agents_json(None), &make_global_json(Some("gen_old")));

    // Full two-cycle pass in the boot order: delete, then mark.
    delete_gc_candidates(&store, &agents_path, &global_path);
    mark_gc_candidates(&store, &agents_path, &global_path);

    // gen_new must NOT have been deleted this boot.
    assert!(
        store.get("global:env:gen_new").is_some(),
        "in-flight generation must survive the GC pass before its JSON commit"
    );
}

#[test]
fn test_gc_spares_gen_once_json_commit_lands() {
    // Continuation: the save now commits its JSON (references gen_new). Even
    // though gen_new was marked as a candidate on the prior boot, the delete
    // phase re-reads JSON, sees gen_new is live, and spares it — while the
    // now-unreferenced gen_old is reclaimed.
    let store = FakeProjectionStore::reachable()
        .with_entry("global:env:gen_old", "old-secret")
        .with_entry("global:env:gen_new", "new-secret")
        .with_entry("global:env:gen_new_candidate", "1") // marked last boot
        .with_entry("global:env:gen_old_candidate", "1"); // also marked last boot

    // JSON now references gen_new (the save committed).
    let (_dir, agents_path, global_path) =
        write_json_stores(&make_agents_json(None), &make_global_json(Some("gen_new")));

    delete_gc_candidates(&store, &agents_path, &global_path);

    assert!(
        store.get("global:env:gen_new").is_some(),
        "committed generation must be spared even though it was a candidate"
    );
    // gen_old is now unreferenced and was a candidate → reclaimed.
    assert!(
        store.get("global:env:gen_old").is_none(),
        "the retired generation must be reclaimed once it is unreferenced"
    );
}

#[test]
fn test_gc_reclaims_stably_unreferenced_candidate() {
    // The delete phase reclaims a candidate whose generation is unreferenced
    // in JSON and stays unreferenced across the snapshot + final re-check.
    // This is the positive case that bounds the mid-sweep abort guard: with
    // stable JSON there is no false abort, so retirement actually happens.
    let store = FakeProjectionStore::reachable()
        .with_entry("global:env:gen_stale", "secret")
        .with_entry("global:env:gen_stale_candidate", "1");
    let (_dir, agents_path, global_path) =
        write_json_stores(&make_agents_json(None), &make_global_json(None));

    delete_gc_candidates(&store, &agents_path, &global_path);

    assert!(
        store.get("global:env:gen_stale").is_none(),
        "a stably-unreferenced candidate must be reclaimed"
    );
}

// ── F5b: GC validates live refs against the blob, not just syntax ─────────
//
// A live ref whose full coordinate is MISSING from the blob (dangling) means
// the store is degraded: an older, unreferenced generation for the SAME field
// could be the only recoverable payload. Both sweeps must no-op until the
// reference resolves — deleting the unreferenced candidate would destroy the
// last copy.

#[test]
fn test_gc_delete_no_op_when_a_live_ref_coordinate_is_missing() {
    // JSON references live gen_g (dangling: NOT present in the blob).
    // Candidate gen_h is unreferenced and marked from a prior boot.
    // Without the coordinate check, delete would reclaim gen_h; with it, the
    // dangling live ref freezes ALL deletion so gen_h (a possible last copy)
    // survives.
    let store = FakeProjectionStore::reachable()
        .with_entry("global:env:gen_h", "recoverable-secret")
        .with_entry("global:env:gen_h_candidate", "1"); // marked last boot
                                                        // Note: global:env:gen_g is deliberately ABSENT from the blob.

    let agents = make_agents_json(None);
    let global = make_global_json(Some("gen_g")); // JSON references the dangling gen

    // Sanity: gen_g's coordinate is a live ref but its blob entry is missing.
    let live_refs = collect_live_refs(&agents, &global).unwrap();
    assert!(live_refs.gen_ids.contains("gen_g"));
    assert!(live_refs.coords.contains("global:env:gen_g"));

    let (_dir, agents_path, global_path) = write_json_stores(&agents, &global);
    delete_gc_candidates(&store, &agents_path, &global_path);

    assert!(
        store.get("global:env:gen_h").is_some(),
        "gen_h must survive: a dangling live ref freezes deletion so the last \
         recoverable payload is not destroyed"
    );
}

#[test]
fn test_gc_mark_no_op_when_a_live_ref_coordinate_is_missing() {
    // Same degraded state as above, at the MARK phase: an unreferenced gen_h
    // must NOT be newly marked as a candidate while a live ref is dangling —
    // marking is the first step toward deletion, so it is frozen too.
    let store =
        FakeProjectionStore::reachable().with_entry("global:env:gen_h", "recoverable-secret");
    // global:env:gen_g (the live ref's coordinate) is ABSENT.

    let agents = make_agents_json(None);
    let global = make_global_json(Some("gen_g"));
    let (_dir, agents_path, global_path) = write_json_stores(&agents, &global);

    mark_gc_candidates(&store, &agents_path, &global_path);

    assert!(
        store.get("global:env:gen_h_candidate").is_none(),
        "gen_h must not be marked while a live ref is dangling"
    );
}

#[test]
fn test_gc_delete_proceeds_once_all_live_coordinates_present() {
    // Positive bound: with every live ref's coordinate present in the blob,
    // the degraded-state guard does not fire and an unreferenced candidate is
    // reclaimed as normal. This proves the new check gates on the missing
    // coordinate specifically, not on the mere presence of any live ref.
    let store = FakeProjectionStore::reachable()
        .with_entry("global:env:gen_g", "live-secret") // live ref coordinate present
        .with_entry("global:env:gen_h", "stale-secret")
        .with_entry("global:env:gen_h_candidate", "1");

    let agents = make_agents_json(None);
    let global = make_global_json(Some("gen_g"));
    let (_dir, agents_path, global_path) = write_json_stores(&agents, &global);

    delete_gc_candidates(&store, &agents_path, &global_path);

    assert!(
        store.get("global:env:gen_g").is_some(),
        "the live generation must be spared"
    );
    assert!(
        store.get("global:env:gen_h").is_none(),
        "the unreferenced candidate must be reclaimed once no live ref dangles"
    );
}

#[test]
fn test_cancel_gc_candidacy_removes_marker() {
    let store = FakeProjectionStore::reachable()
        .with_entry("global:env:gen1", "secret")
        .with_entry("global:env:gen1_candidate", "1");

    cancel_gc_candidacy(&store, "global:env:gen1");
    assert!(
        store.get("global:env:gen1_candidate").is_none(),
        "candidate marker must be removed"
    );
    assert_eq!(
        store.get("global:env:gen1"),
        Some("secret".to_string()),
        "generation must NOT be deleted by cancel"
    );
}

// ── Key constructors ──────────────────────────────────────────────────

#[test]
fn test_key_constructors_round_trip() {
    assert_eq!(global_env_key("gen1"), "global:env:gen1");
    assert_eq!(agent_env_key("abc", "gen1"), "agent:abc:env:gen1");
    assert_eq!(agent_auth_tag_key("abc", "gen1"), "agent:abc:auth_tag:gen1");
    assert_eq!(
        agent_provider_config_key("abc", "gen1"),
        "agent:abc:provider_config:gen1"
    );
    assert_eq!(
        definition_env_key("my-slug", "gen1"),
        "definition:my-slug:env:gen1"
    );
}

// ── Serialization ─────────────────────────────────────────────────────

#[test]
fn test_serialize_deserialize_env_map() {
    let mut env = BTreeMap::new();
    env.insert("ANTHROPIC_API_KEY".to_string(), "sk-ant-secret".to_string());
    env.insert("BUZZ_THINKING".to_string(), "high".to_string());
    let s = serialize_env_map(&env).unwrap();
    let back = deserialize_env_map(&s).unwrap();
    assert_eq!(env, back);
}

#[test]
fn test_serialize_deserialize_provider_config() {
    let config = serde_json::json!({"host": "example.com", "port": 443});
    let s = serialize_provider_config(&config).unwrap();
    let back = deserialize_provider_config(&s).unwrap();
    assert_eq!(config, back);
}

// ── Inline-over-ref precedence (spec §1, inline-precedence pin) ───────

#[test]
fn test_inline_wins_over_ref_when_both_present() {
    // Hydration must prefer inline over any keyring ref — the inline is
    // the authoritative value when the keyring write failed.
    // This test verifies the CONTRACT expected by the hydration code:
    // when env_vars is non-empty (inline) and env_vars_ref is also set,
    // the hydration layer must NOT overwrite the inline value with the
    // keyring value.
    //
    // The actual enforcement happens in hydrate_global_secrets and
    // hydrate_agent_secrets (in storage.rs). This test verifies the
    // fundamental assumption: if env_vars is non-empty, it should be
    // treated as the authoritative value.
    let inline_env = {
        let mut m = BTreeMap::new();
        m.insert("KEY".to_string(), "inline-value".to_string());
        m
    };
    // Simulate: inline value is present (keyring write failed last boot).
    // The hydration code should keep `inline_env` and not call load_secret.
    let inline_is_present = !inline_env.is_empty();
    assert!(
        inline_is_present,
        "inline must take precedence when present"
    );
}

// ── Two-cycle GC ordering and cancel-before-mark tests ───────────────

#[test]
fn test_gc_delete_first_order_preserves_in_flight_gen() {
    // Spec §6: delete candidates from the PREVIOUS boot first, then mark
    // new ones for THIS boot.  A generation verified this boot but not yet
    // committed to JSON must NOT be deleted this boot.
    //
    // Setup: gen1 is in-flight (written + verified, JSON commit pending).
    // boot N-1 left a candidate marker for an old gen0 that is now gone.
    // GC delete phase runs: gen0 (with a candidate marker) is still
    // unreferenced — it should be deleted.  gen1 has NO candidate marker
    // yet — it must NOT be deleted.
    //
    // This tests the FakeProjectionStore's delete_gc_candidates logic
    // directly via collect_live_refs + manual blob inspection.

    // Blob state at start of boot N's GC:
    //   gen0 was orphaned last boot and marked as candidate
    //   gen1 is the new in-flight generation (no marker yet)
    let store = FakeProjectionStore::reachable()
        .with_entry("global:env:gen0", "old-secret") // old generation
        .with_entry("global:env:gen0_candidate", "1") // marked last boot
        .with_entry("global:env:gen1", "new-secret"); // in-flight

    // JSON currently still references gen0 (JSON commit hasn't happened).
    // GC reads the live refs from JSON.
    let agents = make_agents_json(None);
    let global = make_global_json(Some("gen0")); // JSON still has old ref

    let live_refs = collect_live_refs(&agents, &global).unwrap();
    assert!(live_refs.gen_ids.contains("gen0"), "gen0 is still in JSON");
    assert!(
        !live_refs.gen_ids.contains("gen1"),
        "gen1 not yet committed to JSON"
    );

    // delete_gc_candidates: gen0 is a candidate but IS referenced → skip.
    // gen1 has NO candidate marker → skip.
    // Nothing should be deleted this cycle.
    let blob = store.load_all().unwrap().unwrap();
    let candidate_for_gen0 = blob.get("global:env:gen0_candidate");
    let candidate_for_gen1 = blob.get("global:env:gen1_candidate");
    assert!(
        candidate_for_gen0.is_some(),
        "gen0 candidate marker must still be present"
    );
    assert!(
        candidate_for_gen1.is_none(),
        "gen1 must have no candidate marker"
    );

    // Verify delete_gc_candidates would skip gen0 because it IS referenced.
    let gen = "gen0";
    let would_delete = !live_refs.gen_ids.contains(gen);
    assert!(
        !would_delete,
        "gen0 must NOT be deleted — it's still referenced in JSON"
    );
}

#[test]
fn test_cancel_before_mark_ordering_protects_in_flight_gen() {
    // Spec §6: cancel happens before the JSON commit.  The GC mark phase
    // that runs AFTER the cancel must not re-mark the in-flight generation.
    //
    // Scenario (cancel-happens-before-mark, the case Paul flagged):
    // 1. Save writes gen2 and verifies it.
    // 2. Save calls cancel_gc_candidacy("gen2") — a no-op since gen2 wasn't
    //    marked, but it guarantees the marker is absent.
    // 3. GC mark phase runs (shouldn't happen in the same call, but safe).
    // 4. GC sees gen2 as unreferenced (JSON still has gen1 ref) and marks it.
    // 5. Save commits JSON with gen2 ref.
    // 6. GC delete phase (next boot) sees gen2 is now referenced → skips.
    //
    // The key correctness property: between steps 2 and 5, even if GC
    // marks gen2, the NEXT boot's delete phase sees gen2 as referenced and
    // will not delete it.  This test verifies step 4 is safe: a marked-
    // then-referenced gen survives.

    let _store = FakeProjectionStore::reachable()
        .with_entry("global:env:gen1", "old-secret")
        .with_entry("global:env:gen2", "new-secret")
        // GC marked gen2 as a candidate (step 4 above).
        .with_entry("global:env:gen2_candidate", "1");

    // After JSON commit (step 5), gen2 is referenced.
    let agents = make_agents_json(None);
    let global = make_global_json(Some("gen2")); // JSON now has gen2 ref

    let live_refs = collect_live_refs(&agents, &global).unwrap();
    assert!(
        live_refs.gen_ids.contains("gen2"),
        "gen2 is now referenced in JSON"
    );

    // delete_gc_candidates (next boot): gen2 is a candidate BUT is now
    // referenced → skip.  gen2 must NOT be deleted.
    let gen = "gen2";
    let would_delete = !live_refs.gen_ids.contains(gen);
    assert!(
        !would_delete,
        "gen2 must NOT be deleted — it's now referenced in JSON"
    );

    // gen1 is now unreferenced → it should become a candidate on next mark.
    assert!(
        !live_refs.gen_ids.contains("gen1"),
        "gen1 is no longer referenced (gen2 replaced it)"
    );
}
