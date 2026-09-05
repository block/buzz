//! Regression coverage for the SELF-AUTHORED overlay write-through (defect 5):
//! the overlay only ever learned config from events this device *received*, so
//! a second edit in the same session resolved a stale patch onto the fresher
//! disk record and published a silent revert of the first edit.
//!
//! Split out of `stale_republish_tests.rs` to stay inside the desktop
//! file-size ratchet. Shares the parent module's fixtures (`sample_record`,
//! `private_payload_from_record`, `retain_agent_record`) via `use super::*`.

use super::*;
use crate::managed_agents::private_config_overlay::{hydrate_from_retention, PrivateConfigOverlay};

/// The production body of `retain_managed_agent_pending`
/// (`commands/agents.rs:42`) minus its `AppHandle`/`AppState` plumbing: retain,
/// then write the just-retained head through to the overlay. The command is a
/// `#[tauri::command]` descendant needing a live `AppHandle`, so this models
/// the ordering; `retain_managed_agent_pending_writes_through_to_the_overlay`
/// below pins that production actually calls it.
fn retain_and_absorb(
    conn: &rusqlite::Connection,
    overlay: &mut PrivateConfigOverlay,
    owner_keys: &nostr::Keys,
    record: &ManagedAgentRecord,
) {
    retain_agent_record(conn, owner_keys, record).unwrap();
    overlay
        .absorb_retained_head(conn, owner_keys, &record.pubkey)
        .unwrap();
}

fn published_head(
    conn: &rusqlite::Connection,
    owner_keys: &nostr::Keys,
    pubkey: &str,
) -> buzz_core_pkg::private_managed_agent::Payload {
    let row = get_retained_event(
        conn,
        KIND_PRIVATE_MANAGED_AGENT,
        &owner_keys.public_key().to_hex(),
        pubkey,
    )
    .unwrap()
    .unwrap();
    let (_, payload) = private_managed_agent::validate_and_decrypt(
        &nostr::Event::from_json(&row.raw_event).unwrap(),
        owner_keys,
    )
    .unwrap();
    payload
}

