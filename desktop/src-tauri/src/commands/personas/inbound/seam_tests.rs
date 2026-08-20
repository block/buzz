//! P2-I1 command-seam integration fixture for the §2.8 frozen-linkage
//! convergence contract.
//!
//! Split from `inbound_tests.rs` to keep that file under the 1000-line ratchet;
//! included via `#[path = "seam_tests.rs"] mod seam_tests;`. `use super::*`
//! brings in the `inbound_tests.rs` fixtures (`local_agent`, `linked_agent`,
//! `definition`, `AGENT_PUBKEY`).
//!
//! Unlike the helper-level convergence tests, this fixture drives the real
//! command core [`apply_inbound_upsert_in_scope`] — crossing the exact `?` on
//! the corrective re-retain — against a mock Tauri app, an active workspace
//! scope, and a retention DB whose second (corrective) write is rejected by an
//! injected conditional trigger. It proves the sequence the helper tests
//! cannot: inbound retain succeeds → safe fields + frozen linkage persist to
//! disk → corrective retain fails → the command returns `Err`, never a
//! successful outcome. It then proves the durable retry: the real boot
//! reconcile, run against the state the failed command left behind, restores
//! the authoritative projection over the hostile head.
//!
//! If the `?` at the corrective call site (`inbound.rs`) were weakened to a
//! swallow (`let _ = converge_frozen_linkage(...)`, `.ok()`, or a bare
//! `eprintln!`), assertion (a) fails — the command would report success over a
//! retained non-authoritative head.
use super::*;

use crate::app_state::AppState;
use crate::managed_agents::reconcile::reconcile_agents_to_events;
use crate::managed_agents::retention::{get_retained_event, open_retention_db, RetentionScope};
use crate::managed_agents::scope::{current_scope_generation, WorkspaceAgentScope};
use crate::managed_agents::storage::load_managed_agents_at;
use buzz_core_pkg::kind::KIND_MANAGED_AGENT;
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use tauri::Manager;

/// The projected definition the local instance links to, plus the linked
/// instance itself, written to a scope's `managed-agents.json` exactly as the
/// unified store persists them (key-less definitions first, keyed instances
/// after). The instance's `persona_id` resolves to the projected definition,
/// so the §2.8 resolver classifies its linkage library-owned.
fn seed_store(definitions_dir: &std::path::Path) {
    std::fs::create_dir_all(definitions_dir).unwrap();
    let store = vec![definition("shared-def", true), linked_agent("shared-def")];
    let json = serde_json::to_vec_pretty(&store).unwrap();
    std::fs::write(definitions_dir.join("managed-agents.json"), json).unwrap();
}

/// A signed kind:30177 whose author is `owner_keys` (so it retains under the
/// owner's coordinate — the same coordinate the corrective re-retain and the
/// boot reconcile write, exactly as a device's own event reflected back off
/// the relay). Its `persona_id` re-points the library-owned linkage, which the
/// §2.8 rule freezes. Dated in the future so the retained head is
/// unambiguously newer, forcing the boot reconcile to bump past it.
fn hostile_repoint_event(owner_keys: &Keys, created_at: i64) -> nostr::Event {
    let content = serde_json::json!({
        "name": "Hostile Rename",
        "persona_id": "other-def",
        "parallelism": 7,
        "respond_to": "owner-only",
    })
    .to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_MANAGED_AGENT as u16), content)
        .tags(vec![Tag::parse(["d", AGENT_PUBKEY]).unwrap()])
        .custom_created_at(Timestamp::from(created_at as u64))
        .sign_with_keys(owner_keys)
        .unwrap();
    // Round-trip through JSON to mirror the wire path the command parses from.
    nostr::Event::from_json(event.as_json()).unwrap()
}

