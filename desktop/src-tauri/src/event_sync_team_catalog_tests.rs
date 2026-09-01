use super::*;
use crate::managed_agents::{
    retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
    team_catalog::build_team_catalog_event,
    AgentDefinition, TeamRecord,
};
use buzz_core_pkg::kind::{event_is_shared, KIND_TEAM_CATALOG};
use nostr::JsonUtil;
use std::collections::BTreeMap;

const TEAM_ID: &str = "team-alpha";

fn member(id: &str, prompt: &str) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        display_name: id.to_string(),
        description: None,
        avatar_url: None,
        system_prompt: prompt.to_string(),
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
        team_catalog_source: None,
        env_vars: BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: "2026-07-30T00:00:00Z".to_string(),
        updated_at: "2026-07-30T00:00:00Z".to_string(),
    }
}

fn team() -> TeamRecord {
    TeamRecord {
        id: TEAM_ID.to_string(),
        name: "Alpha".to_string(),
        description: None,
        instructions: None,
        persona_ids: vec!["m1".to_string()],
        is_builtin: false,
        shared: false,
        catalog_source: None,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "2026-07-30T00:00:00Z".to_string(),
        updated_at: "2026-07-30T00:00:00Z".to_string(),
    }
}

fn write_stores(base_dir: &Path, teams: &[TeamRecord], personas: &[AgentDefinition]) {
    std::fs::write(
        base_dir.join("teams.json"),
        serde_json::to_string(teams).unwrap(),
    )
    .unwrap();
    std::fs::write(
        base_dir.join("personas.json"),
        serde_json::to_string(personas).unwrap(),
    )
    .unwrap();
}

