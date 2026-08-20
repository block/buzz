//! Phase-3 §2.5 pure identity-binding helper tests: the `key_archive_protected`
//! deletion-safety predicate (P13-C1/P17-C1), deterministic `select_binding_seed`
//! (P3-I2), and the crash-safe `mint_bound_identity` order (P5-I1). The mint
//! transaction WIRING (the atomic step-4 commit into `insert_linked_instance`)
//! lands later (Phase 4b). Kept in their own module so the Phase-1 `tests.rs`
//! suite stays clear of the 1000-line file ratchet.

use serde_json::json;

use super::*;
use crate::secret_store::KeyringProbe;
use nostr::{Keys, ToBech32};
use std::cell::RefCell;
use std::collections::HashMap;

// ── key_archive_protected ──────────────────────────────────────────────────────

/// A raw entry `Value` binding `agent_pubkey` under one owner, with the given
/// `deleted` tombstone flag. Only the fields the predicate reads are populated —
/// the predicate scans raw entries, so a partial value is a faithful stand-in
/// for both a healthy and a quarantined on-disk entry. The owner map key is a
/// fixed placeholder: the predicate reads binding values, never the key.
fn bound_entry(agent_pubkey: &str, deleted: bool) -> serde_json::Value {
    json!({
        "deleted": deleted,
        "identity_bindings": {
            "owner-key-placeholder": { "agent_pubkey": agent_pubkey, "auth_tag": "{}" }
        }
    })
}

fn deferred_entry(agent_pubkey: &str) -> serde_json::Value {
    json!({
        "deferred_archives": [ { "scope_id": "scope-1", "agent_pubkey": agent_pubkey } ]
    })
}

fn library(entries: Vec<serde_json::Value>) -> LibraryDocument {
    LibraryDocument {
        version: SUPPORTED_LIBRARY_VERSION,
        entries,
        orphan_keys: vec![],
    }
}

#[test]
fn test_ever_bound_live_entry_protects_key() {
    let target = "aa".repeat(32);
    let doc = library(vec![bound_entry(&target, false)]);
    assert!(doc.key_archive_protected(&target));
}

#[test]
fn test_ever_bound_tombstoned_entry_still_protects_key() {
    // Tombstones are kept forever (P1-OQ2); a deleted entry that once bound the
    // pubkey must keep protecting it — the global keyring identity outlives the
    // tombstone.
    let target = "bb".repeat(32);
    let doc = library(vec![bound_entry(&target, true)]);
    assert!(doc.key_archive_protected(&target));
}

#[test]
fn test_quarantined_entry_binding_pubkey_protects_key() {
    // A structurally invalid entry (would be quarantined on decode) that still
    // names the pubkey in identity_bindings must protect it: the predicate scans
    // RAW entries so quarantine can never strip protection.
    let target = "cc".repeat(32);
    let garbage = json!({
        "not_a_real_field": 7,
        "identity_bindings": {
            "owner-key-placeholder": { "agent_pubkey": target, "auth_tag": "{}" }
        }
    });
    let doc = library(vec![garbage]);
    assert!(doc.key_archive_protected(&target));
}

#[test]
fn test_outstanding_deferred_archive_protects_key() {
    // A deferred-archive obligation with no surviving binding must still protect:
    // the retirement marker is the record that the key was never archived.
    let target = "dd".repeat(32);
    let doc = library(vec![deferred_entry(&target)]);
    assert!(doc.key_archive_protected(&target));
}

#[test]
fn test_unrelated_entry_does_not_protect_key() {
    let target = "ee".repeat(32);
    let other = "11".repeat(32);
    let doc = library(vec![bound_entry(&other, false), deferred_entry(&other)]);
    assert!(!doc.key_archive_protected(&target));
}

#[test]
fn test_empty_library_protects_nothing() {
    let doc = library(vec![]);
    assert!(!doc.key_archive_protected(&"22".repeat(32)));
}

// ── select_binding_seed ─────────────────────────────────────────────────────────

