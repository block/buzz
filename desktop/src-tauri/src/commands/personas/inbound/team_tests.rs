//! Team (kind:30176) inbound reconciliation tests. Split from
//! `inbound_tests.rs` to keep it under the file-size cap; `use super::*` brings
//! in the parent fixtures (`local_agent`) and the apply/commit helpers.

use super::*;

// ── Team (30176) inbound ─────────────────────────────────────────────────

pub(super) const TEAM_ID: &str = "team-local-id";

pub(super) fn local_team() -> TeamRecord {
    TeamRecord {
        id: TEAM_ID.to_string(),
        name: "Local Team".to_string(),
        description: Some("local desc".to_string()),
        instructions: None,
        persona_ids: vec!["p-local".to_string()],
        is_builtin: false,
        source_dir: Some(std::path::PathBuf::from("/local/team/dir")),
        is_symlink: true,
        symlink_target: Some("/external".to_string()),
        version: Some("1.0".to_string()),
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    }
}

fn team_content(name: &str) -> TeamEventContent {
    TeamEventContent {
        name: name.to_string(),
        description: Some("remote desc".to_string()),
        instructions: Some(Some("remote instructions".to_string())),
        persona_ids: Some(vec!["p-remote-1".to_string(), "p-remote-2".to_string()]),
    }
}

/// An inbound event shaped like one from a client that predates
/// always-publish: `instructions`/`persona_ids` both omitted (`None`).
fn team_content_omitting_optional_fields(name: &str) -> TeamEventContent {
    TeamEventContent {
        name: name.to_string(),
        description: Some("remote desc".to_string()),
        instructions: None,
        persona_ids: None,
    }
}

/// An inbound event that explicitly clears both fields: `instructions` is
/// `Some(None)` (JSON `null`), `persona_ids` is `Some(vec![])`.
fn team_content_clearing_optional_fields(name: &str) -> TeamEventContent {
    TeamEventContent {
        name: name.to_string(),
        description: Some("remote desc".to_string()),
        instructions: Some(None),
        persona_ids: Some(vec![]),
    }
}

#[test]
fn inbound_team_match_patches_shared_preserves_local() {
    let mut teams = vec![local_team()];
    apply_inbound_team(
        &mut teams,
        TEAM_ID.to_string(),
        team_content("Renamed Team"),
    );

    assert_eq!(teams.len(), 1, "no duplicate row");
    let t = &teams[0];
    // Shared fields overwritten.
    assert_eq!(t.name, "Renamed Team");
    assert_eq!(t.description, Some("remote desc".to_string()));
    assert_eq!(t.instructions, Some("remote instructions".to_string()));
    assert_eq!(
        t.persona_ids,
        vec!["p-remote-1".to_string(), "p-remote-2".to_string()]
    );
    // Install-local fields preserved.
    assert_eq!(t.id, TEAM_ID);
    assert_eq!(
        t.source_dir,
        Some(std::path::PathBuf::from("/local/team/dir"))
    );
    assert!(t.is_symlink);
    assert_eq!(t.symlink_target, Some("/external".to_string()));
    assert_eq!(t.version, Some("1.0".to_string()));
    assert_eq!(t.created_at, "2025-01-01T00:00:00Z");
}

#[test]
fn inbound_team_omitted_fields_preserve_local() {
    // A `None` for instructions/persona_ids means the publisher predates
    // always-publish — its true value is unknown, so reconcile must
    // preserve whatever this device already has. This is the fix for the
    // Sietch Tabr wipe: an old-shaped (or genuinely field-omitting) event
    // must not blank out a team that has real membership/instructions.
    let mut teams = vec![local_team()];
    // Give local_team real instructions so preservation is discriminating:
    // the pre-fix blind-overwrite bug would collapse this to `None`, while
    // the fix must leave it untouched on an omitted field.
    teams[0].instructions = Some("local instructions".to_string());
    apply_inbound_team(
        &mut teams,
        TEAM_ID.to_string(),
        team_content_omitting_optional_fields("Renamed Team"),
    );

    assert_eq!(teams.len(), 1);
    let t = &teams[0];
    assert_eq!(
        t.name, "Renamed Team",
        "shared non-optional field still overwrites"
    );
    assert_eq!(
        t.instructions,
        Some("local instructions".to_string()),
        "omitted instructions preserves local value rather than wiping it"
    );
    assert_eq!(
        t.persona_ids,
        vec!["p-local".to_string()],
        "omitted persona_ids preserves local membership rather than wiping it"
    );
}

#[test]
fn inbound_team_explicit_clear_overwrites_local() {
    // `Some(None)` / `Some(vec![])` are the explicit-clear signals a
    // pre-fix client can never produce — these must still overwrite local.
    let mut teams = vec![local_team()];
    // Give local_team real instructions so the clear has something to erase.
    teams[0].instructions = Some("local instructions".to_string());

    apply_inbound_team(
        &mut teams,
        TEAM_ID.to_string(),
        team_content_clearing_optional_fields("Cleared Team"),
    );

    assert_eq!(teams.len(), 1);
    let t = &teams[0];
    assert_eq!(t.instructions, None, "explicit null clears instructions");
    assert_eq!(
        t.persona_ids,
        Vec::<String>::new(),
        "explicit empty array clears membership"
    );
}

#[test]
fn inbound_team_no_match_inserts_idempotently() {
    let mut teams = vec![local_team()];
    let other = "team-remote-id";
    apply_inbound_team(&mut teams, other.to_string(), team_content("New Team"));

    assert_eq!(teams.len(), 2, "unmatched inbound is inserted");
    let inserted = teams.iter().find(|t| t.id == other).unwrap();
    assert_eq!(inserted.name, "New Team");
    assert!(
        inserted.source_dir.is_none(),
        "inserted team has no local install dir"
    );
    // Re-receive stays idempotent.
    apply_inbound_team(&mut teams, other.to_string(), team_content("New Team"));
    assert_eq!(teams.len(), 2, "re-receive of inserted team no-ops");
}

