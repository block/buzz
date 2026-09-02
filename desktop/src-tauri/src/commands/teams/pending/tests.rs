use super::*;
use crate::managed_agents::retention::{
    get_pending_sync, get_retained_event, open_retention_db, retain_event,
    scoped_retention_db_path, tombstone_retention_d_tag,
};
use buzz_core_pkg::kind::{event_is_shared, KIND_TEAM};
use nostr::JsonUtil;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const KIND_DELETE: u32 = 5;

fn member(id: &str, display_name: &str) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        display_name: display_name.to_string(),
        description: None,
        avatar_url: None,
        system_prompt: "Do the work.".to_string(),
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
        id: "team-abc".to_string(),
        name: "Catalog Team".to_string(),
        description: Some("A shared team".to_string()),
        instructions: None,
        persona_ids: vec!["m1".to_string(), "m2".to_string()],
        is_builtin: false,
        shared: false,
        catalog_source: None,
        source_dir: Some(PathBuf::from("/local/only/path")),
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "2026-07-30T00:00:00Z".to_string(),
        updated_at: "2026-07-30T00:00:00Z".to_string(),
    }
}

fn members() -> Vec<AgentDefinition> {
    vec![member("m1", "One"), member("m2", "Two")]
}

/// A retention database in its own scope directory, ready to write.
fn scoped_db(dir: &Path, relay_url: &str, owner: &str) -> PathBuf {
    let db_path = scoped_retention_db_path(dir, relay_url, owner);
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    db_path
}

fn retained_head(db_path: &Path, owner: &str) -> Option<RetainedEvent> {
    let conn = open_retention_db(db_path).unwrap();
    get_retained_event(&conn, KIND_TEAM_CATALOG, owner, "team-abc").unwrap()
}

// ── Publish / unshare ────────────────────────────────────────────────────────

#[test]
fn test_share_retains_a_pending_head_carrying_the_shared_tag() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    let (event, _, scoped_team) =
        prepare_team_publication_at(&db_path, &keys, &team(), &members(), Some(true)).unwrap();

    assert!(event_is_shared(&event));
    assert!(scoped_team.shared);
    let row = retained_head(&db_path, &owner).expect("the head is retained on share");
    assert!(row.pending_sync, "the flush loop must still owe a publish");
}

#[test]
fn test_unshare_publishes_a_newer_untagged_head_instead_of_deleting() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    let (shared_event, _, _) =
        prepare_team_publication_at(&db_path, &keys, &team(), &members(), Some(true)).unwrap();
    let (untagged_event, _, scoped_team) =
        prepare_team_publication_at(&db_path, &keys, &team(), &members(), Some(false)).unwrap();

    assert!(!event_is_shared(&untagged_event));
    assert!(!scoped_team.shared);
    assert!(
        untagged_event.created_at > shared_event.created_at,
        "the retraction must supersede the shared head monotonically"
    );
    let row = retained_head(&db_path, &owner).expect("unshare replaces the head, never deletes it");
    assert!(!retained_team_is_shared(Some(&row)));
    assert!(row.pending_sync);
}

#[test]
fn test_edit_without_an_override_preserves_the_scoped_share_state() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);
    prepare_team_publication_at(&db_path, &keys, &team(), &members(), Some(true)).unwrap();

    let mut edited = team();
    edited.name = "Renamed Team".to_string();
    let (event, _, scoped_team) =
        prepare_team_publication_at(&db_path, &keys, &edited, &members(), None).unwrap();

    assert!(
        scoped_team.shared && event_is_shared(&event),
        "an ordinary edit must not silently unshare the team"
    );
}

#[test]
fn test_share_state_is_scoped_by_relay_and_owner() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let community_a = scoped_db(dir.path(), "wss://a.example", &owner);
    let community_b = scoped_db(dir.path(), "wss://b.example", &owner);

    prepare_team_publication_at(&community_a, &keys, &team(), &members(), Some(true)).unwrap();
    let (_, _, in_b) =
        prepare_team_publication_at(&community_b, &keys, &team(), &members(), None).unwrap();

    assert!(!in_b.shared, "one community's share choice must not leak");
    assert!(retained_team_is_shared(
        retained_head(&community_a, &owner).as_ref()
    ));
    assert!(!retained_team_is_shared(
        retained_head(&community_b, &owner).as_ref()
    ));
}