#[test]
fn test_seed_picks_earliest_created() {
    let seed = select_binding_seed(vec![
        SeedCandidate {
            created_at: "2026-08-11T10:00:00Z",
            pubkey: "aaa",
        },
        SeedCandidate {
            created_at: "2026-08-11T09:00:00Z",
            pubkey: "bbb",
        },
        SeedCandidate {
            created_at: "2026-08-11T11:00:00Z",
            pubkey: "ccc",
        },
    ]);
    assert_eq!(seed, Some("bbb"));
}

#[test]
fn test_seed_breaks_created_tie_by_lowest_pubkey() {
    let seed = select_binding_seed(vec![
        SeedCandidate {
            created_at: "2026-08-11T09:00:00Z",
            pubkey: "ffff",
        },
        SeedCandidate {
            created_at: "2026-08-11T09:00:00Z",
            pubkey: "0001",
        },
        SeedCandidate {
            created_at: "2026-08-11T09:00:00Z",
            pubkey: "abcd",
        },
    ]);
    assert_eq!(seed, Some("0001"));
}

#[test]
fn test_seed_single_instance() {
    let seed = select_binding_seed(vec![SeedCandidate {
        created_at: "2026-08-11T09:00:00Z",
        pubkey: "solo",
    }]);
    assert_eq!(seed, Some("solo"));
}

#[test]
fn test_seed_empty_is_none() {
    assert_eq!(select_binding_seed(Vec::new()), None);
}

// ── build_identity_binding ──────────────────────────────────────────────────────

#[test]
fn test_build_binding_derives_pubkey_from_nsec_and_verifies_on_read() {
    // The binding a fresh mint produces must pass validate_entry_bindings on the
    // next read with no special case: agent_pubkey derived from the read-back
    // nsec, a fresh tag embedding exactly the owner it is keyed under.
    let owner = Keys::generate();
    let agent = Keys::generate();
    let nsec = agent.secret_key().to_bech32().expect("encode nsec");

    let (owner_hex, binding) = build_identity_binding(&owner, &nsec).expect("build binding");

    assert_eq!(owner_hex, owner.public_key().to_hex());
    assert_eq!(binding.agent_pubkey, agent.public_key().to_hex());
    // Feed it straight into the read-side validator via a full entry.
    let mut bindings = std::collections::BTreeMap::new();
    bindings.insert(owner_hex.clone(), binding.clone());
    let entry = LibraryEntry {
        library_id: "lib-1".into(),
        origin: OriginKey {
            scope_id: "s".into(),
            slug: "a".into(),
        },
        revision: 1,
        deleted: false,
        owner_pubkey_at_share: owner_hex,
        shared: SharedDefinition {
            display_name: "A".into(),
            avatar_url: None,
            system_prompt: "p".into(),
            runtime: None,
            model: None,
            provider: None,
            name_pool: vec![],
            respond_to: None,
            respond_to_allowlist: vec![],
            parallelism: None,
        },
        deferred_archives: vec![],
        identity_bindings: bindings,
        projections: std::collections::BTreeMap::new(),
    };
    validate_entry_bindings(&entry).expect("freshly built binding validates on read");
}

#[test]
fn test_build_binding_ignores_stored_tag_uses_read_back_nsec() {
    // agent_pubkey comes from the nsec, never a caller-supplied record: two
    // different owners over the same agent nsec yield the same agent_pubkey with
    // different, each-valid tags.
    let owner_a = Keys::generate();
    let owner_b = Keys::generate();
    let agent = Keys::generate();
    let nsec = agent.secret_key().to_bech32().expect("encode nsec");

    let (_, ba) = build_identity_binding(&owner_a, &nsec).expect("a");
    let (_, bb) = build_identity_binding(&owner_b, &nsec).expect("b");

    assert_eq!(ba.agent_pubkey, agent.public_key().to_hex());
    assert_eq!(bb.agent_pubkey, agent.public_key().to_hex());
    assert_ne!(ba.auth_tag, bb.auth_tag);
}

#[test]
fn test_build_binding_rejects_garbage_nsec() {
    let owner = Keys::generate();
    assert!(build_identity_binding(&owner, "not-a-real-nsec").is_err());
}

// ── orphan-key journal + recovery selection ─────────────────────────────────────

