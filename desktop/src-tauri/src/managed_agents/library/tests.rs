//! Phase-1 data-model tests (§7): the document envelope's quarantine-preserving
//! load/rebuild (P2-I2), per-entry decode + semantic + identity-index
//! quarantine (P4-I3/P6-I3), `apply_shared_definition` field isolation (§2.2),
//! and the scope-local journals' asymmetric failure domains (P9-I1/P10-C3).

use std::collections::BTreeMap;

use nostr::Keys;
use serde_json::{json, Value};
use tempfile::tempdir;

use super::journals::*;
use super::*;
use crate::managed_agents::types::AgentDefinition;

// ── fixtures ──────────────────────────────────────────────────────────────────

/// A blank [`AgentDefinition`] — the seed for `apply_shared_definition` field
/// tests. `AgentDefinition` derives no `Default`, and a full literal is robust
/// to future field additions in a way a `..Default::default()` shorthand can't
/// be (there is nothing to spread).
fn minimal_definition() -> AgentDefinition {
    AgentDefinition {
        id: "seed".to_string(),
        display_name: "Seed".to_string(),
        avatar_url: None,
        system_prompt: "seed prompt".to_string(),
        runtime: None,
        model: None,
        provider: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        shared: false,
        source_team: None,
        source_team_persona_slug: None,
        catalog_source: None,
        env_vars: BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2026-08-11T00:00:00Z".to_string(),
        updated_at: "2026-08-11T00:00:00Z".to_string(),
    }
}

/// A verified binding under `owner`, plus the (owner, agent) hex pair.
fn binding(owner: &Keys, agent: &Keys) -> (String, IdentityBinding, String) {
    let owner_hex = owner.public_key().to_hex();
    let agent_hex = agent.public_key().to_hex();
    let auth_tag = buzz_sdk_pkg::nip_oa::compute_auth_tag(owner, &agent.public_key(), "")
        .expect("compute auth tag");
    (
        owner_hex,
        IdentityBinding {
            agent_pubkey: agent_hex.clone(),
            auth_tag,
        },
        agent_hex,
    )
}

/// A minimal healthy entry: `library_id`, origin `(scope, slug)`, one
/// `Materialized` projection in `scope`, and (optionally) one verified binding.
fn entry(library_id: &str, scope: &str, slug: &str, bind: Option<(&Keys, &Keys)>) -> LibraryEntry {
    let mut identity_bindings = BTreeMap::new();
    let mut owner_at_share = "00".repeat(32);
    if let Some((owner, agent)) = bind {
        let (owner_hex, b, _) = binding(owner, agent);
        owner_at_share = owner_hex.clone();
        identity_bindings.insert(owner_hex, b);
    }
    let mut projections = BTreeMap::new();
    projections.insert(
        scope.to_string(),
        ProjectionEntry {
            state: ProjectionState::Materialized { revision: 1 },
            local_slug: slug.to_string(),
            relay_url: "wss://relay.example".to_string(),
            workspace_label: Some("Home".to_string()),
        },
    );
    LibraryEntry {
        library_id: library_id.to_string(),
        origin: OriginKey {
            scope_id: scope.to_string(),
            slug: slug.to_string(),
        },
        revision: 1,
        deleted: false,
        owner_pubkey_at_share: owner_at_share,
        shared: SharedDefinition {
            display_name: "Aria".to_string(),
            avatar_url: None,
            system_prompt: "be helpful".to_string(),
            runtime: Some("acp".to_string()),
            model: None,
            provider: None,
            name_pool: vec!["Aria".to_string()],
            respond_to: Some("mentions".to_string()),
            respond_to_allowlist: vec![],
            parallelism: Some(1),
        },
        deferred_archives: vec![],
        identity_bindings,
        projections,
    }
}

fn value_of(entry: &LibraryEntry) -> Value {
    serde_json::to_value(entry).expect("serialize entry")
}

/// A projection claiming `local_slug` in some scope, in the given state.
fn proj(state: ProjectionState, local_slug: &str) -> ProjectionEntry {
    ProjectionEntry {
        state,
        local_slug: local_slug.to_string(),
        relay_url: "wss://relay.example".to_string(),
        workspace_label: None,
    }
}

fn doc(entries: Vec<Value>) -> LibraryDocument {
    LibraryDocument {
        version: SUPPORTED_LIBRARY_VERSION,
        entries,
        orphan_keys: vec![],
    }
}

