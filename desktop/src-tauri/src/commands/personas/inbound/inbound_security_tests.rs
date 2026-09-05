//! Security-focused inbound regressions: tombstone (kind:5) authorization
//! and retained-head coverage, the inbound signature gate, and
//! restart-rehydration of the private-config overlay.
//!
//! Extracted from `inbound_tests.rs` to keep it under the file-size cap;
//! `#[path]`-included from there. Helpers (`AGENT_PUBKEY`,
//! `private_agent_payload`, `local_agent`) live in the parent module.

#[allow(unused_imports)]
use super::super::*;
use super::*;

// ── Tombstone (kind:5) consume ────────────────────────────────────────────

fn deletion_event(coord: &str) -> nostr::Event {
    deletion_event_with_keys(coord, &nostr::Keys::generate())
}

fn deletion_event_with_keys(coord: &str, keys: &nostr::Keys) -> nostr::Event {
    use nostr::{EventBuilder, JsonUtil, Kind, Tag};
    let event = EventBuilder::new(Kind::Custom(5), "")
        .tags(vec![Tag::parse(["a", coord]).unwrap()])
        .sign_with_keys(keys)
        .unwrap();
    nostr::Event::from_json(event.as_json()).unwrap()
}

/// A deletion event whose coordinate owner IS its signer — the only shape
/// `parse_deletion_coordinate` accepts since the owner check landed.
fn owned_deletion_event(kind: u32, d_tag: &str) -> nostr::Event {
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    deletion_event_with_keys(&format!("{kind}:{owner}:{d_tag}"), &keys)
}

/// Like [`deletion_event_with_keys`] but with a controlled `created_at`, for
/// tests that race a tombstone against a retained head's timestamp.
fn owned_deletion_event_at(
    keys: &nostr::Keys,
    kind: u32,
    d_tag: &str,
    created_at: u64,
) -> nostr::Event {
    use nostr::{EventBuilder, JsonUtil, Kind, Tag, Timestamp};
    let owner = keys.public_key().to_hex();
    let event = EventBuilder::new(Kind::Custom(5), "")
        .tags(vec![
            Tag::parse(["a", &format!("{kind}:{owner}:{d_tag}")]).unwrap()
        ])
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .unwrap();
    nostr::Event::from_json(event.as_json()).unwrap()
}

// ── resolve_inbound_tombstone: authorization + retained-head coverage ──────
//
// The seam is deliberately a pure function over (event, scope_owner, conn) so
// the security decision is testable WITHOUT a live AppHandle — the production
// caller (`reconcile_inbound_tombstone`) is a `#[tauri::command]` body that
// only adds scope resolution and the per-kind store mutation around it. These
// are the direct-IPC regressions: the frontend owner filter never runs here.

/// Issue-4 regression: a VALIDLY SIGNED tombstone whose signer owns the
/// coordinate it names — so it clears `parse_deletion_coordinate` — but who is
/// not the workspace owner must no-op. Local stores match by d-tag alone, so
/// without the scope-owner binding this would delete the victim's record.
#[test]
fn resolve_inbound_tombstone_rejects_foreign_owner() {
    use crate::managed_agents::retention::open_retention_db;
    use std::path::Path;

    let conn = open_retention_db(Path::new(":memory:")).unwrap();
    let scope_owner_keys = nostr::Keys::generate();
    let attacker_keys = nostr::Keys::generate();

    // The attacker names their OWN coordinate (passes the NIP-09 signer ==
    // coordinate-owner check) with a d-tag colliding with the victim's agent.
    let forged = owned_deletion_event_at(&attacker_keys, 30177, AGENT_PUBKEY, 100);
    let resolved =
        resolve_inbound_tombstone(&forged, &scope_owner_keys.public_key(), &conn).unwrap();
    assert_eq!(resolved, None, "foreign-owner tombstone must not authorize");

    // Nothing was retained: the attacker cannot even park a kind:5 row in the
    // victim's scoped store.
    assert!(
        !crate::managed_agents::retention::has_retained_personas(
            &conn,
            &attacker_keys.public_key().to_hex()
        )
        .unwrap(),
        "a rejected tombstone must leave no retained row"
    );

    // POSITIVE CONTROL: the same event shape signed by the scope owner
    // resolves — proving the rejection above is the owner binding, not a
    // broken fixture.
    let owned = owned_deletion_event_at(&scope_owner_keys, 30177, AGENT_PUBKEY, 100);
    assert_eq!(
        resolve_inbound_tombstone(&owned, &scope_owner_keys.public_key(), &conn).unwrap(),
        Some((30177, AGENT_PUBKEY.to_string())),
        "control: the scope owner's tombstone authorizes"
    );
}