#[test]
fn test_journal_orphan_is_idempotent() {
    let mut doc = library(vec![]);
    doc.journal_orphan_pubkey("aa");
    doc.journal_orphan_pubkey("aa");
    assert_eq!(doc.orphan_keys, vec!["aa".to_string()]);
}

#[test]
fn test_remove_orphan_drops_row_and_is_noop_when_absent() {
    let mut doc = library(vec![]);
    doc.journal_orphan_pubkey("aa");
    doc.journal_orphan_pubkey("bb");
    doc.remove_orphan_pubkey("aa");
    assert_eq!(doc.orphan_keys, vec!["bb".to_string()]);
    doc.remove_orphan_pubkey("missing");
    assert_eq!(doc.orphan_keys, vec!["bb".to_string()]);
}

#[test]
fn test_unreferenced_orphans_reaps_only_uncommitted() {
    // committed: a live binding references it → NOT reaped.
    // dangling: journaled but no binding → reaped (crash between step 2 and 4).
    let committed = "aa".repeat(32);
    let dangling = "bb".repeat(32);
    let mut doc = library(vec![bound_entry(&committed, false)]);
    doc.journal_orphan_pubkey(&committed);
    doc.journal_orphan_pubkey(&dangling);

    let reap = doc.unreferenced_orphans();
    assert_eq!(reap, vec![dangling.as_str()]);
}

#[test]
fn test_unreferenced_orphans_ignores_deferred_only_reference() {
    // A deferred-archive marker is NOT a live binding: a journaled orphan whose
    // only mention is a deferred row is still uncommitted and must be reaped.
    let target = "cc".repeat(32);
    let mut doc = library(vec![deferred_entry(&target)]);
    doc.journal_orphan_pubkey(&target);
    assert_eq!(doc.unreferenced_orphans(), vec![target.as_str()]);
}

// ── mint_bound_identity: crash-safe order (§2.5, P5-I1) ─────────────────────────

/// In-memory [`KeyStore`] recording writes/reads so a test can assert the
/// keyring was (or was not) touched. `fail_write` fails `write_and_verify`
/// after recording the attempt; the map is only populated on success, matching
/// the real read-back-verified contract.
struct FakeKeyStore {
    stored: RefCell<HashMap<String, String>>,
    fail_write: bool,
    fail_delete: bool,
    /// `load` returns `Ok(None)` even after a verified write — models a keyring
    /// where the minted key vanishes between write-back and read-back (§2.5
    /// step 4's "minted key absent immediately after verified write" arm).
    load_misses: bool,
    writes: RefCell<usize>,
    deletes: RefCell<usize>,
}

impl FakeKeyStore {
    fn ok() -> Self {
        Self {
            stored: RefCell::new(HashMap::new()),
            fail_write: false,
            fail_delete: false,
            load_misses: false,
            writes: RefCell::new(0),
            deletes: RefCell::new(0),
        }
    }
    fn write_fails() -> Self {
        Self {
            fail_write: true,
            ..Self::ok()
        }
    }
    fn delete_fails() -> Self {
        Self {
            fail_delete: true,
            ..Self::ok()
        }
    }
    fn load_misses() -> Self {
        Self {
            load_misses: true,
            ..Self::ok()
        }
    }
    fn with_key(self, name: &str, value: &str) -> Self {
        self.stored
            .borrow_mut()
            .insert(name.to_string(), value.to_string());
        self
    }
}

impl crate::managed_agents::storage::KeyStore for FakeKeyStore {
    fn probe(&self, _name: &str) -> KeyringProbe {
        KeyringProbe::ReachableButEmpty
    }
    fn load(&self, name: &str) -> Result<Option<String>, String> {
        if self.load_misses {
            return Ok(None);
        }
        Ok(self.stored.borrow().get(name).cloned())
    }
    fn load_all_readonly(&self) -> Result<Option<HashMap<String, String>>, String> {
        let map = self.stored.borrow().clone();
        Ok((!map.is_empty()).then_some(map))
    }
    fn write_and_verify(&self, name: &str, value: &str) -> Result<(), String> {
        *self.writes.borrow_mut() += 1;
        if self.fail_write {
            return Err("read-back verify failed".to_string());
        }
        self.stored
            .borrow_mut()
            .insert(name.to_string(), value.to_string());
        Ok(())
    }
    fn store_all(&self, entries: &HashMap<String, String>) -> Result<(), String> {
        self.stored
            .borrow_mut()
            .extend(entries.iter().map(|(k, v)| (k.clone(), v.clone())));
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        *self.deletes.borrow_mut() += 1;
        if self.fail_delete {
            return Err("keyring backend unreachable".to_string());
        }
        self.stored.borrow_mut().remove(name);
        Ok(())
    }
}