fn write_doc(base: &std::path::Path, document: &LibraryDocument) {
    save_library_document(base, document).expect("save library.json");
}

// ── envelope: absent / version / corruption ────────────────────────────────────

#[test]
fn test_absent_library_loads_as_empty() {
    let dir = tempdir().unwrap();
    assert!(matches!(load_library(dir.path()), LibraryLoad::Empty));
}

#[test]
fn test_unknown_version_is_read_only_fail() {
    let dir = tempdir().unwrap();
    let mut document = doc(vec![]);
    document.version = 999;
    write_doc(dir.path(), &document);
    match load_library(dir.path()) {
        LibraryLoad::UnknownVersion(v) => assert_eq!(v, 999),
        other => panic!("expected UnknownVersion, got {other:?}"),
    }
    // Read-only fail preserves the file untouched — no `.invalid` snapshot.
    assert!(!library_path(dir.path())
        .with_extension("json.invalid")
        .exists());
}

#[test]
fn test_whole_document_corruption_is_preserved_and_blocks() {
    let dir = tempdir().unwrap();
    let path = library_path(dir.path());
    std::fs::write(&path, b"{ this is not json").unwrap();
    assert!(matches!(load_library(dir.path()), LibraryLoad::Corrupt));
    // Loud-failure discipline: the malformed bytes survive as `.invalid`.
    let invalid = path.with_extension("json.invalid");
    assert_eq!(std::fs::read(&invalid).unwrap(), b"{ this is not json");
    // And the original is left in place (copy, not rename).
    assert!(path.exists());
}

