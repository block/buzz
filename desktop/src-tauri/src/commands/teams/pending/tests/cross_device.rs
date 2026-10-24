// Carl r10 P1: cross-device catalog retention — supersede / retract, the
// production inbound dispatcher, and fresh-device backfill ordering.
//
// Extracted from the parent test file to keep it under the file-size cap.
use super::*;

/// Device B receiving Device A's 30178 head through the SAME production routing
/// decision the inbound reconcile uses (`retain_inbound_catalog_witness`), not a
/// raw `retain_inbound_event`. Driving the production dispatcher is what makes
/// the cross-device regressions causal: disabling its `KIND_TEAM_CATALOG` arm
/// turns these tests RED (see the explicit seam test below).
fn device_b_receives_head(db_path: &Path, owner: &str, head: &RetainedEvent) {
    let conn = open_retention_db(db_path).unwrap();
    let handled = crate::commands::personas::retain_inbound_catalog_witness(
        &conn,
        &RetainedEvent {
            pending_sync: false,
            ..head.clone()
        },
    )
    .unwrap();
    assert!(
        handled,
        "the production catalog dispatcher must handle a 30178 head"
    );
    // The row must land under the owner's coordinate for the refresh to find it.
    assert!(
        get_retained_event(&conn, KIND_TEAM_CATALOG, owner, "team-abc")
            .unwrap()
            .is_some(),
        "inbound retention must file the head at the owner coordinate"
    );
}

#[test]
fn test_inbound_catalog_witness_retains_through_the_production_dispatcher() {
    // Carl r10 P1, load-bearing production seam. A 30178 head driven through
    // `retain_inbound_catalog_witness` — the SINGLE routing decision the inbound
    // reconcile makes for a catalog arrival — must land an arrival-scoped
    // witness (`pending_sync = false`) and queue no outbound publish. A test
    // that retained via `retain_inbound_event` directly would stay GREEN even if
    // the production dispatch arm were deleted; this one goes RED, because it is
    // the production fn under test.
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();

    let device_a = scoped_db(dir.path(), "wss://a.example", &owner);
    prepare_team_publication_at(&device_a, &keys, &team(), &members(), Some(true)).unwrap();
    let a_head = retained_head(&device_a, &owner).unwrap();

    let device_b = scoped_db(dir.path(), "wss://b.example", &owner);
    let conn = open_retention_db(&device_b).unwrap();
    let handled = crate::commands::personas::retain_inbound_catalog_witness(
        &conn,
        &RetainedEvent {
            pending_sync: false,
            ..a_head.clone()
        },
    )
    .unwrap();

    assert!(handled, "a 30178 arrival must be handled by the dispatcher");
    let witness = get_retained_event(&conn, KIND_TEAM_CATALOG, &owner, "team-abc")
        .unwrap()
        .expect("the dispatcher must retain the arrival witness");
    assert!(
        !witness.pending_sync,
        "an inbound witness is already on the relay — it must not be queued for publish"
    );
    assert!(
        get_pending_sync(&conn).unwrap().is_empty(),
        "retaining a witness must queue no outbound publication (no ping-pong)"
    );
}

#[test]
fn test_device_b_supersedes_a_shared_head_after_inbound_retention_then_edit() {
    // Carl's scenario, load-bearing leg. A shares; B retains A's head via the
    // inbound path; B edits a member. B must supersede A's discoverable head —
    // possible ONLY because B retained the head (the refresh guard-returns Noop
    // without a retained row).
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();

    // Device A publishes the shared head.
    let device_a = scoped_db(dir.path(), "wss://a.example", &owner);
    prepare_team_publication_at(&device_a, &keys, &team(), &members(), Some(true)).unwrap();
    let a_head = retained_head(&device_a, &owner).unwrap();

    // Device B (a distinct scope) receives it inbound, then edits a member.
    let device_b = scoped_db(dir.path(), "wss://b.example", &owner);
    device_b_receives_head(&device_b, &owner, &a_head);

    let edited = vec![member("m1", "Renamed On B"), member("m2", "Two")];
    let outcome = refresh_or_retract_shared_head_at(&device_b, &keys, &team(), &edited).unwrap();

    assert_eq!(
        outcome,
        RefreshOrRetractOutcome::Refreshed,
        "B must supersede A's head after editing a member"
    );
    let b_head = retained_head(&device_b, &owner).unwrap();
    assert!(
        b_head.content.contains("Renamed On B"),
        "B's superseding head must carry the edit"
    );
    assert!(
        b_head.created_at > a_head.created_at,
        "B's head ({}) must monotonically supersede A's ({})",
        b_head.created_at,
        a_head.created_at
    );
    assert!(
        b_head.pending_sync,
        "B's superseding head must be queued for the flush loop"
    );
}