#[test]
fn test_mint_journals_before_keyring_and_binding_validates_on_read() {
    // Happy path: keypair minted, orphan journaled + persisted BEFORE the
    // keyring write, binding built from the read-back nsec passes the read-side
    // validator. The orphan row is left for the caller's atomic step-4 commit.
    let owner = Keys::generate();
    let store = FakeKeyStore::ok();
    let mut doc = library(vec![]);
    let saves: RefCell<usize> = RefCell::new(0);

    let minted = mint_bound_identity(&mut doc, &owner, &store, |d| {
        *saves.borrow_mut() += 1;
        // The pubkey is durably journaled at persist time, before any secret.
        assert_eq!(d.orphan_keys.len(), 1);
        assert_eq!(
            *store.writes.borrow(),
            0,
            "keyring untouched at journal persist"
        );
        Ok(())
    })
    .expect("mint");

    assert_eq!(
        *saves.borrow(),
        1,
        "exactly one durable checkpoint (step 2)"
    );
    assert_eq!(minted.owner_hex, owner.public_key().to_hex());
    assert_eq!(
        minted.binding.agent_pubkey,
        minted.keys.public_key().to_hex()
    );
    // Orphan row still present — the caller drops it in the atomic step-4 commit.
    assert_eq!(doc.orphan_keys, vec![minted.keys.public_key().to_hex()]);
    // The keyring holds exactly the minted nsec under agent:{pubkey}.
    let name = format!("agent:{}", minted.keys.public_key().to_hex());
    assert_eq!(
        store.stored.borrow().get(&name).map(String::as_str),
        Some(minted.keys.secret_key().to_bech32().expect("nsec").as_str())
    );
    // The binding validates on the very next read with no special case.
    let mut bindings = std::collections::BTreeMap::new();
    bindings.insert(minted.owner_hex.clone(), minted.binding.clone());
    let entry = LibraryEntry {
        library_id: "lib-1".into(),
        origin: OriginKey {
            scope_id: "s".into(),
            slug: "a".into(),
        },
        revision: 1,
        deleted: false,
        owner_pubkey_at_share: minted.owner_hex.clone(),
        shared: SharedDefinition {
            display_name: "A".into(),
            avatar_url: None,
            system_prompt: "p".into(),
            runtime: None,
            model: None,
            provider: None,
            name_pool: vec![],
            respond_to: None,
            respond_to_allowlist: vec![],
            parallelism: None,
        },
        deferred_archives: vec![],
        identity_bindings: bindings,
        projections: std::collections::BTreeMap::new(),
    };
    validate_entry_bindings(&entry).expect("minted binding validates on read");
}

#[test]
fn test_mint_crash_at_journal_persist_writes_no_secret() {
    // Crash point after step 1, at the durable journal write (step 2): the
    // keyring is never touched, so no un-journaled secret can exist. The
    // in-memory document holds the pubkey, but nothing was persisted or stored.
    let owner = Keys::generate();
    let store = FakeKeyStore::ok();
    let mut doc = library(vec![]);

    let err = mint_bound_identity(&mut doc, &owner, &store, |_| {
        Err("disk full at journal persist".to_string())
    })
    .expect_err("persist failure propagates");

    assert!(err.contains("disk full"));
    assert_eq!(
        *store.writes.borrow(),
        0,
        "no keyring write before durable journal"
    );
    assert!(store.stored.borrow().is_empty(), "no secret persisted");
}