#[test]
fn test_saved_document_round_trips_at_0600() {
    let dir = tempdir().unwrap();
    let owner = Keys::generate();
    let agent = Keys::generate();
    let document = doc(vec![value_of(&entry(
        "lib-a",
        "scope-a",
        "aria",
        Some((&owner, &agent)),
    ))]);
    write_doc(dir.path(), &document);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(library_path(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    match load_library(dir.path()) {
        LibraryLoad::Loaded(loaded) => {
            assert_eq!(loaded.healthy.len(), 1);
            assert!(loaded.quarantined.is_empty());
            assert_eq!(loaded.healthy[0].library_id, "lib-a");
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
}

// ── quarantine: decode / semantic / identity index ─────────────────────────────

#[test]
fn test_unknown_field_entry_quarantines_while_sibling_stays_healthy() {
    let dir = tempdir().unwrap();
    let owner = Keys::generate();
    let agent = Keys::generate();
    let mut bad = value_of(&entry("lib-bad", "scope-b", "bob", None));
    bad.as_object_mut()
        .unwrap()
        .insert("surprise".to_string(), json!("unexpected"));
    let good = value_of(&entry(
        "lib-good",
        "scope-g",
        "gwen",
        Some((&owner, &agent)),
    ));
    write_doc(dir.path(), &doc(vec![bad.clone(), good]));

    match load_library(dir.path()) {
        LibraryLoad::Loaded(loaded) => {
            assert_eq!(loaded.healthy.len(), 1);
            assert_eq!(loaded.healthy[0].library_id, "lib-good");
            assert_eq!(loaded.quarantined, vec![bad]);
            assert_eq!(loaded.degradations.len(), 1);
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn test_binding_with_wrong_owner_key_quarantines() {
    let dir = tempdir().unwrap();
    let owner = Keys::generate();
    let other_owner = Keys::generate();
    let agent = Keys::generate();
    // Tag is valid under `owner`, but keyed under `other_owner` — owner mismatch.
    let mut e = entry("lib-mismatch", "scope-m", "mia", None);
    let (_, b, _) = binding(&owner, &agent);
    e.identity_bindings
        .insert(other_owner.public_key().to_hex(), b);
    let raw = value_of(&e);
    write_doc(dir.path(), &doc(vec![raw.clone()]));

    match load_library(dir.path()) {
        LibraryLoad::Loaded(loaded) => {
            assert!(loaded.healthy.is_empty());
            assert_eq!(loaded.quarantined, vec![raw]);
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn test_binding_with_wrong_agent_pubkey_quarantines() {
    let dir = tempdir().unwrap();
    let owner = Keys::generate();
    let agent = Keys::generate();
    let wrong_agent = Keys::generate();
    // Tag authorizes `agent`, but the binding claims `wrong_agent`.
    let owner_hex = owner.public_key().to_hex();
    let auth_tag = buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "").unwrap();
    let mut e = entry("lib-wrongagent", "scope-w", "wes", None);
    e.identity_bindings.insert(
        owner_hex,
        IdentityBinding {
            agent_pubkey: wrong_agent.public_key().to_hex(),
            auth_tag,
        },
    );
    let raw = value_of(&e);
    write_doc(dir.path(), &doc(vec![raw.clone()]));

    match load_library(dir.path()) {
        LibraryLoad::Loaded(loaded) => {
            assert!(loaded.healthy.is_empty());
            assert_eq!(loaded.quarantined, vec![raw]);
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn test_one_agent_bound_under_two_owners_quarantines() {
    let dir = tempdir().unwrap();
    let owner_a = Keys::generate();
    let owner_b = Keys::generate();
    let agent = Keys::generate();
    let mut e = entry("lib-two-owners", "scope-t", "tom", None);
    let (owner_a_hex, ba, _) = binding(&owner_a, &agent);
    let (owner_b_hex, bb, _) = binding(&owner_b, &agent);
    e.identity_bindings.insert(owner_a_hex, ba);
    e.identity_bindings.insert(owner_b_hex, bb);
    let raw = value_of(&e);
    write_doc(dir.path(), &doc(vec![raw.clone()]));

    match load_library(dir.path()) {
        LibraryLoad::Loaded(loaded) => {
            assert!(loaded.healthy.is_empty());
            assert_eq!(loaded.quarantined, vec![raw]);
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn test_agent_bound_under_two_owners_across_entries_group_quarantines() {
    let dir = tempdir().unwrap();
    let owner_a = Keys::generate();
    let owner_b = Keys::generate();
    let agent = Keys::generate();
    // Two individually-valid entries with distinct library_ids AND distinct
    // origins, each binding the SAME agent pubkey but under a DIFFERENT owner.
    // The keyring is process-global by pubkey, so this aliases one agent
    // identity across owner authorities — a document-wide collision that the
    // per-entry `seen_agents` pass cannot see (§2.5, spec lines 1761-1762).
    let a = value_of(&entry(
        "lib-cross-a",
        "scope-a",
        "aria",
        Some((&owner_a, &agent)),
    ));
    let b = value_of(&entry(
        "lib-cross-b",
        "scope-b",
        "bram",
        Some((&owner_b, &agent)),
    ));
    // A healthy sibling binding a DIFFERENT agent stays fully usable.
    let sibling_owner = Keys::generate();
    let sibling_agent = Keys::generate();
    let sibling = value_of(&entry(
        "lib-sibling",
        "scope-s",
        "sara",
        Some((&sibling_owner, &sibling_agent)),
    ));
    write_doc(dir.path(), &doc(vec![a.clone(), b.clone(), sibling]));

    match load_library(dir.path()) {
        LibraryLoad::Loaded(loaded) => {
            assert_eq!(loaded.healthy.len(), 1);
            assert_eq!(loaded.healthy[0].library_id, "lib-sibling");
            assert_eq!(loaded.quarantined.len(), 2);
            assert!(loaded.quarantined.contains(&a));
            assert!(loaded.quarantined.contains(&b));
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn test_same_owner_agent_reuse_across_entries_stays_healthy() {
    let dir = tempdir().unwrap();
    let owner = Keys::generate();
    let agent = Keys::generate();
    // The SAME owner binding the SAME agent across multiple entries is the
    // identity auto-carry across a same-owner's workspaces (§2.5) — explicitly
    // permitted, NOT a cross-owner alias. Neither entry may quarantine.
    let a = value_of(&entry(
        "lib-reuse-a",
        "scope-a",
        "aria",
        Some((&owner, &agent)),
    ));
    let b = value_of(&entry(
        "lib-reuse-b",
        "scope-b",
        "bram",
        Some((&owner, &agent)),
    ));
    write_doc(dir.path(), &doc(vec![a, b]));

    match load_library(dir.path()) {
        LibraryLoad::Loaded(loaded) => assert_eq!(loaded.healthy.len(), 2),
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn test_noncanonical_agent_spelling_cannot_alias_across_owners() {
    let dir = tempdir().unwrap();
    let owner_a = Keys::generate();
    let owner_b = Keys::generate();
    let agent = Keys::generate();
    // Entry A binds agent P (canonical lowercase) under owner A. Entry B binds
    // the SAME P but spelled UPPERCASE under owner B, with a still-VALID auth
    // tag (computed over the canonical key). Before the fix, the raw-string
    // index saw two distinct agent keys and quarantined neither — the §2.5
    // cross-owner process-global alias. Now B fails canonical validation at
    // pass 1 (the only rejection cause is the encoding), so the alias can never
    // reach the healthy set; A stays usable.
    let a = value_of(&entry(
        "lib-canon-a",
        "scope-a",
        "aria",
        Some((&owner_a, &agent)),
    ));
    let mut e_b = entry("lib-canon-b", "scope-b", "bram", Some((&owner_b, &agent)));
    for binding in e_b.identity_bindings.values_mut() {
        binding.agent_pubkey = binding.agent_pubkey.to_uppercase();
    }
    let b = value_of(&e_b);
    write_doc(dir.path(), &doc(vec![a.clone(), b.clone()]));

    match load_library(dir.path()) {
        LibraryLoad::Loaded(loaded) => {
            assert_eq!(loaded.healthy.len(), 1);
            assert_eq!(loaded.healthy[0].library_id, "lib-canon-a");
            assert!(loaded.quarantined.contains(&b));
            assert!(loaded.degradations.iter().any(|d| d.contains("canonical")));
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn test_same_owner_noncanonical_spelling_is_not_a_false_cross_owner_collision() {
    let dir = tempdir().unwrap();
    let owner = Keys::generate();
    let agent = Keys::generate();
    // ONE owner, spelled canonically in entry A and UPPERCASE (re-keyed) in
    // entry B. A raw-string index would read the two owner spellings as two
    // different owners and falsely group-quarantine the healthy same-owner
    // reuse. Under canonical enforcement B is rejected for its encoding and A
    // survives — the valid reuse is NOT dragged down by a phantom collision.
    let a = value_of(&entry(
        "lib-owner-canon-a",
        "scope-a",
        "aria",
        Some((&owner, &agent)),
    ));
    let mut e_b = entry(
        "lib-owner-canon-b",
        "scope-b",
        "bram",
        Some((&owner, &agent)),
    );
    let (owner_hex, binding) = e_b
        .identity_bindings
        .iter()
        .next()
        .map(|(k, v)| (k.clone(), v.clone()))
        .expect("one binding");
    e_b.identity_bindings.clear();
    e_b.identity_bindings
        .insert(owner_hex.to_uppercase(), binding);
    let b = value_of(&e_b);
    write_doc(dir.path(), &doc(vec![a, b.clone()]));

    match load_library(dir.path()) {
        LibraryLoad::Loaded(loaded) => {
            assert_eq!(loaded.healthy.len(), 1);
            assert_eq!(loaded.healthy[0].library_id, "lib-owner-canon-a");
            assert!(loaded.quarantined.contains(&b));
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn test_duplicate_library_id_group_quarantines_all_colliders() {
    let dir = tempdir().unwrap();
    let a = value_of(&entry("lib-dup", "scope-1", "one", None));
    let b = value_of(&entry("lib-dup", "scope-2", "two", None));
    let unrelated = value_of(&entry("lib-solo", "scope-3", "three", None));
    write_doc(dir.path(), &doc(vec![a.clone(), b.clone(), unrelated]));

    match load_library(dir.path()) {
        LibraryLoad::Loaded(loaded) => {
            assert_eq!(loaded.healthy.len(), 1);
            assert_eq!(loaded.healthy[0].library_id, "lib-solo");
            assert_eq!(loaded.quarantined.len(), 2);
            assert!(loaded.quarantined.contains(&a));
            assert!(loaded.quarantined.contains(&b));
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn test_duplicate_live_origin_group_quarantines() {
    let dir = tempdir().unwrap();
    // Distinct library_ids, same LIVE origin — share resumption is ambiguous.
    let mut a = entry("lib-x", "scope-shared", "sameslug", None);
    let mut b = entry("lib-y", "scope-shared", "sameslug", None);
    // Give them distinct non-terminal projections so ONLY the origin collides.
    a.projections.clear();
    b.projections.clear();
    let a = value_of(&a);
    let b = value_of(&b);
    let unrelated = value_of(&entry("lib-z", "scope-else", "elsewhere", None));
    write_doc(dir.path(), &doc(vec![a.clone(), b.clone(), unrelated]));

    match load_library(dir.path()) {
        LibraryLoad::Loaded(loaded) => {
            assert_eq!(loaded.healthy.len(), 1);
            assert_eq!(loaded.healthy[0].library_id, "lib-z");
            assert_eq!(loaded.quarantined.len(), 2);
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn test_deleted_origin_does_not_collide() {
    let dir = tempdir().unwrap();
    // Same origin, but one entry is a tombstone — a `deleted` origin is not live,
    // so there is no live-origin collision.
    let mut live = entry("lib-live", "scope-o", "shared", None);
    live.projections.clear();
    let mut tomb = entry("lib-tomb", "scope-o", "shared", None);
    tomb.deleted = true;
    tomb.projections.clear();
    write_doc(dir.path(), &doc(vec![value_of(&live), value_of(&tomb)]));

    match load_library(dir.path()) {
        LibraryLoad::Loaded(loaded) => assert_eq!(loaded.healthy.len(), 2),
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn test_conflicting_non_terminal_scope_slug_group_quarantines() {
    let dir = tempdir().unwrap();
    // Distinct library_ids AND distinct origins, but both non-terminal
    // projections claim `(scope-c, dup-slug)` — a scope would be rewritten from
    // an arbitrary entry.
    let mut a = entry("lib-p", "origin-a", "slug-a", None);
    let mut b = entry("lib-q", "origin-b", "slug-b", None);
    a.projections = BTreeMap::from([(
        "scope-c".to_string(),
        proj(ProjectionState::Materialized { revision: 1 }, "dup-slug"),
    )]);
    b.projections = BTreeMap::from([(
        "scope-c".to_string(),
        proj(ProjectionState::Pending, "dup-slug"),
    )]);
    write_doc(dir.path(), &doc(vec![value_of(&a), value_of(&b)]));

    match load_library(dir.path()) {
        LibraryLoad::Loaded(loaded) => {
            assert!(loaded.healthy.is_empty());
            assert_eq!(loaded.quarantined.len(), 2);
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn test_terminal_projection_claims_do_not_collide() {
    let dir = tempdir().unwrap();
    // Same `(scope, slug)` claim but both terminal — terminal claims never
    // conflict (the index only guards non-terminal ownership); distinct origins
    // keep origin out of it.
    let mut a = entry("lib-ta", "origin-ta", "slug-ta", None);
    let mut b = entry("lib-tb", "origin-tb", "slug-tb", None);
    a.projections = BTreeMap::from([(
        "scope-c".to_string(),
        proj(ProjectionState::Excluded, "dup-slug"),
    )]);
    b.projections = BTreeMap::from([(
        "scope-c".to_string(),
        proj(ProjectionState::Deleted, "dup-slug"),
    )]);
    write_doc(dir.path(), &doc(vec![value_of(&a), value_of(&b)]));

    match load_library(dir.path()) {
        LibraryLoad::Loaded(loaded) => assert_eq!(loaded.healthy.len(), 2),
        other => panic!("expected Loaded, got {other:?}"),
    }
}

// ── P2-I2 acceptance: preservation across a healthy rewrite ─────────────────────

#[test]
fn test_quarantined_entry_preserved_value_equivalently_across_edit_and_restart() {
    let dir = tempdir().unwrap();
    let owner = Keys::generate();
    let agent = Keys::generate();
    let mut bad = value_of(&entry("lib-quar", "scope-q", "quinn", None));
    bad.as_object_mut()
        .unwrap()
        .insert("mystery".to_string(), json!({"nested": [1, 2, 3]}));
    let good = entry("lib-edit", "scope-e", "evan", Some((&owner, &agent)));
    write_doc(dir.path(), &doc(vec![bad.clone(), value_of(&good)]));

    // Load, edit the healthy entry, rebuild, and persist — the quarantined raw
    // must ride through unchanged.
    let mut loaded = match load_library(dir.path()) {
        LibraryLoad::Loaded(l) => l,
        other => panic!("expected Loaded, got {other:?}"),
    };
    loaded.healthy[0].revision = 2;
    let rebuilt = loaded.rebuild_document().expect("rebuild");
    write_doc(dir.path(), &rebuilt);

    match load_library(dir.path()) {
        LibraryLoad::Loaded(l) => {
            assert_eq!(l.healthy.len(), 1);
            assert_eq!(l.healthy[0].revision, 2);
            assert_eq!(l.quarantined, vec![bad]);
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
}

// ── §2.2: apply_shared_definition writes only shared slots ──────────────────────

#[test]
fn test_apply_shared_definition_writes_only_shared_slots() {
    let mut record = minimal_definition().into_agent_record();
    record.pubkey = "agent-pubkey".to_string();
    record.private_key_nsec = "nsec-secret".to_string();
    record.is_active = true;
    record.env_vars = BTreeMap::from([("SECRET".to_string(), "value".to_string())]);
    record.library_ref = Some("lib-existing".to_string());
    record.backend_agent_id = Some("backend-123".to_string());
    record.last_completed_deploy_attempt_id = Some("attempt-9".to_string());
    let shared = SharedDefinition {
        display_name: "Renamed".to_string(),
        avatar_url: Some("https://a.example/x.png".to_string()),
        system_prompt: "new prompt".to_string(),
        runtime: Some("acp".to_string()),
        model: Some("gpt".to_string()),
        provider: Some("openai".to_string()),
        name_pool: vec!["Renamed".to_string()],
        respond_to: Some("all".to_string()),
        respond_to_allowlist: vec!["npub1".to_string()],
        parallelism: Some(3),
    };
    apply_shared_definition(&mut record, &shared, 7);

    // Shared slots overwritten.
    assert_eq!(record.name, "Renamed");
    assert_eq!(record.display_name.as_deref(), Some("Renamed"));
    assert_eq!(record.system_prompt.as_deref(), Some("new prompt"));
    assert_eq!(record.model.as_deref(), Some("gpt"));
    assert_eq!(record.definition_respond_to.as_deref(), Some("all"));
    assert_eq!(record.definition_parallelism, Some(3));
    assert_eq!(record.library_applied_revision, Some(7));
    // Identity, secrets, activation, linkage, deploy provenance untouched.
    assert_eq!(record.pubkey, "agent-pubkey");
    assert_eq!(record.private_key_nsec, "nsec-secret");
    assert!(record.is_active);
    assert_eq!(
        record.env_vars,
        BTreeMap::from([("SECRET".to_string(), "value".to_string())])
    );
    assert_eq!(record.library_ref.as_deref(), Some("lib-existing"));
    assert_eq!(record.backend_agent_id.as_deref(), Some("backend-123"));
    assert_eq!(
        record.last_completed_deploy_attempt_id.as_deref(),
        Some("attempt-9")
    );
}

#[test]
fn test_apply_shared_definition_empty_prompt_maps_to_none() {
    let mut record = minimal_definition().into_agent_record();
    let shared = SharedDefinition {
        display_name: "N".to_string(),
        avatar_url: None,
        system_prompt: String::new(),
        runtime: None,
        model: None,
        provider: None,
        name_pool: vec![],
        respond_to: None,
        respond_to_allowlist: vec![],
        parallelism: None,
    };
    apply_shared_definition(&mut record, &shared, 1);
    // Mirrors `into_agent_record`: an empty prompt becomes `None`, not `Some("")`.
    assert_eq!(record.system_prompt, None);
}

// ── §2.3: deferred_archives SET semantics (P15-MINOR) ───────────────────────────

#[test]
fn test_upsert_deferred_archive_is_idempotent_across_retries() {
    let mut e = entry("lib-def", "scope-d", "dora", None);
    // Simulate §3.6's crash-then-re-consent retry: the same obligation write
    // re-runs three times. SET semantics keep exactly one marker.
    for _ in 0..3 {
        e.upsert_deferred_archive("scope-x".to_string(), "agent-pk".to_string());
    }
    assert_eq!(e.deferred_archives.len(), 1);
    assert_eq!(e.deferred_archive_obligations().len(), 1);
}

#[test]
fn test_upsert_deferred_archive_reports_new_versus_existing() {
    let mut e = entry("lib-def", "scope-d", "dora", None);
    assert!(e.upsert_deferred_archive("scope-x".to_string(), "agent-a".to_string()));
    // Same coordinate → no-op.
    assert!(!e.upsert_deferred_archive("scope-x".to_string(), "agent-a".to_string()));
    // A different scope or agent is a distinct obligation.
    assert!(e.upsert_deferred_archive("scope-y".to_string(), "agent-a".to_string()));
    assert!(e.upsert_deferred_archive("scope-x".to_string(), "agent-b".to_string()));
    assert_eq!(e.deferred_archives.len(), 3);
}

#[test]
fn test_legacy_duplicate_deferred_archives_read_as_one_obligation() {
    // A legacy on-disk entry with duplicate rows (written before the upsert
    // API existed) must READ as one obligation per (scope_id, agent_pubkey).
    let mut e = entry("lib-legacy", "scope-l", "leo", None);
    e.deferred_archives = vec![
        DeferredArchive {
            scope_id: "scope-x".to_string(),
            agent_pubkey: "agent-a".to_string(),
        },
        DeferredArchive {
            scope_id: "scope-x".to_string(),
            agent_pubkey: "agent-a".to_string(),
        },
        DeferredArchive {
            scope_id: "scope-y".to_string(),
            agent_pubkey: "agent-a".to_string(),
        },
    ];
    let obligations = e.deferred_archive_obligations();
    assert_eq!(obligations.len(), 2);
    assert!(obligations
        .iter()
        .any(|d| d.scope_id == "scope-x" && d.agent_pubkey == "agent-a"));
    assert!(obligations
        .iter()
        .any(|d| d.scope_id == "scope-y" && d.agent_pubkey == "agent-a"));
}

// ── scope-local journals (P9-I1 / P10-C3) ───────────────────────────────────────

#[test]
fn test_absent_journals_load_as_empty() {
    let dir = tempdir().unwrap();
    assert!(matches!(
        load_pending_keys(dir.path()),
        PendingKeysLoad::Empty
    ));
    assert!(matches!(
        load_deploy_intents(dir.path()),
        DeployIntentsLoad::Empty
    ));
}

#[test]
fn test_pending_keys_round_trip() {
    let dir = tempdir().unwrap();
    let journal = PendingKeysJournal {
        version: SUPPORTED_JOURNAL_VERSION,
        pending: vec!["pk1".to_string(), "pk2".to_string()],
    };
    save_pending_keys(dir.path(), &journal).unwrap();
    match load_pending_keys(dir.path()) {
        PendingKeysLoad::Loaded(j) => assert_eq!(j, journal),
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn test_pending_keys_unknown_version_is_unreadable() {
    let dir = tempdir().unwrap();
    std::fs::write(
        pending_keys_path(dir.path()),
        br#"{"version":2,"pending":[]}"#,
    )
    .unwrap();
    assert!(matches!(
        load_pending_keys(dir.path()),
        PendingKeysLoad::Unreadable(_)
    ));
}

#[test]
fn test_deploy_intents_round_trip_preserves_phase() {
    let dir = tempdir().unwrap();
    let journal = DeployIntentsJournal {
        version: SUPPORTED_JOURNAL_VERSION,
        intents: vec![
            DeployIntent {
                agent_pubkey: "pk-run".to_string(),
                phase: DeployIntentPhase::Running {
                    attempt_id: "attempt-1".to_string(),
                },
                provider_id: "fly".to_string(),
                provider_config: json!({"region": "iad"}),
                created_at: "2026-08-11T00:00:00Z".to_string(),
            },
            DeployIntent {
                agent_pubkey: "pk-consent".to_string(),
                phase: DeployIntentPhase::OrphanConsentPendingRemoval {
                    consent_id: "consent-1".to_string(),
                    cleanup: ConsentCleanup {
                        tombstone_required: true,
                        archive_required: false,
                        deferred_archive_entries: vec!["lib-a".to_string()],
                        projected: Some(ProjectedCleanupRef {
                            library_id: "lib-a".to_string(),
                            local_slug: "lib-a-slug".to_string(),
                            obligation: ProjectedObligation::ManifestMembership,
                        }),
                    },
                },
                provider_id: "fly".to_string(),
                provider_config: json!({"region": "sjc"}),
                created_at: "2026-08-11T00:01:00Z".to_string(),
            },
        ],
    };
    save_deploy_intents(dir.path(), &journal).unwrap();
    match load_deploy_intents(dir.path()) {
        DeployIntentsLoad::Loaded(j) => assert_eq!(j, journal),
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn test_deploy_intents_duplicate_pubkey_group_rejects_whole_journal() {
    let dir = tempdir().unwrap();
    let row = |pubkey: &str| DeployIntent {
        agent_pubkey: pubkey.to_string(),
        phase: DeployIntentPhase::Running {
            attempt_id: "a".to_string(),
        },
        provider_id: "fly".to_string(),
        provider_config: json!({}),
        created_at: "t".to_string(),
    };
    let journal = DeployIntentsJournal {
        version: SUPPORTED_JOURNAL_VERSION,
        intents: vec![row("dup"), row("dup"), row("other")],
    };
    save_deploy_intents(dir.path(), &journal).unwrap();
    // The at-most-one mutex is validated on read: any duplicate rejects the
    // WHOLE journal, never first-wins (P10-C3).
    match load_deploy_intents(dir.path()) {
        DeployIntentsLoad::Unreadable(reason) => assert!(reason.contains("dup")),
        other => panic!("expected Unreadable, got {other:?}"),
    }
}

#[test]
fn test_deploy_intents_unknown_field_is_unreadable() {
    let dir = tempdir().unwrap();
    std::fs::write(
        deploy_intents_path(dir.path()),
        br#"{"version":1,"intents":[],"surprise":true}"#,
    )
    .unwrap();
    assert!(matches!(
        load_deploy_intents(dir.path()),
        DeployIntentsLoad::Unreadable(_)
    ));
}

/// A `Running` deploy row with the given `provider_config`, for routing-
/// validation fixtures.
fn intent_with_config(pubkey: &str, provider_config: Value) -> DeployIntent {
    DeployIntent {
        agent_pubkey: pubkey.to_string(),
        phase: DeployIntentPhase::Running {
            attempt_id: "a".to_string(),
        },
        provider_id: "fly".to_string(),
        provider_config,
        created_at: "t".to_string(),
    }
}

#[test]
fn test_deploy_intents_non_object_provider_config_is_unreadable() {
    let dir = tempdir().unwrap();
    // A hand-edited row whose provider_config is a bare scalar, not an object.
    // `validate_provider_config` rejects it; the row must not be exposed as
    // authoritative routing (§2.1 row contract).
    std::fs::write(
        deploy_intents_path(dir.path()),
        serde_json::to_vec(&DeployIntentsJournal {
            version: SUPPORTED_JOURNAL_VERSION,
            intents: vec![intent_with_config("pk", json!("us-east"))],
        })
        .unwrap(),
    )
    .unwrap();
    match load_deploy_intents(dir.path()) {
        DeployIntentsLoad::Unreadable(reason) => assert!(reason.contains("provider_config")),
        other => panic!("expected Unreadable, got {other:?}"),
    }
}

#[test]
fn test_deploy_intents_nested_provider_config_is_unreadable() {
    let dir = tempdir().unwrap();
    // Nested (non-scalar) values are rejected — routing must be a flat object.
    std::fs::write(
        deploy_intents_path(dir.path()),
        serde_json::to_vec(&DeployIntentsJournal {
            version: SUPPORTED_JOURNAL_VERSION,
            intents: vec![intent_with_config("pk", json!({"region": {"nested": 1}}))],
        })
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        load_deploy_intents(dir.path()),
        DeployIntentsLoad::Unreadable(_)
    ));
}

#[test]
fn test_deploy_intents_secret_key_provider_config_is_unreadable() {
    let dir = tempdir().unwrap();
    // A secret-like key is rejected — provider_config is never secret (§2.1).
    std::fs::write(
        deploy_intents_path(dir.path()),
        serde_json::to_vec(&DeployIntentsJournal {
            version: SUPPORTED_JOURNAL_VERSION,
            intents: vec![intent_with_config("pk", json!({"api_key": "leak"}))],
        })
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        load_deploy_intents(dir.path()),
        DeployIntentsLoad::Unreadable(_)
    ));
}

#[test]
fn test_deploy_intents_valid_scalar_object_provider_config_loads() {
    let dir = tempdir().unwrap();
    // A flat object of scalar, non-secret values is the healthy shape.
    let journal = DeployIntentsJournal {
        version: SUPPORTED_JOURNAL_VERSION,
        intents: vec![intent_with_config(
            "pk",
            json!({"region": "iad", "size": 2}),
        )],
    };
    save_deploy_intents(dir.path(), &journal).unwrap();
    match load_deploy_intents(dir.path()) {
        DeployIntentsLoad::Loaded(j) => assert_eq!(j, journal),
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn test_save_deploy_intents_rejects_invalid_provider_config() {
    let dir = tempdir().unwrap();
    // The writer boundary refuses an invalid row, so no crate-internal caller
    // can persist state a later read would fail closed against.
    let journal = DeployIntentsJournal {
        version: SUPPORTED_JOURNAL_VERSION,
        intents: vec![intent_with_config("pk", json!({"secret": "x"}))],
    };
    assert!(save_deploy_intents(dir.path(), &journal).is_err());
    assert!(!deploy_intents_path(dir.path()).exists());
}

#[test]
fn test_journals_written_at_0600() {
    let dir = tempdir().unwrap();
    save_pending_keys(dir.path(), &PendingKeysJournal::empty()).unwrap();
    save_deploy_intents(dir.path(), &DeployIntentsJournal::empty()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [
            pending_keys_path(dir.path()),
            deploy_intents_path(dir.path()),
        ] {
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{}", path.display());
        }
    }
}