// ── Inbound team → membership propagation (commit_inbound_team wiring) ─────

use std::cell::RefCell;

/// A running instance of `persona_id`, optionally bound to a team.
fn team_instance(seed: char, persona_id: &str, team_id: Option<&str>) -> ManagedAgentRecord {
    let mut record = local_agent();
    record.pubkey = seed.to_string().repeat(64);
    record.name = persona_id.to_string();
    record.persona_id = Some(persona_id.to_string());
    record.team_id = team_id.map(str::to_string);
    record
}

/// An inbound team edit that ADDS a persona must bind that persona's unbound
/// running instances to the team — exactly like a local `update_team`. Without
/// the propagation wiring the instance stays unbound (member in roster, not in
/// behavior) until restart.
#[test]
fn inbound_team_add_binds_unbound_instance_through_wiring() {
    let mut teams = vec![local_team()];
    teams[0].persona_ids = vec!["p-existing".to_string()];
    let existing = vec![
        team_instance('a', "p-added", None),
        team_instance('b', "p-existing", Some(TEAM_ID)),
    ];
    let saved = RefCell::new(None);

    commit_inbound_team(
        &mut teams,
        TEAM_ID.to_string(),
        TeamEventContent {
            name: "Team".to_string(),
            description: None,
            instructions: None,
            persona_ids: Some(vec!["p-existing".to_string(), "p-added".to_string()]),
        },
        |_| Ok(()),
        || Ok(existing.clone()),
        |records| {
            *saved.borrow_mut() = Some(records.to_vec());
            Ok(())
        },
    )
    .expect("inbound add succeeds");

    let saved = saved
        .borrow()
        .clone()
        .expect("add must save the agent store");
    assert_eq!(
        saved[0].team_id.as_deref(),
        Some(TEAM_ID),
        "the added persona's unbound instance is bound to the team"
    );
    assert_eq!(
        saved[1].team_id.as_deref(),
        Some(TEAM_ID),
        "an instance already on the team is untouched"
    );
}

/// An inbound team edit that REMOVES a persona ("keep agents") must detach that
/// persona's instances bound to this team, so a kept instance stops drawing the
/// team's instructions at spawn.
#[test]
fn inbound_team_removal_detaches_instance_through_wiring() {
    let mut teams = vec![local_team()];
    teams[0].persona_ids = vec!["p-removed".to_string()];
    let existing = vec![team_instance('a', "p-removed", Some(TEAM_ID))];
    let saved = RefCell::new(None);

    commit_inbound_team(
        &mut teams,
        TEAM_ID.to_string(),
        TeamEventContent {
            name: "Team".to_string(),
            description: None,
            instructions: None,
            persona_ids: Some(vec![]),
        },
        |_| Ok(()),
        || Ok(existing.clone()),
        |records| {
            *saved.borrow_mut() = Some(records.to_vec());
            Ok(())
        },
    )
    .expect("inbound removal succeeds");

    let saved = saved
        .borrow()
        .clone()
        .expect("removal must save the agent store");
    assert_eq!(
        saved[0].team_id, None,
        "the removed persona's instance is detached from the team"
    );
}

/// An inbound edit that omits `persona_ids` (a pre-always-publish client)
/// preserves local membership, so the delta is empty and no instance is
/// re-pointed — a metadata-only inbound edit must not disturb bindings.
#[test]
fn inbound_team_omitted_roster_leaves_bindings_untouched() {
    let mut teams = vec![local_team()];
    teams[0].persona_ids = vec!["p-a".to_string()];
    let existing = vec![team_instance('a', "p-a", None)];
    let saved = RefCell::new(None);

    commit_inbound_team(
        &mut teams,
        TEAM_ID.to_string(),
        team_content_omitting_optional_fields("Renamed"),
        |_| Ok(()),
        || Ok(existing.clone()),
        |records| {
            *saved.borrow_mut() = Some(records.to_vec());
            Ok(())
        },
    )
    .expect("inbound metadata-only edit succeeds");

    assert!(
        saved.borrow().is_none(),
        "an empty membership delta writes nothing to the agent store"
    );
}

/// A failing agent-store write after the authoritative `save_teams` is
/// swallowed: the inbound reconcile still succeeds (boot repair is the retry),
/// so a secondary-store hiccup never aborts an inbound event whose team write
/// already landed.
#[test]
fn inbound_team_swallows_agent_store_failure() {
    let mut teams = vec![local_team()];
    teams[0].persona_ids = vec![];
    commit_inbound_team(
        &mut teams,
        TEAM_ID.to_string(),
        TeamEventContent {
            name: "Team".to_string(),
            description: None,
            instructions: None,
            persona_ids: Some(vec!["p-added".to_string()]),
        },
        |_| Ok(()),
        || Err("agent store unreadable".to_string()),
        |_| Ok(()),
    )
    .expect("inbound reconcile swallows secondary-store failure");
}

/// A `persist_teams` error propagates — the authoritative team write failing is
/// a real reconcile failure, unlike best-effort agent IO.
#[test]
fn inbound_team_propagates_persist_teams_error() {
    let mut teams = vec![local_team()];
    let err = commit_inbound_team(
        &mut teams,
        TEAM_ID.to_string(),
        team_content("Team"),
        |_| Err("disk full".to_string()),
        || Ok(vec![]),
        |_| Ok(()),
    )
    .expect_err("a failed team persist must propagate");
    assert_eq!(err, "disk full");
}