#[test]
fn test_oversized_team_fails_before_anything_is_enqueued() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);
    let mut huge = member("m1", "One");
    huge.system_prompt =
        "x".repeat(crate::managed_agents::team_catalog::MAX_SYSTEM_PROMPT_BYTES + 1);

    let error =
        prepare_team_publication_at(&db_path, &keys, &team(), &[huge], Some(true)).unwrap_err();

    assert!(
        error.contains("the system prompt for 'One'"),
        "the error must name the oversized field, got: {error}"
    );
    assert!(
        retained_head(&db_path, &owner).is_none(),
        "a projection the relay would refuse must never reach the pending queue"
    );
}

// ── Projection ───────────────────────────────────────────────────────────────

#[test]
fn test_resolvable_scope_projects_the_retained_share_state() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);
    prepare_team_publication_at(&db_path, &keys, &team(), &members(), Some(true)).unwrap();
    let mut teams = vec![team()];

    project_scoped_team_sharing(
        Ok(RetentionScope {
            db_path,
            relay_url: "wss://a.example".to_string(),
            owner_keys: keys,
        }),
        &mut teams,
    );

    assert!(teams[0].shared);
}

#[test]
fn test_builtin_teams_project_as_unshared() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);
    // A head exists at the coordinate, so only the built-in guard can keep the
    // projection false.
    prepare_team_publication_at(&db_path, &keys, &team(), &members(), Some(true)).unwrap();
    let mut teams = vec![team()];
    teams[0].is_builtin = true;

    project_scoped_team_sharing(
        Ok(RetentionScope {
            db_path,
            relay_url: "wss://a.example".to_string(),
            owner_keys: keys,
        }),
        &mut teams,
    );

    assert!(!teams[0].shared, "built-in teams are never shareable");
}

#[test]
fn test_unresolvable_scope_projects_unshared_instead_of_failing() {
    let mut teams = vec![team()];
    teams[0].shared = true;
    // The real recovery-mode failure: `active_retention_scope` cannot resolve a
    // scope without signing keys, which is exactly what `identity_lost`
    // withholds.
    let state = crate::app_state::build_app_state();
    state
        .identity_lost
        .store(true, std::sync::atomic::Ordering::Release);
    let error = state
        .signing_keys()
        .expect_err("recovery mode must withhold signing keys");

    project_scoped_team_sharing(Err(error), &mut teams);

    assert!(
        !teams[0].shared,
        "an unresolvable scope degrades to unshared so list/create/update keep working"
    );
}

#[test]
fn test_unopenable_retention_db_projects_unshared_instead_of_failing() {
    let dir = tempfile::tempdir().unwrap();
    let mut teams = vec![team()];
    teams[0].shared = true;

    project_scoped_team_sharing(
        Ok(RetentionScope {
            // A directory cannot be opened as the retention database.
            db_path: dir.path().to_path_buf(),
            relay_url: "wss://a.example".to_string(),
            owner_keys: nostr::Keys::generate(),
        }),
        &mut teams,
    );

    assert!(!teams[0].shared);
}

// ── Tombstone ────────────────────────────────────────────────────────────────

#[test]
fn test_delete_purges_the_catalog_head_and_enqueues_a_tombstone() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);
    prepare_team_publication_at(&db_path, &keys, &team(), &members(), Some(true)).unwrap();

    tombstone_team_catalog_at(&db_path, &keys, "team-abc").unwrap();

    assert!(
        retained_head(&db_path, &owner).is_none(),
        "the purge must run first so an unpublished re-share cannot resurrect the entry"
    );
    let conn = open_retention_db(&db_path).unwrap();
    let pending = get_pending_sync(&conn).unwrap();
    let tombstone = pending
        .iter()
        .find(|row| row.kind == KIND_DELETE)
        .expect("the deletion is enqueued for the flush loop");
    assert_eq!(
        tombstone.d_tag,
        tombstone_retention_d_tag(KIND_TEAM_CATALOG, "team-abc")
    );
    assert!(tombstone.pending_sync, "an offline delete stays durable");
    let event = nostr::Event::from_json(&tombstone.raw_event).unwrap();
    assert!(
        event.tags.iter().any(|tag| tag.as_slice()
            == [
                "a".to_string(),
                format!("{KIND_TEAM_CATALOG}:{owner}:team-abc")
            ]),
        "the published deletion targets the 30178 coordinate"
    );
}