/// P2-I1: the real command core returns `Err` when the corrective re-retain
/// fails after the inbound head is retained, leaves the frozen linkage intact
/// on disk, keeps the hostile head retained, and the real boot reconcile then
/// converges the relay head back to the authoritative projection.
#[test]
fn frozen_linkage_corrective_failure_errs_and_boot_reconcile_converges() {
    let tmp = tempfile::tempdir().unwrap();
    let definitions_dir = tmp.path().join("defs");
    let db_path = tmp.path().join("retention.db");

    seed_store(&definitions_dir);

    let owner_keys = Keys::generate();
    let owner_hex = owner_keys.public_key().to_hex();

    // Pre-create the retention DB and install the fault injection: a BEFORE
    // INSERT/UPDATE trigger that ABORTs any write with `pending_sync = 1`. The
    // inbound retain writes `pending_sync = 0` (permitted); the corrective
    // re-retain writes `pending_sync = 1` into the SAME coordinate (an UPSERT
    // that resolves to UPDATE — hence the UPDATE trigger), so it is rejected.
    {
        let conn = open_retention_db(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER reject_corrective_insert
                 BEFORE INSERT ON persona_events
                 FOR EACH ROW WHEN NEW.pending_sync = 1
                 BEGIN SELECT RAISE(ABORT, 'injected fault: corrective write rejected'); END;
             CREATE TRIGGER reject_corrective_update
                 BEFORE UPDATE ON persona_events
                 FOR EACH ROW WHEN NEW.pending_sync = 1
                 BEGIN SELECT RAISE(ABORT, 'injected fault: corrective write rejected'); END;",
        )
        .unwrap();
    }

    // Mock Tauri app whose active scope points its definitions dir at the
    // seeded store; the signing keys are the event's author.
    let state = crate::app_state::build_app_state();
    *state.identity_lifecycle_keys_guard().unwrap() = owner_keys.clone();
    let app = tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("failed to build mock app for inbound seam test");
    app.state::<AppState>()
        .commit_active_scope(WorkspaceAgentScope {
            scope_id: "seam-test-scope".to_string(),
            relay_url: "wss://relay.local".to_string(),
            owner_pubkey: owner_hex.clone(),
            definitions_dir: definitions_dir.clone(),
            generation: current_scope_generation(),
        });
    let app_handle = app.handle().clone();

    let scope = RetentionScope {
        db_path: db_path.clone(),
        relay_url: "wss://relay.local".to_string(),
        owner_keys: owner_keys.clone(),
    };

    let hostile_created_at = (Timestamp::now().as_secs() as i64) + 3_600;
    let event = hostile_repoint_event(&owner_keys, hostile_created_at);

    // Drive the REAL command core across the corrective `?` propagation site.
    let result = super::super::apply_inbound_upsert_in_scope(
        &app_handle,
        &scope,
        KIND_MANAGED_AGENT,
        AGENT_PUBKEY.to_string(),
        None,
        &event,
    );

    // (a) The command MUST fail — never a successful typed outcome — while the
    // non-authoritative head is retained. This is the assertion that fails if
    // the corrective `?` is weakened to a swallow.
    let err = result.expect_err("corrective retain failure must propagate as Err");
    assert!(
        err.contains("convergence"),
        "the error must be the frozen-linkage convergence failure: {err}"
    );

    // (b) Disk holds the safe-field update with the original projected linkage
    // intact — the freeze applied `name` but refused the re-point.
    let instances = load_managed_agents_at(&definitions_dir).unwrap();
    let instance = instances
        .iter()
        .find(|r| r.pubkey == AGENT_PUBKEY)
        .expect("instance must remain on disk");
    assert_eq!(
        instance.persona_id,
        Some("shared-def".to_string()),
        "the library-owned linkage must stay frozen on disk, not re-pointed"
    );
    assert_eq!(
        instance.name, "Hostile Rename",
        "the safe per-instance field must have been applied and saved"
    );

    // (c) Retention still holds the hostile inbound head after the injected
    // failure — the corrective row never landed.
    {
        let conn = open_retention_db(&db_path).unwrap();
        let head = get_retained_event(&conn, KIND_MANAGED_AGENT, &owner_hex, AGENT_PUBKEY)
            .unwrap()
            .expect("the inbound head must remain retained");
        assert!(
            head.content.contains("other-def"),
            "the retained head must still be the hostile re-point"
        );
        assert!(
            !head.pending_sync,
            "the inbound head is retained pending_sync = 0"
        );
        assert_eq!(
            head.created_at, hostile_created_at,
            "the retained head is the hostile inbound event"
        );
    }

    // The injected transient failure clears (the DB is healthy at next boot);
    // the RETAINED STATE — hostile head + authoritative on-disk record — is
    // unchanged. Drop the triggers so the healthy boot reconcile can write.
    {
        let conn = open_retention_db(&db_path).unwrap();
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS reject_corrective_insert;
             DROP TRIGGER IF EXISTS reject_corrective_update;",
        )
        .unwrap();
    }

    // (d) The REAL boot reconcile, run against that same disk/DB state, re-diffs
    // the authoritative on-disk record against the hostile head and replaces it
    // with the authoritative projection at a greater created_at, queued for
    // publish. This is the durable retry owner that makes the failure
    // recoverable.
    reconcile_agents_to_events(&definitions_dir, &owner_keys, &db_path);

    let conn = open_retention_db(&db_path).unwrap();
    let converged = get_retained_event(&conn, KIND_MANAGED_AGENT, &owner_hex, AGENT_PUBKEY)
        .unwrap()
        .expect("boot reconcile must retain the authoritative row");
    assert!(
        converged.content.contains("shared-def"),
        "boot reconcile must restore the authoritative library-owned linkage"
    );
    assert!(
        !converged.content.contains("other-def"),
        "the hostile linkage must be superseded"
    );
    assert!(
        converged.pending_sync,
        "the corrective row must queue for publish"
    );
    assert!(
        converged.created_at > hostile_created_at,
        "created_at must bump past the hostile head so the relay accepts it"
    );
}