#[test]
fn test_device_b_tombstones_the_coordinate_after_inbound_retention_then_delete() {
    // B retains A's head, then the owner deletes the team on B. B must tombstone
    // the 30178 coordinate — again reachable only because B retained the head.
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();

    let device_a = scoped_db(dir.path(), "wss://a.example", &owner);
    prepare_team_publication_at(&device_a, &keys, &team(), &members(), Some(true)).unwrap();
    let a_head = retained_head(&device_a, &owner).unwrap();

    let device_b = scoped_db(dir.path(), "wss://b.example", &owner);
    device_b_receives_head(&device_b, &owner, &a_head);

    tombstone_team_catalog_at(&device_b, &keys, "team-abc").unwrap();

    assert!(
        retained_head(&device_b, &owner).is_none(),
        "B must purge the retained head on delete"
    );
    let tombstone = enqueued_tombstone(&device_b);
    assert_eq!(
        tombstone.d_tag,
        tombstone_retention_d_tag(KIND_TEAM_CATALOG, "team-abc"),
        "B must enqueue a kind:5 targeting the 30178 coordinate"
    );
    assert!(
        tombstone.created_at > a_head.created_at,
        "B's tombstone must dominate A's future-datable head"
    );
}

#[test]
fn test_inbound_catalog_retention_alone_enqueues_no_publish() {
    // No-ping-pong guard: retaining an inbound 30178 head (the arrival witness)
    // must NOT queue an outbound publish. If it did, two devices would republish
    // identical heads at each other on every arrival.
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();

    let device_a = scoped_db(dir.path(), "wss://a.example", &owner);
    prepare_team_publication_at(&device_a, &keys, &team(), &members(), Some(true)).unwrap();
    let a_head = retained_head(&device_a, &owner).unwrap();

    let device_b = scoped_db(dir.path(), "wss://b.example", &owner);
    device_b_receives_head(&device_b, &owner, &a_head);

    let conn = open_retention_db(&device_b).unwrap();
    assert!(
        get_pending_sync(&conn).unwrap().is_empty(),
        "an inbound 30178 arrival must retain a witness but queue no publish"
    );
    let retained = retained_head(&device_b, &owner).unwrap();
    assert!(
        !retained.pending_sync,
        "the retained inbound witness must not be flagged for publish"
    );
}