#[test]
fn test_catalog_tombstone_does_not_clobber_the_team_tombstone() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);
    let conn = open_retention_db(&db_path).unwrap();
    // The kind:30176 tombstone `delete_team` enqueues alongside this one. Both
    // carry kind 5 and the same team id, so only the folded-in target kind
    // keeps them on separate primary-key rows.
    retain_event(
        &conn,
        &RetainedEvent {
            kind: KIND_DELETE,
            pubkey: owner.clone(),
            d_tag: tombstone_retention_d_tag(KIND_TEAM, "team-abc"),
            content: String::new(),
            created_at: 1,
            raw_event: "{}".to_string(),
            pending_sync: true,
        },
    )
    .unwrap();

    tombstone_team_catalog_at(&db_path, &keys, "team-abc").unwrap();

    let mut keys_seen: Vec<String> = get_pending_sync(&conn)
        .unwrap()
        .into_iter()
        .filter(|row| row.kind == KIND_DELETE)
        .map(|row| row.d_tag)
        .collect();
    keys_seen.sort();
    assert_eq!(keys_seen, ["30176:team-abc", "30178:team-abc"]);
}

// ── F2 / I1 / I2: refresh_or_retract_shared_head_at ──────────────────────

#[test]
fn test_team_edit_refreshes_a_shared_head() {
    // After a team rename / member reorder, the 30178 content must reflect the
    // new state without waiting for the next workspace apply or restart.
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    // Initial share.
    prepare_team_publication_at(&db_path, &keys, &team(), &members(), Some(true)).unwrap();
    let before = retained_head(&db_path, &owner).unwrap();
    assert!(before.content.contains("One"));

    // Rename the member; refresh_or_retract_shared_head_at with shared_override:None
    // is what refresh_shared_team_catalog_head_resolving calls.
    let new_members = vec![member("m1", "Renamed"), member("m2", "Two")];
    refresh_or_retract_shared_head_at(&db_path, &keys, &team(), &new_members).unwrap();

    let after = retained_head(&db_path, &owner).unwrap();
    assert!(
        after.content.contains("Renamed"),
        "head must reflect the member rename immediately"
    );
    assert!(
        after.pending_sync,
        "the refreshed head must be queued for the flush loop"
    );
    // Shared tag must be preserved.
    let event = nostr::Event::from_json(&after.raw_event).unwrap();
    assert!(event_is_shared(&event), "refresh must not unshare the team");
}

#[test]
fn test_team_edit_retracts_immediately_when_projection_fails() {
    // A member edit that pushes past MAX_TOTAL_BYTES or MAX_SYSTEM_PROMPT_BYTES
    // must immediately purge+tombstone the shared head — not leave it public
    // until the next boot (I2).
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    // Initial share.
    prepare_team_publication_at(&db_path, &keys, &team(), &members(), Some(true)).unwrap();
    assert!(
        retained_head(&db_path, &owner).is_some(),
        "shared head exists"
    );

    // A member with a system_prompt that exceeds MAX_SYSTEM_PROMPT_BYTES (16 KiB)
    // causes build_team_catalog_event to fail.
    let mut oversized = member("m1", "One");
    oversized.system_prompt = "x".repeat(17 * 1024);
    let bad_members = vec![oversized, member("m2", "Two")];

    // refresh_or_retract_shared_head_at must succeed (Ok) even on projection
    // failure — the failure triggers a tombstone, not an error return.
    refresh_or_retract_shared_head_at(&db_path, &keys, &team(), &bad_members).unwrap();

    // The 30178 head must have been purged.
    let head_after = retained_head(&db_path, &owner);
    assert!(
        head_after.is_none(),
        "oversized projection must immediately purge the shared 30178 head"
    );

    // A kind:5 tombstone must be queued.
    let conn = open_retention_db(&db_path).unwrap();
    let pending = get_pending_sync(&conn).unwrap();
    assert!(
        pending.iter().any(|row| row.kind == 5),
        "a kind:5 tombstone must be queued after immediate retraction"
    );
}