/// Issue-2 regression: head → tombstone → restart. The tombstone must remove
/// the retained kind:30179 head so a fresh boot's `hydrate_from_retention`
/// cannot rematerialize the deleted secret-bearing agent.
#[test]
fn tombstoned_private_head_does_not_survive_restart_hydration() {
    use crate::managed_agents::{
        private_config_overlay::{hydrate_from_retention, PrivateConfigOverlay},
        retention::open_retention_db,
    };
    use buzz_core_pkg::private_managed_agent;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("retention.db");
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let agent_hex = agent_keys.public_key().to_hex();

    // Session 1: a private head lands and is retained durably.
    {
        let conn = open_retention_db(&db_path).unwrap();
        let payload = private_agent_payload(&owner_keys, &agent_keys, "doomed", 4);
        let event = private_managed_agent::build_event(&owner_keys, &payload, 20).unwrap();
        let mut overlay = PrivateConfigOverlay::default();
        apply_inbound_private_managed_agent_event(&event, &owner_keys, &conn, &mut overlay)
            .unwrap();

        // The delete arrives: a kind:5 covering the PUBLIC coordinate (either
        // agent kind must cover both heads — the sibling tombstone may be
        // lost).
        let tombstone = owned_deletion_event_at(&owner_keys, 30177, &agent_hex, 30);
        let resolved =
            resolve_inbound_tombstone(&tombstone, &owner_keys.public_key(), &conn).unwrap();
        assert_eq!(resolved, Some((30177, agent_hex.clone())));
    }

    // Session 2 (restart, fresh overlay): hydration must NOT resurrect it.
    let conn = open_retention_db(&db_path).unwrap();
    let overlay = hydrate_from_retention(&conn, &owner_keys).unwrap();
    assert!(
        overlay.resolved_records(&[]).is_empty(),
        "a tombstoned 30179 head must not rehydrate after restart"
    );

    // Replay: the same tombstone re-received dedupes to a no-op instead of
    // re-deleting (retention row already at its created_at).
    let replay = owned_deletion_event_at(&owner_keys, 30177, &agent_hex, 30);
    assert_eq!(
        resolve_inbound_tombstone(&replay, &owner_keys.public_key(), &conn).unwrap(),
        None,
        "a replayed tombstone must dedupe"
    );
}

/// NIP-09 bound: a deletion covers events up to its own `created_at`. A head
/// NEWER than the tombstone is a recreation (or the tombstone is a late
/// replay) — the record and its heads must survive.
#[test]
fn tombstone_older_than_the_retained_head_preserves_it() {
    use crate::managed_agents::{
        private_config_overlay::{hydrate_from_retention, PrivateConfigOverlay},
        retention::open_retention_db,
    };
    use buzz_core_pkg::private_managed_agent;
    use std::path::Path;

    let conn = open_retention_db(Path::new(":memory:")).unwrap();
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let agent_hex = agent_keys.public_key().to_hex();

    // Head at t=50; tombstone authored earlier at t=30 arrives late.
    let payload = private_agent_payload(&owner_keys, &agent_keys, "survivor", 4);
    let event = private_managed_agent::build_event(&owner_keys, &payload, 50).unwrap();
    let mut overlay = PrivateConfigOverlay::default();
    apply_inbound_private_managed_agent_event(&event, &owner_keys, &conn, &mut overlay).unwrap();

    let stale_tombstone = owned_deletion_event_at(&owner_keys, 30179, &agent_hex, 30);
    assert_eq!(
        resolve_inbound_tombstone(&stale_tombstone, &owner_keys.public_key(), &conn).unwrap(),
        None,
        "a tombstone older than the head must not delete it"
    );
    let overlay = hydrate_from_retention(&conn, &owner_keys).unwrap();
    assert_eq!(
        overlay.resolved_records(&[]).len(),
        1,
        "the newer head survives the stale tombstone"
    );
}

#[test]
fn parse_deletion_coordinate_extracts_kind_and_d_tag() {
    // Persona / team / agent coordinates all route by their leading kind.
    let p = owned_deletion_event(30175, "my-persona");
    assert_eq!(
        parse_deletion_coordinate(&p),
        Some((30175, "my-persona".to_string()))
    );
    let a = owned_deletion_event(30177, "agentpubkeyhex");
    assert_eq!(
        parse_deletion_coordinate(&a),
        Some((30177, "agentpubkeyhex".to_string()))
    );
}

#[test]
fn parse_deletion_coordinate_rejects_foreign_owner() {
    // A validly signed kind:5 naming ANOTHER owner's coordinate must no-op:
    // NIP-09 scopes deletion to the record's own author.
    let foreign_owner = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    let forged = deletion_event(&format!("30175:{foreign_owner}:my-persona"));
    assert_eq!(parse_deletion_coordinate(&forged), None);
}