/// Retain a catalog head for `team`/`members`, as the share toggle would.
fn retain_head(
    base_dir: &Path,
    keys: &nostr::Keys,
    team: &TeamRecord,
    members: &[AgentDefinition],
) {
    let event = build_team_catalog_event(team, members, true)
        .unwrap()
        .sign_with_keys(keys)
        .unwrap();
    let conn = open_retention_db(&base_dir.join("retention.db")).unwrap();
    retain_event(
        &conn,
        &RetainedEvent {
            kind: KIND_TEAM_CATALOG,
            pubkey: keys.public_key().to_hex(),
            d_tag: team.id.clone(),
            content: event.content.to_string(),
            created_at: event.created_at.as_secs() as i64,
            raw_event: event.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();
}

fn head(base_dir: &Path, keys: &nostr::Keys) -> Option<RetainedEvent> {
    let conn = open_retention_db(&base_dir.join("retention.db")).unwrap();
    get_retained_event(
        &conn,
        KIND_TEAM_CATALOG,
        &keys.public_key().to_hex(),
        TEAM_ID,
    )
    .unwrap()
}

fn reconcile(base_dir: &Path, keys: &nostr::Keys) -> Result<u32, String> {
    crate::event_sync::reconcile_team_catalog_heads_at_for_test(
        base_dir,
        keys,
        &base_dir.join("retention.db"),
    )
}

fn head_is_shared(row: &RetainedEvent) -> bool {
    event_is_shared(&nostr::Event::from_json(&row.raw_event).unwrap())
}

#[test]
fn test_member_edit_republishes_a_newer_shared_head() {
    let base = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    retain_head(base.path(), &keys, &team(), &[member("m1", "Original.")]);
    let before = head(base.path(), &keys).unwrap();
    // The team is untouched; only the member's prompt changed, which the
    // publish path never observes.
    write_stores(base.path(), &[team()], &[member("m1", "Rewritten.")]);

    assert_eq!(reconcile(base.path(), &keys).unwrap(), 1);

    let after = head(base.path(), &keys).unwrap();
    assert!(after.content.contains("Rewritten."));
    assert!(head_is_shared(&after), "a refresh stays discoverable");
    assert!(
        after.pending_sync,
        "the refreshed head is queued to publish"
    );
    assert!(after.created_at > before.created_at);
}

#[test]
fn test_unchanged_team_is_left_alone() {
    let base = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    retain_head(base.path(), &keys, &team(), &[member("m1", "Original.")]);
    write_stores(base.path(), &[team()], &[member("m1", "Original.")]);

    assert_eq!(reconcile(base.path(), &keys).unwrap(), 0);

    assert!(
        !head(base.path(), &keys).unwrap().pending_sync,
        "an unchanged team must not churn pending_sync on every boot"
    );
}

#[test]
fn test_deleted_member_tombstones_the_coordinate() {
    // I4: a member disappears making the team unrebuildable. The reconcile
    // must purge+tombstone the coordinate (not retain a stale-body unshared
    // head), and the tombstone must be queued for the flush loop.
    let base = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    retain_head(base.path(), &keys, &team(), &[member("m1", "Original.")]);
    // The member is gone, so the team can no longer be projected at all.
    write_stores(base.path(), &[team()], &[]);

    assert_eq!(reconcile(base.path(), &keys).unwrap(), 1);

    // The 30178 row must be purged (not merely unshared).
    assert!(
        head(base.path(), &keys).is_none(),
        "unrebuildable team must purge the 30178 row, not retain a stale-body unshared head"
    );

    // A kind:5 tombstone must be queued.
    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    let pending = crate::managed_agents::retention::get_pending_sync(&conn).unwrap();
    assert!(
        pending.iter().any(|row| row.kind == 5),
        "a kind:5 tombstone must be queued after purge"
    );
}

#[test]
fn test_tombstone_is_not_repeated_on_next_boot() {
    // After the first boot tombstones the unrebuildable head (purging the 30178
    // row), the next boot must see no 30178 head and do nothing.
    let base = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    retain_head(base.path(), &keys, &team(), &[member("m1", "Original.")]);
    write_stores(base.path(), &[team()], &[]);
    reconcile(base.path(), &keys).unwrap();

    assert_eq!(
        reconcile(base.path(), &keys).unwrap(),
        0,
        "no 30178 head remains after tombstone, so nothing to do"
    );
}

#[test]
fn test_unshared_head_is_never_touched() {
    let base = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    // An unshared head with a member that no longer exists — the retraction
    // trigger — must still be left alone: it is not discoverable.
    let event = build_team_catalog_event(&team(), &[member("m1", "Original.")], false)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    retain_event(
        &conn,
        &RetainedEvent {
            kind: KIND_TEAM_CATALOG,
            pubkey: keys.public_key().to_hex(),
            d_tag: TEAM_ID.to_string(),
            content: event.content.to_string(),
            created_at: event.created_at.as_secs() as i64,
            raw_event: event.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();
    write_stores(base.path(), &[team()], &[]);

    assert_eq!(reconcile(base.path(), &keys).unwrap(), 0);

    assert!(!head(base.path(), &keys).unwrap().pending_sync);
}

#[test]
fn test_team_with_no_head_is_skipped() {
    let base = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    write_stores(base.path(), &[team()], &[member("m1", "Original.")]);

    assert_eq!(
        reconcile(base.path(), &keys).unwrap(),
        0,
        "a team the owner never shared must not be published by a boot reconcile"
    );
    assert!(head(base.path(), &keys).is_none());
}

#[test]
fn test_members_are_read_from_the_unified_agent_store_after_the_fold() {
    let base = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    retain_head(base.path(), &keys, &team(), &[member("m1", "Original.")]);
    // Post-fold there is no personas.json; definitions are key-less records in
    // managed-agents.json. Reading only personas.json would see zero members
    // and retract every shared team on the next boot.
    std::fs::write(
        base.path().join("teams.json"),
        serde_json::to_string(&[team()]).unwrap(),
    )
    .unwrap();
    let folded: Vec<crate::managed_agents::ManagedAgentRecord> =
        vec![member("m1", "Original.").into_agent_record()];
    std::fs::write(
        base.path().join("managed-agents.json"),
        serde_json::to_string(&folded).unwrap(),
    )
    .unwrap();

    assert_eq!(reconcile(base.path(), &keys).unwrap(), 0);

    let after = head(base.path(), &keys).unwrap();
    assert!(head_is_shared(&after), "the team must not be retracted");
    assert!(!after.pending_sync);
}

#[test]
fn test_builtin_teams_are_skipped() {
    let base = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    retain_head(base.path(), &keys, &team(), &[member("m1", "Original.")]);
    let mut builtin = team();
    builtin.is_builtin = true;
    write_stores(base.path(), &[builtin], &[]);

    assert_eq!(reconcile(base.path(), &keys).unwrap(), 0);
}

#[test]
fn test_deleted_team_with_shared_head_is_tombstoned_at_reconcile() {
    // F1: a team is deleted after it was shared. `delete_team` is best-effort
    // for the tombstone; a crash there (or any failure) leaves the shared head
    // visible indefinitely until the next boot reconcile. The reconcile must
    // see the orphaned head via the retained-coordinate worklist and tombstone
    // it — it cannot rely on the team still existing in the store.
    let base = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    retain_head(base.path(), &keys, &team(), &[member("m1", "Original.")]);
    assert!(!head(base.path(), &keys).unwrap().pending_sync);

    // Simulate the team having been deleted: write empty stores, as if the
    // team record was removed before the tombstone helper ran.
    write_stores(base.path(), &[], &[]);

    assert_eq!(reconcile(base.path(), &keys).unwrap(), 1);

    // The 30178 coordinate is gone from the retention store (tombstone_team_catalog_at
    // purges it and enqueues a kind:5 in its place). Verify the head is absent.
    assert!(
        head(base.path(), &keys).is_none(),
        "the orphaned shared head must be purged from the retention store"
    );
}

#[test]
fn test_deleted_team_tombstone_is_not_repeated_on_next_boot() {
    // After the first boot tombstones the orphaned head (purging the 30178
    // row), the next boot must see no 30178 heads and do nothing.
    let base = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    retain_head(base.path(), &keys, &team(), &[member("m1", "Original.")]);
    write_stores(base.path(), &[], &[]);
    reconcile(base.path(), &keys).unwrap();

    assert_eq!(
        reconcile(base.path(), &keys).unwrap(),
        0,
        "no 30178 head remains, so nothing to tombstone"
    );
}

// ── I2: Multi-head continuation ─────────────────────────────────────────────

fn team_b() -> TeamRecord {
    TeamRecord {
        id: "team-beta".to_string(),
        name: "Beta".to_string(),
        description: None,
        instructions: None,
        persona_ids: vec!["m2".to_string()],
        is_builtin: false,
        shared: false,
        catalog_source: None,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "2026-07-30T00:00:00Z".to_string(),
        updated_at: "2026-07-30T00:00:00Z".to_string(),
    }
}

fn head_for(base_dir: &Path, keys: &nostr::Keys, team_id: &str) -> Option<RetainedEvent> {
    let conn = open_retention_db(&base_dir.join("retention.db")).unwrap();
    get_retained_event(
        &conn,
        KIND_TEAM_CATALOG,
        &keys.public_key().to_hex(),
        team_id,
    )
    .unwrap()
}

#[test]
fn test_two_unrebuildable_teams_are_both_tombstoned_in_one_reconcile() {
    // I2: when two shared teams cannot be reprojected, BOTH must be tombstoned
    // in a single boot reconcile — not just the first one, with the second
    // waiting for the next boot (the original `drop(conn); return` bug).
    let base = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();

    // Share two teams.
    retain_head(base.path(), &keys, &team(), &[member("m1", "Alpha.")]);
    retain_head(base.path(), &keys, &team_b(), &[member("m2", "Beta.")]);

    // Both members vanish — both teams are unrebuildable.
    write_stores(base.path(), &[team(), team_b()], &[]);

    // One reconcile must tombstone both.
    let count = reconcile(base.path(), &keys).unwrap();
    assert_eq!(count, 2, "both tombstones must be applied in one pass");

    // Both 30178 heads must be gone.
    assert!(
        head_for(base.path(), &keys, TEAM_ID).is_none(),
        "team-alpha 30178 head must be purged"
    );
    assert!(
        head_for(base.path(), &keys, "team-beta").is_none(),
        "team-beta 30178 head must be purged"
    );

    // Both kind:5 tombstones must be queued.
    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    let pending = crate::managed_agents::retention::get_pending_sync(&conn).unwrap();
    let tombstones: Vec<_> = pending.iter().filter(|r| r.kind == 5).collect();
    assert_eq!(
        tombstones.len(),
        2,
        "two kind:5 tombstones must be queued (one per team)"
    );
}

#[test]
fn test_one_valid_one_unrebuildable_team_both_processed() {
    // Continuation must also work when only one of two teams fails rebuild:
    // the failed team gets tombstoned, the valid team gets refreshed.
    let base = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();

    retain_head(base.path(), &keys, &team(), &[member("m1", "Alpha.")]);
    retain_head(base.path(), &keys, &team_b(), &[member("m2", "Beta.")]);

    // team-alpha's m1 disappears; team-beta's m2 stays but with a new prompt.
    write_stores(
        base.path(),
        &[team(), team_b()],
        &[member("m2", "Beta revised.")],
    );

    let count = reconcile(base.path(), &keys).unwrap();
    assert_eq!(count, 2, "one tombstone + one refresh = 2 reconciled");

    // team-alpha must be tombstoned.
    assert!(head_for(base.path(), &keys, TEAM_ID).is_none());

    // team-beta must still have a shared head with the new content.
    let beta_head = head_for(base.path(), &keys, "team-beta").unwrap();
    assert!(
        beta_head.content.contains("Beta revised."),
        "team-beta must reflect the updated member prompt"
    );
    assert!(
        head_is_shared(&beta_head),
        "the refreshed team-beta must remain discoverable"
    );
}