#[test]
fn test_refresh_skips_never_shared_team() {
    // A never-shared team must produce no 30178 row even after refresh is
    // called — this is the I1 security guard.
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    // No retained head at all — simulate what an edit of a never-shared team sees.
    let result = refresh_or_retract_shared_head_at(&db_path, &keys, &team(), &members());
    assert!(result.is_ok(), "no-op must return Ok");

    // No head must have been written.
    assert!(
        retained_head(&db_path, &owner).is_none(),
        "never-shared team must produce no 30178 row after refresh"
    );
}

#[test]
fn test_refresh_skips_unshared_retained_head() {
    // A team with a retained unshared (retracted) head must also be a no-op —
    // only a live shared head triggers a refresh.
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    // Retain an unshared head (what unshare produces).
    prepare_team_publication_at(&db_path, &keys, &team(), &members(), Some(false)).unwrap();
    let before = retained_head(&db_path, &owner).unwrap();
    let before_content = before.content.clone();

    // Rename a member and call refresh — the unshared head must not be touched.
    let new_members = vec![member("m1", "Renamed"), member("m2", "Two")];
    refresh_or_retract_shared_head_at(&db_path, &keys, &team(), &new_members).unwrap();

    let after = retained_head(&db_path, &owner).unwrap();
    assert_eq!(
        after.content, before_content,
        "unshared head must not be refreshed"
    );
}

// ── CRITICAL: persona edit must only project team members ──────────────────
//
// These tests use `refresh_for_persona_at`, the file-based testable seam for
// `refresh_shared_team_catalog_heads_for_persona`, to verify that a persona
// edit never embeds unrelated local personas in the published 30178.