#[test]
fn parse_deletion_coordinate_handles_colon_in_d_tag_and_rejects_malformed() {
    // A d-tag containing ':' keeps its remainder intact (splitn(3)).
    let weird = owned_deletion_event(30176, "a:b:c");
    assert_eq!(
        parse_deletion_coordinate(&weird),
        Some((30176, "a:b:c".to_string()))
    );
    // Missing d-tag segment / non-numeric kind → None (no-op).
    assert_eq!(
        parse_deletion_coordinate(&deletion_event("30175:owner")),
        None
    );
    assert_eq!(
        parse_deletion_coordinate(&deletion_event("notakind:owner:d")),
        None
    );
}

#[test]
fn tombstone_removal_predicates_match_apply_fn_keys() {
    // The deletion path removes by the SAME per-kind key the apply fns use.
    // Persona: by persona_d_tag (slug/id).
    let mut personas = vec![local_in_app()];
    let target = persona_d_tag(&personas[0]);
    personas.retain(|r| persona_d_tag(r) != target);
    assert!(personas.is_empty(), "persona removed by its d-tag");

    // Team: by id.
    let mut teams = vec![local_team()];
    teams.retain(|r| r.id != TEAM_ID);
    assert!(teams.is_empty(), "team removed by id");

    // Managed-agent: by pubkey. A non-matching d-tag is a no-op.
    let mut agents = vec![local_agent()];
    agents.retain(|r| r.pubkey != "someoneelse");
    assert_eq!(agents.len(), 1, "non-matching agent tombstone no-ops");
    agents.retain(|r| r.pubkey != AGENT_PUBKEY);
    assert!(agents.is_empty(), "agent removed by pubkey");
}

// ── Inbound signature gate ──────────────────────────────────────────────────