/// SAMI RED-FIRST (defect 5, the live gate red Max captured): two edits in ONE
/// session on ONE device. The first edit publishes parallelism 19; the second
/// edit — a rename, touching no other field — must not take parallelism back
/// to the received head's 17.
///
/// Discriminator is the PARALLELISM, not the name: Max retracted the Device-B
/// attribution (his broker had a single global native connection and no device
/// routing), so the name-reversion leg is unattributed. The
/// gen3-returns-19 → gen4-publishes-17 sequence is receipt-backed on a single
/// backend regardless of which app served it.
#[test]
fn sami_second_edit_in_one_session_preserves_the_first_edit() {
    let dir = TempDir::new().unwrap();
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    let owner_hex = owner_keys.public_key().to_hex();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();

    // Gen 2: the head this device RECEIVED, parallelism 17. Seeded through the
    // real inbound path so the overlay is hydrated exactly as boot hydrates it.
    let mut received = sample_record(&pubkey, "Fizz Relay");
    received.private_key_nsec = agent_keys.secret_key().to_bech32().unwrap();
    received.parallelism = 17;
    let received_created_at = nostr::Timestamp::now().as_secs() as i64;
    let received_payload =
        private_payload_from_record(&received, &owner_hex, 2, Some("aa".repeat(32))).unwrap();
    let received_event = private_managed_agent::build_event(
        &owner_keys,
        &received_payload,
        received_created_at as u64,
    )
    .unwrap();
    crate::managed_agents::retention::retain_inbound_event(
        &conn,
        &RetainedEvent {
            kind: KIND_PRIVATE_MANAGED_AGENT,
            pubkey: owner_hex.clone(),
            d_tag: pubkey.clone(),
            content: received_event.content.clone(),
            created_at: received_created_at,
            raw_event: received_event.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();
    let mut overlay = hydrate_from_retention(&conn, &owner_keys).unwrap();
    assert_eq!(overlay.len(), 1, "fixture: the overlay follows gen 2");

    // Disk agrees with the head at the start of the session.
    let mut disk = received.clone();

    // EDIT 1 — parallelism 17 -> 19. `update_managed_agent`'s shape: resolve
    // the overlay onto the disk record, apply the user's patch, SAVE to disk,
    // then retain.
    let mut edited = overlay.resolve_local_record(&disk);
    edited.parallelism = 19;
    disk = edited.clone();
    retain_and_absorb(&conn, &mut overlay, &owner_keys, &disk);

    let after_edit = published_head(&conn, &owner_keys, &pubkey);
    assert_eq!(after_edit.generation, 3);
    assert_eq!(
        after_edit.config.parallelism,
        Some(19),
        "edit 1 published the user's value"
    );

    // EDIT 2 — a rename in the SAME session, touching only `name`. Same shape.
    let mut renamed = overlay.resolve_local_record(&disk);
    renamed.name = "Fizz Relay Rename".into();
    disk = renamed.clone();
    retain_and_absorb(&conn, &mut overlay, &owner_keys, &disk);

    let after_rename = published_head(&conn, &owner_keys, &pubkey);
    assert_eq!(after_rename.generation, 4);
    // The intended write lands...
    assert_eq!(after_rename.config.name, "Fizz Relay Rename");
    // ...and the FIRST edit survives it. Without the write-through this is
    // Some(17): the overlay is still pinned at the received gen-2 patch, so
    // resolving it onto disk reverts parallelism, and the revert publishes as
    // an audit-clean gen-4 successor that wins LWW.
    assert_eq!(
        after_rename.config.parallelism,
        Some(19),
        "the rename reverted the previous edit's parallelism from the stale overlay"
    );
    // The revert is durable, not just in-flight: it was written back to disk.
    assert_eq!(disk.parallelism, 19, "the reverted value also reached disk");

    // NEGATIVE CONTROL: the same sequence with the write-through omitted must
    // FAIL to preserve the edit, or the assertion above is vacuous — it would
    // pass on a fixture where the overlay never had a competing value at all.
    let control_conn = open_retention_db(&dir.path().join("control.db")).unwrap();
    crate::managed_agents::retention::retain_inbound_event(
        &control_conn,
        &RetainedEvent {
            kind: KIND_PRIVATE_MANAGED_AGENT,
            pubkey: owner_hex.clone(),
            d_tag: pubkey.clone(),
            content: received_event.content.clone(),
            created_at: received_created_at,
            raw_event: received_event.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();
    let control_overlay = hydrate_from_retention(&control_conn, &owner_keys).unwrap();
    let mut control_disk = control_overlay.resolve_local_record(&received);
    control_disk.parallelism = 19;
    retain_agent_record(&control_conn, &owner_keys, &control_disk).unwrap();
    let mut control_renamed = control_overlay.resolve_local_record(&control_disk);
    control_renamed.name = "Fizz Relay Rename".into();
    retain_agent_record(&control_conn, &owner_keys, &control_renamed).unwrap();
    let control = published_head(&control_conn, &owner_keys, &pubkey);
    assert_eq!(
        control.config.parallelism,
        Some(17),
        "control: WITHOUT the write-through the same sequence reverts to the \
         received head's 17 — this is the defect the test above pins as fixed"
    );
}

/// The write-through must never CLEAR what the overlay is following. A retain
/// that produced no private row (empty nsec — the keyring-only case) or a
/// coordinate with no head at all must leave the existing entry alone.
#[test]
fn absorb_retained_head_leaves_the_overlay_alone_when_there_is_no_head() {
    let dir = TempDir::new().unwrap();
    let owner_keys = nostr::Keys::generate();
    let agent_keys = nostr::Keys::generate();
    let pubkey = agent_keys.public_key().to_hex();
    let owner_hex = owner_keys.public_key().to_hex();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();

    let mut record = sample_record(&pubkey, "followed");
    record.private_key_nsec = agent_keys.secret_key().to_bech32().unwrap();
    record.parallelism = 16;
    let payload = private_payload_from_record(&record, &owner_hex, 1, None).unwrap();
    let event = private_managed_agent::build_event(
        &owner_keys,
        &payload,
        nostr::Timestamp::now().as_secs(),
    )
    .unwrap();
    crate::managed_agents::retention::retain_inbound_event(
        &conn,
        &RetainedEvent {
            kind: KIND_PRIVATE_MANAGED_AGENT,
            pubkey: owner_hex.clone(),
            d_tag: pubkey.clone(),
            content: event.content.clone(),
            created_at: event.created_at.as_secs() as i64,
            raw_event: event.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();
    let mut overlay = hydrate_from_retention(&conn, &owner_keys).unwrap();

    // A DIFFERENT db with no rows at all: absorbing must not drop the entry.
    let empty = open_retention_db(&dir.path().join("empty.db")).unwrap();
    overlay
        .absorb_retained_head(&empty, &owner_keys, &pubkey)
        .unwrap();
    assert_eq!(
        overlay.len(),
        1,
        "a missing head must not clear the overlay"
    );
    let mut disk = sample_record(&pubkey, "disk-name");
    disk.parallelism = 1;
    assert_eq!(
        overlay.resolve_local_record(&disk).parallelism,
        16,
        "the followed head is still applied after a no-op absorb"
    );

    // POSITIVE CONTROL: the same call against the db that DOES hold a newer
    // head updates the overlay, so the no-op above is the absent row and not a
    // broken instrument.
    let mut newer = record.clone();
    newer.parallelism = 24;
    retain_agent_record(&conn, &owner_keys, &newer).unwrap();
    overlay
        .absorb_retained_head(&conn, &owner_keys, &pubkey)
        .unwrap();
    assert_eq!(
        overlay.resolve_local_record(&disk).parallelism,
        24,
        "control: absorbing a present head DOES update the overlay"
    );
}

/// Source guard for the seam. Both behavioural tests above model
/// `retain_managed_agent_pending`'s body — every caller is inside a
/// `#[tauri::command]` needing a live `AppHandle`, so deleting the production
/// write-through leaves them green. Same weakness, same remedy, as
/// `write_site_resolve_guard` in `private_config_overlay.rs`: assert the call
/// exists in the one helper that all five writers funnel through.
#[cfg(test)]
mod retain_managed_agent_pending_writes_through_to_the_overlay {
    const AGENTS_PENDING_RS: &str = include_str!("../../../commands/agents_pending.rs");

    /// Exactly one — the single seam. A second copy would mean a per-caller
    /// write-through crept back in, which is the shape this fix replaced.
    #[test]
    fn agents_pending_rs_absorbs_the_retained_head_exactly_once() {
        assert_eq!(
            AGENTS_PENDING_RS.matches("absorb_retained_head(").count(),
            1,
            "commands/agents_pending.rs must write the just-retained 30179 head back to \
             the overlay exactly once, in `retain_managed_agent_pending`. Without \
             it the overlay stays pinned at the last RECEIVED generation and the \
             next edit republishes a revert of the previous one (see \
             `sami_second_edit_in_one_session_preserves_the_first_edit`)."
        );
    }

    /// Positional, not semantic: the guard above is a substring count and
    /// cannot see ordering. Absorbing BEFORE the retain would read the
    /// previous head and reintroduce the defect one generation later, so pin
    /// the source order too — the behavioural test pins the runtime ordering.
    #[test]
    fn the_absorb_follows_the_retain() {
        let retain = AGENTS_PENDING_RS
            .find("retain_agent_record(&conn")
            .expect("positive control: the retain call must be present");
        let absorb = AGENTS_PENDING_RS
            .find("absorb_retained_head(")
            .expect("positive control: the absorb call must be present");
        assert!(
            retain < absorb,
            "the overlay must absorb the head AFTER the retain writes it"
        );
    }

    /// Negative control: prove the searched strings are load-bearing. Without
    /// this a typo in either literal makes both guards vacuous.
    #[test]
    fn the_guard_can_fail() {
        let stripped = AGENTS_PENDING_RS.replace("absorb_retained_head(", "REMOVED(");
        assert_eq!(stripped.matches("absorb_retained_head(").count(), 0);
        assert_ne!(
            AGENTS_PENDING_RS
                .matches("retain_agent_record(&conn")
                .count(),
            0
        );
    }
}