fn write_stores(base_dir: &std::path::Path, teams: &[TeamRecord], personas: &[AgentDefinition]) {
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

fn team_with_members(id: &str, name: &str, persona_ids: Vec<String>) -> TeamRecord {
    TeamRecord {
        id: id.to_string(),
        name: name.to_string(),
        description: None,
        instructions: None,
        persona_ids,
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

#[test]
fn test_persona_edit_only_projects_team_members_not_the_whole_store() {
    // CRITICAL: editing persona "m1" must only project m1 and m2 into the
    // shared 30178 — not "unrelated" (which happens to be in the persona store
    // but is not a member of the team).
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    let m1 = member("m1", "Member One.");
    let m2 = member("m2", "Member Two.");
    let unrelated = member("unrelated", "SECRET INSTRUCTIONS.");

    let t = team_with_members(
        "team-abc",
        "Catalog Team",
        vec!["m1".to_string(), "m2".to_string()],
    );

    // Pre-share the team head.
    prepare_team_publication_at(&db_path, &keys, &t, &[m1.clone(), m2.clone()], Some(true))
        .unwrap();

    // Write stores: 3 personas (2 team members + 1 unrelated).
    write_stores(
        dir.path(),
        &[t],
        &[m1.clone(), m2.clone(), unrelated.clone()],
    );

    // Simulate a persona edit on "m1".
    super::refresh_for_persona_at(dir.path(), &keys, &db_path, "m1").unwrap();

    // The resulting 30178 must contain m1 and m2 — never "unrelated".
    let head = retained_head(&db_path, &owner).expect("shared head must still exist");
    let event = nostr::Event::from_json(&head.raw_event).unwrap();
    assert!(
        event_is_shared(&event),
        "the team must remain discoverable after a member edit"
    );
    assert!(
        head.content.contains("Member One."),
        "the edited persona's content must be in the 30178"
    );
    assert!(
        head.content.contains("Member Two."),
        "the other team member must be in the 30178"
    );
    assert!(
        !head.content.contains("SECRET INSTRUCTIONS."),
        "unrelated personas must NEVER appear in the 30178 projection"
    );
}

#[test]
fn test_persona_edit_does_not_publish_for_never_shared_team() {
    // A persona that belongs to a never-shared team must produce no 30178
    // even when the persona is edited and the store has many other personas.
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    let m1 = member("m1", "Member One.");
    let t = team_with_members("team-abc", "Catalog Team", vec!["m1".to_string()]);

    // No shared head — the team was never shared.
    write_stores(dir.path(), &[t], &[m1]);

    super::refresh_for_persona_at(dir.path(), &keys, &db_path, "m1").unwrap();

    assert!(
        retained_head(&db_path, &owner).is_none(),
        "persona edit on a never-shared team must not produce a 30178 row"
    );
}

#[test]
fn test_persona_edit_tombstones_when_another_member_is_missing() {
    // If m2 is deleted from the persona store while the team is still shared,
    // an edit of m1 must tombstone the shared head rather than publishing a
    // projection that is missing a team member.
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    let m1 = member("m1", "Member One.");
    let m2 = member("m2", "Member Two.");
    let t = team_with_members(
        "team-abc",
        "Catalog Team",
        vec!["m1".to_string(), "m2".to_string()],
    );

    // Pre-share with both members.
    prepare_team_publication_at(&db_path, &keys, &t, &[m1.clone(), m2.clone()], Some(true))
        .unwrap();

    // m2 is gone from the store — team is now unresolvable.
    write_stores(dir.path(), &[t], &[m1]);

    super::refresh_for_persona_at(dir.path(), &keys, &db_path, "m1").unwrap();

    // The shared head must be purged (tombstoned).
    assert!(
        retained_head(&db_path, &owner).is_none(),
        "unresolvable team must be tombstoned, not left with stale members"
    );
    let conn = open_retention_db(&db_path).unwrap();
    let pending = get_pending_sync(&conn).unwrap();
    assert!(
        pending.iter().any(|r| r.kind == 5),
        "a kind:5 tombstone must be queued"
    );
}

// ── Typed outcome ─────────────────────────────────────────────────────────

#[test]
fn test_refresh_returns_refreshed_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    prepare_team_publication_at(&db_path, &keys, &team(), &members(), Some(true)).unwrap();

    // A rebuild whose content differs from the retained head returns Refreshed.
    // (Identical content returns Noop — see the idempotency test below.)
    let new_members = vec![member("m1", "Renamed"), member("m2", "Two")];
    let outcome =
        refresh_or_retract_shared_head_at(&db_path, &keys, &team(), &new_members).unwrap();

    assert_eq!(
        outcome,
        RefreshOrRetractOutcome::Refreshed,
        "a rebuild that changes the projection must return Refreshed"
    );
}

#[test]
fn test_refresh_is_idempotent_when_rebuild_matches_the_retained_head() {
    // Cross-device convergence guard: an owner's edit refreshes device A's head
    // AND is re-applied inbound on device B, which rebuilds the SAME content. If
    // that rebuild republished, the two devices would churn identical heads at
    // each other. A byte-identical rebuild must be a no-op.
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    prepare_team_publication_at(&db_path, &keys, &team(), &members(), Some(true)).unwrap();
    let before = retained_head(&db_path, &owner).unwrap();

    let outcome = refresh_or_retract_shared_head_at(&db_path, &keys, &team(), &members()).unwrap();

    assert_eq!(
        outcome,
        RefreshOrRetractOutcome::Noop,
        "a rebuild matching the retained head must not republish"
    );
    let after = retained_head(&db_path, &owner).unwrap();
    assert_eq!(
        after.created_at, before.created_at,
        "an unchanged projection must not bump the head's created_at"
    );
    assert_eq!(
        after.content, before.content,
        "the retained head content must be untouched"
    );
}

#[test]
fn test_refresh_returns_noop_for_never_shared_team() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    // No retained head at all.
    let outcome = refresh_or_retract_shared_head_at(&db_path, &keys, &team(), &members()).unwrap();

    assert_eq!(
        outcome,
        RefreshOrRetractOutcome::Noop,
        "no retained head must return Noop"
    );
    let _ = owner; // suppress unused warning
}

#[test]
fn test_refresh_returns_removal_queued_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    prepare_team_publication_at(&db_path, &keys, &team(), &members(), Some(true)).unwrap();

    let mut oversized = member("m1", "One");
    oversized.system_prompt =
        "x".repeat(crate::managed_agents::team_catalog::MAX_SYSTEM_PROMPT_BYTES + 1);
    let bad = vec![oversized, member("m2", "Two")];

    let outcome = refresh_or_retract_shared_head_at(&db_path, &keys, &team(), &bad).unwrap();

    assert!(
        matches!(outcome, RefreshOrRetractOutcome::RemovalQueued { .. }),
        "projection failure must return RemovalQueued, got {outcome:?}"
    );
    let _ = owner;
}