#[test]
fn inbound_gate_rejects_tampered_event() {
    use nostr::JsonUtil;
    // A validly signed event whose content was altered post-signing: the
    // pubkey is real, the sig no longer covers the bytes. Must die at the
    // gate before any store logic runs.
    let keys = nostr::Keys::generate();
    let event = nostr::EventBuilder::new(nostr::Kind::Custom(30175), "{}")
        .tags(vec![nostr::Tag::parse(["d", "victim-slug"]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let tampered = event.as_json().replace(
        "\"content\":\"{}\"",
        "\"content\":\"{\\\"system_prompt\\\":\\\"pwned\\\"}\"",
    );
    assert_ne!(
        tampered,
        event.as_json(),
        "string replace must have taken effect — if this fails the test is testing an un-tampered event"
    );

    let err = parse_verified_inbound_event(&tampered).unwrap_err();
    assert!(
        err.contains("signature"),
        "tampered event must fail the signature gate: {err}"
    );
}

#[test]
fn inbound_gate_accepts_validly_signed_event() {
    use nostr::JsonUtil;
    let keys = nostr::Keys::generate();
    let event = nostr::EventBuilder::new(nostr::Kind::Custom(30175), "{}")
        .tags(vec![nostr::Tag::parse(["d", "slug"]).unwrap()])
        .sign_with_keys(&keys)
        .unwrap();
    let parsed = parse_verified_inbound_event(&event.as_json()).unwrap();
    assert_eq!(parsed.pubkey, keys.public_key());
}

/// Item-0 FIX verification: after a "restart" (same retention db, fresh
/// overlay), `hydrate_from_retention` repopulates the overlay from the durable
/// rows — so the resolve sites see relay config instead of stale disk.
#[test]
fn sami_fix_overlay_rehydrates_from_retention_after_restart() {
    use crate::managed_agents::{
        private_config_overlay::{hydrate_from_retention, PrivateConfigOverlay},
        retention::open_retention_db,
    };
    use buzz_core_pkg::private_managed_agent;
    use tempfile::TempDir;

    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("retention.db");
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();

    let payload = private_agent_payload(&owner_keys, &agent_keys, "relay name", 4);
    let event = private_managed_agent::build_event(&owner_keys, &payload, 20).unwrap();

    // Session 1: the event lands and is retained durably.
    {
        let conn = open_retention_db(&db_path).unwrap();
        let mut overlay = PrivateConfigOverlay::default();
        apply_inbound_private_managed_agent_event(&event, &owner_keys, &conn, &mut overlay)
            .unwrap();
    }

    // Session 2 (restart): hydrate straight from the retained rows — no
    // inbound event required.
    let conn = open_retention_db(&db_path).unwrap();
    let overlay = hydrate_from_retention(&conn, &owner_keys).unwrap();
    let resolved = overlay.resolved_records(&[]);
    assert_eq!(resolved.len(), 1, "FIX: overlay rehydrates from retention");
    assert_eq!(resolved[0].name, "relay name");
    assert_eq!(resolved[0].parallelism, 4);

    // NEGATIVE CONTROL: a different owner's keys must hydrate NOTHING — proves
    // the query is scoped by owner pubkey and not just returning every row.
    let stranger = nostr::Keys::generate();
    assert!(
        hydrate_from_retention(&conn, &stranger)
            .unwrap()
            .resolved_records(&[])
            .is_empty(),
        "control: hydration is owner-scoped"
    );
}

#[test]
fn meli_deleted_private_head_replay_must_not_resurrect() {
    use crate::managed_agents::{
        private_config_overlay::{hydrate_from_retention, PrivateConfigOverlay},
        retention::{open_retention_db, InboundOutcome},
    };
    let conn = open_retention_db(std::path::Path::new(":memory:")).unwrap();
    let owner = nostr::Keys::generate();
    let agent = nostr::Keys::generate();
    let pubkey = agent.public_key().to_hex();
    let payload = private_agent_payload(&owner, &agent, "deleted agent", 4);
    let head = buzz_core_pkg::private_managed_agent::build_event(&owner, &payload, 20).unwrap();
    let mut overlay = PrivateConfigOverlay::default();
    assert_eq!(
        apply_inbound_private_managed_agent_event(&head, &owner, &conn, &mut overlay).unwrap(),
        InboundOutcome::Applied
    );
    let tombstone = owned_deletion_event_at(&owner, 30177, &pubkey, 30);
    assert_eq!(
        resolve_inbound_tombstone_with_store(&tombstone, &owner.public_key(), &conn, |_, key| {
            overlay.remove(key);
            Ok(())
        })
        .unwrap(),
        Some((30177, pubkey.clone()))
    );
    assert!(hydrate_from_retention(&conn, &owner)
        .unwrap()
        .resolved_records(&[])
        .is_empty());
    let outcome =
        apply_inbound_private_managed_agent_event(&head, &owner, &conn, &mut overlay).unwrap();
    let recovered = hydrate_from_retention(&conn, &owner)
        .unwrap()
        .resolved_records(&[]);
    assert_eq!(
        (outcome, recovered.len()),
        (InboundOutcome::Skipped, 0),
        "head t20 replay after tombstone t30 must not rehydrate secret-bearing config"
    );
}

#[test]
fn retained_authority_corruption_cannot_hydrate_as_absent() {
    use crate::managed_agents::{
        private_config_overlay::{hydrate_from_retention, PrivateConfigOverlay},
        retention::{open_retention_db, retain_event, RetainedEvent},
    };
    let conn = open_retention_db(std::path::Path::new(":memory:")).unwrap();
    let owner = nostr::Keys::generate();
    let agent = nostr::Keys::generate();
    retain_event(
        &conn,
        &RetainedEvent {
            kind: 30179,
            pubkey: owner.public_key().to_hex(),
            d_tag: agent.public_key().to_hex(),
            content: "corrupt ciphertext".into(),
            created_at: 20,
            raw_event: "not a signed event".into(),
            pending_sync: false,
        },
    )
    .unwrap();
    assert!(
        hydrate_from_retention(&conn, &owner).is_err(),
        "known but unreadable authority is not absent authority"
    );
    assert!(
        PrivateConfigOverlay::default()
            .absorb_retained_head(&conn, &owner, &agent.public_key().to_hex())
            .is_err(),
        "self-authored write-through cannot swallow unreadable authority"
    );
}

#[test]
fn retained_authority_metadata_must_match_signed_event() {
    use crate::managed_agents::{
        private_config_overlay::hydrate_from_retention,
        retention::{open_retention_db, retain_event, RetainedEvent},
    };
    use nostr::JsonUtil;
    let owner = nostr::Keys::generate();
    let agent = nostr::Keys::generate();
    let payload = private_agent_payload(&owner, &agent, "authority", 1);
    let event = buzz_core_pkg::private_managed_agent::build_event(&owner, &payload, 20).unwrap();
    for field in ["d_tag", "content", "created_at"] {
        let conn = open_retention_db(std::path::Path::new(":memory:")).unwrap();
        let mut row = RetainedEvent {
            kind: 30179,
            pubkey: owner.public_key().to_hex(),
            d_tag: agent.public_key().to_hex(),
            content: event.content.clone(),
            created_at: 20,
            raw_event: event.as_json(),
            pending_sync: false,
        };
        match field {
            "d_tag" => row.d_tag = nostr::Keys::generate().public_key().to_hex(),
            "content" => row.content = "different ciphertext".into(),
            "created_at" => row.created_at = 30,
            _ => unreachable!(),
        }
        retain_event(&conn, &row).unwrap();
        assert!(
            hydrate_from_retention(&conn, &owner).is_err(),
            "mismatched {field} must not become authority"
        );
    }
}