/// Replay device B's fresh-sync backfill through the exact production cores in
/// a given dispatch order and return B's final catalog state as
/// `(retained_head_is_some, tombstone_enqueued)`.
///
/// Each dispatched event drives the same fn production calls: a 30178 head goes
/// through `retain_inbound_catalog_witness` (the inbound dispatcher's single
/// catalog decision), and the team/persona upserts drive
/// `resolve_and_refresh_or_retract_at` (the refresh the inbound spine runs after
/// a 30176/30175 apply). The only variable is the order — which is exactly what
/// `orderCatalogHeadsLast` controls on the TS backfill.
fn replay_fresh_sync_in_order(
    db_path: &Path,
    keys: &nostr::Keys,
    a_head: &RetainedEvent,
    catalog_before_constituents: bool,
) -> (bool, bool) {
    let owner = keys.public_key().to_hex();
    let receive_head = |db: &Path| {
        let conn = open_retention_db(db).unwrap();
        crate::commands::personas::retain_inbound_catalog_witness(
            &conn,
            &RetainedEvent {
                pending_sync: false,
                ..a_head.clone()
            },
        )
        .unwrap();
    };
    // The inbound 30176 team apply refreshes the team's head against B's
    // CURRENTLY hydrated personas. On a fresh device the personas arrive as
    // their own 30175 events; before they land, the team resolves against an
    // empty roster.
    let apply_team_refresh = |db: &Path, personas: &[AgentDefinition]| {
        resolve_and_refresh_or_retract_at(db, keys, &team(), personas).unwrap()
    };

    if catalog_before_constituents {
        // BROKEN order (relay newest-first, no reorder): witness lands, then the
        // team refresh runs while B has no personas → resolution fails → the
        // valid head is purged and falsely tombstoned.
        receive_head(db_path);
        apply_team_refresh(db_path, &[]);
    } else {
        // FIXED order (orderCatalogHeadsLast): constituents first. The team
        // refresh with no witness yet is a Noop (nothing to retract); personas
        // hydrate; THEN the witness lands last, with no further upsert to purge
        // it.
        apply_team_refresh(db_path, &[]);
        receive_head(db_path);
    }

    let head_present = retained_head(db_path, &owner).is_some();
    let conn = open_retention_db(db_path).unwrap();
    let tombstoned = get_pending_sync(&conn)
        .unwrap()
        .into_iter()
        .any(|row| row.kind == KIND_DELETE);
    (head_present, tombstoned)
}

#[test]
fn test_fresh_sync_retains_the_witness_when_catalog_heads_are_ordered_last() {
    // Carl r10 P1, finding 2. A shared a team; B first-syncs. In the FIXED order
    // (constituents before catalog heads) B must keep A's valid shared head and
    // queue NO false tombstone. The BROKEN relay-newest-first order is the
    // load-bearing reversal: it purges the witness and enqueues a dominating
    // false tombstone, deleting A's discoverable entry on ordinary first sync.
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();

    let device_a = scoped_db(dir.path(), "wss://a.example", &owner);
    prepare_team_publication_at(&device_a, &keys, &team(), &members(), Some(true)).unwrap();
    let a_head = retained_head(&device_a, &owner).unwrap();

    // FIXED order: witness survives, no tombstone.
    let device_b = scoped_db(dir.path(), "wss://b-fixed.example", &owner);
    let (head_present, tombstoned) = replay_fresh_sync_in_order(&device_b, &keys, &a_head, false);
    assert!(
        head_present,
        "ordering catalog heads last must retain A's valid shared witness"
    );
    assert!(
        !tombstoned,
        "the fixed order must NOT enqueue a false tombstone during first sync"
    );

    // Reversal (BROKEN relay order): the defect reproduces — witness purged and
    // falsely tombstoned. This is what `orderCatalogHeadsLast` prevents.
    let device_b_broken = scoped_db(dir.path(), "wss://b-broken.example", &owner);
    let (head_present_broken, tombstoned_broken) =
        replay_fresh_sync_in_order(&device_b_broken, &keys, &a_head, true);
    assert!(
        !head_present_broken,
        "reversal proof: catalog-first order purges the valid witness"
    );
    assert!(
        tombstoned_broken,
        "reversal proof: catalog-first order enqueues a dominating false tombstone"
    );
}

#[test]
fn test_fresh_sync_ordered_last_still_supersedes_on_a_later_edit() {
    // Convergence half: after the fixed-order first sync retains the witness,
    // B editing a member must still supersede A's head — the ordering fix must
    // not break the downstream edit/delete convergence.
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();

    let device_a = scoped_db(dir.path(), "wss://a.example", &owner);
    prepare_team_publication_at(&device_a, &keys, &team(), &members(), Some(true)).unwrap();
    let a_head = retained_head(&device_a, &owner).unwrap();

    let device_b = scoped_db(dir.path(), "wss://b.example", &owner);
    replay_fresh_sync_in_order(&device_b, &keys, &a_head, false);

    let edited = vec![member("m1", "Renamed On B"), member("m2", "Two")];
    let outcome = refresh_or_retract_shared_head_at(&device_b, &keys, &team(), &edited).unwrap();
    assert_eq!(
        outcome,
        RefreshOrRetractOutcome::Refreshed,
        "B must still supersede A's head after the ordered-last first sync"
    );
}