#[test]
fn test_mint_crash_at_keyring_write_leaves_durable_orphan_for_reap() {
    // Crash point after step 2, at the keyring write (step 3): the durable
    // orphan row already exists (persist ran), so the dangling key — whether or
    // not the backend stored it — is reaped at the next recovery point. No
    // binding is produced.
    let owner = Keys::generate();
    let store = FakeKeyStore::write_fails();
    let mut doc = library(vec![]);
    let mut persisted: Option<LibraryDocument> = None;

    let err = mint_bound_identity(&mut doc, &owner, &store, |d| {
        persisted = Some(d.clone());
        Ok(())
    })
    .expect_err("keyring write failure propagates");

    assert!(err.contains("verify"));
    assert_eq!(*store.writes.borrow(), 1, "keyring write was attempted");
    // The durable snapshot carries the orphan coordinate…
    let durable = persisted.expect("journal was persisted before the keyring write");
    assert_eq!(durable.orphan_keys.len(), 1);
    // …and it is unreferenced (no binding committed), so the reap selects it.
    assert_eq!(
        durable.unreferenced_orphans(),
        durable
            .orphan_keys
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_mint_crash_at_read_back_leaves_durable_orphan_no_binding() {
    // Crash point in step 4: the keyring write verified, but the immediate
    // read-back misses (key vanished between write and load). No binding is
    // produced, the error surfaces, and the durable orphan row (step 2) survives
    // so the dangling key is reaped at the next recovery point.
    let owner = Keys::generate();
    let store = FakeKeyStore::load_misses();
    let mut doc = library(vec![]);
    let mut persisted: Option<LibraryDocument> = None;

    let err = mint_bound_identity(&mut doc, &owner, &store, |d| {
        persisted = Some(d.clone());
        Ok(())
    })
    .expect_err("read-back miss propagates");

    assert!(err.contains("absent from keyring"));
    assert_eq!(
        *store.writes.borrow(),
        1,
        "keyring write verified before load"
    );
    // The durable snapshot carries the orphan coordinate, unreferenced — the reap
    // selects it exactly as at the keyring-write crash point.
    let durable = persisted.expect("journal was persisted before the keyring write");
    assert_eq!(durable.orphan_keys.len(), 1);
    assert_eq!(
        durable.unreferenced_orphans(),
        durable
            .orphan_keys
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
}

// ── reap_unreferenced_orphans: §4 recovery sweep (§2.5 step 3) ───────────────────

#[test]
fn test_reap_deletes_uncommitted_keys_and_drops_rows_leaving_committed() {
    // Two orphans journaled: one committed (a live binding references it), one
    // dangling. The sweep deletes only the dangling key and drops only its row;
    // the committed key and row survive untouched.
    let committed = "aa".repeat(32);
    let dangling = "bb".repeat(32);
    let store = FakeKeyStore::ok()
        .with_key(&format!("agent:{committed}"), "nsec-committed")
        .with_key(&format!("agent:{dangling}"), "nsec-dangling");
    let mut doc = library(vec![bound_entry(&committed, false)]);
    doc.journal_orphan_pubkey(&committed);
    doc.journal_orphan_pubkey(&dangling);
    let saves: RefCell<usize> = RefCell::new(0);

    reap_unreferenced_orphans(&mut doc, &store, |_| {
        *saves.borrow_mut() += 1;
        Ok(())
    })
    .expect("reap");

    assert_eq!(
        *saves.borrow(),
        1,
        "one durable write for the trimmed journal"
    );
    assert_eq!(*store.deletes.borrow(), 1, "only the dangling key deleted");
    // Committed row + key survive; dangling row + key gone.
    assert_eq!(doc.orphan_keys, vec![committed.clone()]);
    assert!(store
        .stored
        .borrow()
        .contains_key(&format!("agent:{committed}")));
    assert!(!store
        .stored
        .borrow()
        .contains_key(&format!("agent:{dangling}")));
}

#[test]
fn test_reap_is_idempotent_across_recovery_points() {
    // A second sweep after the first has nothing to reap: no deletes, the
    // journal is already clean. Proves re-running a recovery point is safe.
    let dangling = "cc".repeat(32);
    let store = FakeKeyStore::ok().with_key(&format!("agent:{dangling}"), "nsec");
    let mut doc = library(vec![]);
    doc.journal_orphan_pubkey(&dangling);

    reap_unreferenced_orphans(&mut doc, &store, |_| Ok(())).expect("first reap");
    assert!(doc.orphan_keys.is_empty());
    assert_eq!(*store.deletes.borrow(), 1);

    reap_unreferenced_orphans(&mut doc, &store, |_| Ok(())).expect("second reap");
    assert_eq!(
        *store.deletes.borrow(),
        1,
        "nothing left to delete on re-sweep"
    );
}

#[test]
fn test_reap_delete_failure_keeps_rows_persists_and_errors() {
    // Every keyring delete backend-fails: no row is dropped, yet persist still
    // runs once (recording zero progress durably), and the sweep returns Err
    // naming the retained orphans so a later recovery point retries them.
    let ok_pubkey = "dd".repeat(32);
    let fail_pubkey = "ee".repeat(32);
    let store = FakeKeyStore::delete_fails();
    let mut doc = library(vec![]);
    doc.journal_orphan_pubkey(&ok_pubkey);
    doc.journal_orphan_pubkey(&fail_pubkey);
    let mut persisted: Option<LibraryDocument> = None;

    let err = reap_unreferenced_orphans(&mut doc, &store, |d| {
        persisted = Some(d.clone());
        Ok(())
    })
    .expect_err("backend delete failure surfaces");

    assert!(err.contains("retry"));
    assert_eq!(*store.deletes.borrow(), 2, "both deletes attempted");
    // Persist ran once even though every delete failed — no row was dropped.
    let durable = persisted.expect("persist ran");
    assert_eq!(
        durable.orphan_keys.len(),
        2,
        "failed deletes keep their rows"
    );
}

#[test]
fn test_reap_persist_failure_propagates() {
    // A durable-write failure after the keyring deletes propagates — the caller
    // must know the trimmed journal did not reach disk (the next recovery point
    // re-deletes harmlessly and retries the drop).
    let dangling = "ff".repeat(32);
    let store = FakeKeyStore::ok().with_key(&format!("agent:{dangling}"), "nsec");
    let mut doc = library(vec![]);
    doc.journal_orphan_pubkey(&dangling);

    let err = reap_unreferenced_orphans(&mut doc, &store, |_| Err("disk full".to_string()))
        .expect_err("persist failure propagates");

    assert!(err.contains("disk full"));
    assert_eq!(
        *store.deletes.borrow(),
        1,
        "delete ran before the failed persist"
    );
}

// ── retirement finalizer: §3.5 marker maintenance (P6-I2/P15-I2/P16-I1/P17-C1) ───

/// A projection in the given state (only `state` matters to the finalizer; the
/// other fields are fixed placeholders).
fn projection(state: ProjectionState) -> ProjectionEntry {
    ProjectionEntry {
        state,
        local_slug: "lib-x".into(),
        relay_url: "wss://r".into(),
        workspace_label: None,
    }
}

/// A healthy entry with the given projections, bindings, and deferred rows.
fn entry(
    library_id: &str,
    projections: Vec<(&str, ProjectionState)>,
    bindings: Vec<&str>,
    deferred: Vec<&str>,
) -> LibraryEntry {
    let mut identity_bindings = std::collections::BTreeMap::new();
    for (i, agent_pubkey) in bindings.into_iter().enumerate() {
        identity_bindings.insert(
            format!("owner-{i}"),
            IdentityBinding {
                agent_pubkey: agent_pubkey.into(),
                auth_tag: "{}".into(),
            },
        );
    }
    let mut e = LibraryEntry {
        library_id: library_id.into(),
        origin: OriginKey {
            scope_id: "s".into(),
            slug: library_id.into(),
        },
        revision: 1,
        deleted: false,
        owner_pubkey_at_share: "owner".into(),
        shared: SharedDefinition {
            display_name: "A".into(),
            avatar_url: None,
            system_prompt: "p".into(),
            runtime: None,
            model: None,
            provider: None,
            name_pool: vec![],
            respond_to: None,
            respond_to_allowlist: vec![],
            parallelism: None,
        },
        deferred_archives: vec![],
        identity_bindings,
        projections: projections
            .into_iter()
            .map(|(scope, state)| (scope.to_string(), projection(state)))
            .collect(),
    };
    for agent_pubkey in deferred {
        e.upsert_deferred_archive("s".into(), agent_pubkey.into());
    }
    e
}

fn loaded(healthy: Vec<LibraryEntry>) -> LoadedLibrary {
    LoadedLibrary {
        healthy,
        quarantined: vec![],
        orphan_keys: vec![],
        degradations: vec![],
    }
}

#[test]
fn test_retirement_due_when_all_terminal_and_key_or_row_survives() {
    // Every projection terminal (mixed Excluded + Deleted) while a binding key
    // still exists ⇒ retirement-due. A second entry with a deferred row and no
    // binding is equally due — the marker alone qualifies.
    let bound = entry(
        "lib-bound",
        vec![
            ("a", ProjectionState::Excluded),
            ("b", ProjectionState::Deleted),
        ],
        vec![&"aa".repeat(32)],
        vec![],
    );
    let deferred_only = entry(
        "lib-deferred",
        vec![("a", ProjectionState::Deleted)],
        vec![],
        vec![&"bb".repeat(32)],
    );
    let lib = loaded(vec![bound, deferred_only]);

    let due: Vec<&str> = lib
        .retirement_due_entries()
        .iter()
        .map(|e| e.library_id.as_str())
        .collect();
    assert_eq!(due, vec!["lib-bound", "lib-deferred"]);
}

#[test]
fn test_not_retirement_due_while_any_projection_live() {
    // One non-terminal projection keeps the entry off the list — the binding
    // still backs a live projection, so its key is not even a retirement marker.
    let lib = loaded(vec![entry(
        "lib-live",
        vec![
            ("a", ProjectionState::Excluded),
            ("b", ProjectionState::Materialized { revision: 1 }),
        ],
        vec![&"cc".repeat(32)],
        vec![],
    )]);
    assert!(lib.retirement_due_entries().is_empty());
}

#[test]
fn test_not_retirement_due_without_key_or_deferred_row() {
    // All terminal but neither a binding nor a deferred row survives — there is
    // no marker to maintain, so the entry is not retirement-due.
    let lib = loaded(vec![entry(
        "lib-clean",
        vec![("a", ProjectionState::Deleted)],
        vec![],
        vec![],
    )]);
    assert!(lib.retirement_due_entries().is_empty());
}

#[test]
fn test_never_projected_entry_is_not_retirement_due() {
    // Zero projections is not "vacuously all terminal": retirement means the
    // entry WAS live and is now fully terminal. A bound entry that never
    // projected anywhere must not be flagged.
    let lib = loaded(vec![entry(
        "lib-fresh",
        vec![],
        vec![&"dd".repeat(32)],
        vec![],
    )]);
    assert!(lib.retirement_due_entries().is_empty());
}

#[test]
fn test_finalizer_discharges_nothing_markers_persist() {
    // The finalizer is a pure observer: reading it does not touch the entry's
    // markers. The keyring is untouched by construction (no KeyStore parameter);
    // here we assert the binding + deferred rows survive the observation, the v1
    // permanent-marker contract (P17-C1).
    let lib = loaded(vec![entry(
        "lib-due",
        vec![("a", ProjectionState::Deleted)],
        vec![&"ee".repeat(32)],
        vec![&"ee".repeat(32)],
    )]);

    assert_eq!(lib.retirement_due_entries().len(), 1);

    // Markers untouched: the binding and the deferred row are exactly as journaled.
    let e = &lib.healthy[0];
    assert_eq!(e.identity_bindings.len(), 1);
    assert_eq!(e.deferred_archive_obligations().len(), 1);
    // And the pubkey stays protected — no discharge weakened the predicate.
    let doc = lib.rebuild_document().expect("rebuild");
    assert!(doc.key_archive_protected(&"ee".repeat(32)));
}