// ── Wes P1: tombstone created_at must dominate a future-dated head ──────────

use crate::managed_agents::team_catalog::{
    build_team_catalog_event, tombstone_team_catalog_coordinate,
};

/// Seed a retained 30178 head dated `created_at` seconds since epoch.
fn seed_catalog_head(db_path: &Path, keys: &nostr::Keys, created_at: i64) {
    let event = build_team_catalog_event(&team(), &[member("m1", "One")], true)
        .unwrap()
        .custom_created_at(nostr::Timestamp::from(created_at as u64))
        .sign_with_keys(keys)
        .unwrap();
    let conn = open_retention_db(db_path).unwrap();
    retain_event(
        &conn,
        &RetainedEvent {
            kind: KIND_TEAM_CATALOG,
            pubkey: keys.public_key().to_hex(),
            d_tag: "team-abc".to_string(),
            content: event.content.to_string(),
            created_at,
            raw_event: event.as_json(),
            pending_sync: false,
        },
    )
    .unwrap();
}

fn enqueued_tombstone(db_path: &Path) -> RetainedEvent {
    let conn = open_retention_db(db_path).unwrap();
    get_pending_sync(&conn)
        .unwrap()
        .into_iter()
        .find(|row| row.kind == KIND_DELETE)
        .expect("a kind:5 tombstone is enqueued")
}

#[test]
fn test_catalog_tombstone_created_at_strictly_dominates_a_future_dated_head() {
    // The retained 30178 head may be future-dated (monotonic_created_at bumps a
    // same-second re-share past the prior head). The relay only soft-deletes
    // coordinate versions with created_at <= the tombstone's, so a kind:5 signed
    // at wall-clock `now` would leave the head live forever once its local
    // retry witness is purged.
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    let future = nostr::Timestamp::now().as_secs() as i64 + 86_400;
    seed_catalog_head(&db_path, &keys, future);

    tombstone_team_catalog_at(&db_path, &keys, "team-abc").unwrap();

    let tombstone = enqueued_tombstone(&db_path);
    assert!(
        tombstone.created_at > future,
        "tombstone created_at ({}) must strictly dominate the future-dated head ({future})",
        tombstone.created_at
    );
}

#[test]
fn test_catalog_tombstone_with_no_head_falls_back_to_wall_clock() {
    // No retained head: monotonic_created_at(None) floors at 0, so the tombstone
    // is dated at wall-clock `now` and is still a valid, publishable kind:5.
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    let before = nostr::Timestamp::now().as_secs() as i64;
    tombstone_team_catalog_at(&db_path, &keys, "team-abc").unwrap();
    let after = nostr::Timestamp::now().as_secs() as i64;

    let tombstone = enqueued_tombstone(&db_path);
    assert!(
        tombstone.created_at >= before && tombstone.created_at <= after,
        "no-head tombstone is dated at wall clock; got {}",
        tombstone.created_at
    );
}

#[test]
fn test_all_catalog_call_paths_produce_a_dominating_tombstone() {
    // Direct delete, edit-retraction, and boot-reconcile all converge on
    // tombstone_team_catalog_coordinate. Asserting the single helper dominates a
    // future-dated head across a range of offsets covers the guarantee every
    // caller inherits.
    for offset in [1_i64, 3_600, 86_400] {
        let dir = tempfile::tempdir().unwrap();
        let keys = nostr::Keys::generate();
        let owner = keys.public_key().to_hex();
        let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

        let future = nostr::Timestamp::now().as_secs() as i64 + offset;
        seed_catalog_head(&db_path, &keys, future);

        tombstone_team_catalog_coordinate(&db_path, &keys, "team-abc").unwrap();

        let tombstone = enqueued_tombstone(&db_path);
        assert!(
            tombstone.created_at > future,
            "offset {offset}: tombstone {} must dominate head {future}",
            tombstone.created_at
        );
    }
}

mod cross_device;
mod gate;
