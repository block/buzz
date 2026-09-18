use super::super::{
    apply_team_membership_delta, commit_team_create, commit_team_update,
    detach_agents_outside_roster, load_pending_team_membership_at, pending_replay_delta,
    propagate_membership, save_pending_team_membership_at, PendingTeamMembershipUpdate,
};
use crate::managed_agents::{ManagedAgentRecord, TeamRecord};
use std::cell::RefCell;

/// A running instance: `pubkey` set, linked to a persona, optional binding.
fn instance(seed: char, persona_id: &str, team_id: Option<&str>) -> ManagedAgentRecord {
    let mut record = serde_json::from_value::<ManagedAgentRecord>(serde_json::json!({
        "pubkey": seed.to_string().repeat(64),
        "name": persona_id,
        "persona_id": persona_id,
        "relay_url": "ws://localhost:3000",
        "acp_command": "buzz-acp",
        "agent_command": "goose",
        "agent_args": [],
        "mcp_command": "",
        "turn_timeout_seconds": 320,
        "system_prompt": "prompt",
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
    }))
    .unwrap();
    record.team_id = team_id.map(str::to_string);
    record
}

fn instance_without_persona(seed: char, team_id: Option<&str>) -> ManagedAgentRecord {
    let mut record = instance(seed, "unassigned", team_id);
    record.persona_id = None;
    record
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// A metadata-only edit (no roster change) never re-points an instance —
/// including an unbound instance of a persona this team shares with another.
#[test]
fn metadata_only_edit_leaves_bindings_untouched() {
    let mut records = vec![instance('a', "duncan", None)];
    let roster = ids(&["duncan"]);
    assert!(!apply_team_membership_delta(
        &mut records,
        "team-a",
        &roster,
        &roster
    ));
    assert_eq!(records[0].team_id, None);
}

/// Only the *added* persona's unbound instance is bound; an untouched member
/// already present in the previous roster is not re-pointed.
#[test]
fn added_persona_backfills_only_its_unbound_instance() {
    let mut records = vec![
        instance('a', "duncan", None),
        instance('b', "paul", Some("team-b")),
    ];
    assert!(apply_team_membership_delta(
        &mut records,
        "team-a",
        &ids(&["paul"]),
        &ids(&["paul", "duncan"]),
    ));
    assert_eq!(records[0].team_id.as_deref(), Some("team-a"));
    // Paul was already on the team and bound elsewhere — untouched.
    assert_eq!(records[1].team_id.as_deref(), Some("team-b"));
}

/// An added persona binds even when shared across teams: an explicit add is
/// legitimate evidence (unlike the boot-repair's order-blind case).
#[test]
fn added_shared_persona_binds_to_the_edited_team() {
    let mut records = vec![instance('a', "duncan", None)];
    assert!(apply_team_membership_delta(
        &mut records,
        "team-a",
        &[],
        &ids(&["duncan"]),
    ));
    assert_eq!(records[0].team_id.as_deref(), Some("team-a"));
}

/// Removing a persona ("keep agents") clears its binding to *this* team so a
/// kept instance stops drawing the team's instructions at spawn.
#[test]
fn removed_persona_detaches_instance_bound_to_this_team() {
    let mut records = vec![instance('a', "duncan", Some("team-a"))];
    assert!(apply_team_membership_delta(
        &mut records,
        "team-a",
        &ids(&["duncan"]),
        &[],
    ));
    assert_eq!(records[0].team_id, None);
}

/// Removal only clears a binding pointing at *this* team — an instance of
/// the same persona bound to a different team is left alone.
#[test]
fn removed_persona_leaves_other_team_binding_untouched() {
    let mut records = vec![instance('a', "duncan", Some("team-b"))];
    assert!(!apply_team_membership_delta(
        &mut records,
        "team-a",
        &ids(&["duncan"]),
        &[],
    ));
    assert_eq!(records[0].team_id.as_deref(), Some("team-b"));
}

/// A minimal owner-authored team record for wiring tests.
fn team(id: &str, persona_ids: &[&str]) -> TeamRecord {
    TeamRecord {
        id: id.to_string(),
        name: id.to_string(),
        description: None,
        instructions: None,
        persona_ids: ids(persona_ids),
        is_builtin: false,
        shared: false,
        catalog_source: None,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

/// Records the injected store IO a commit performs, so a test can assert
/// the wiring saved (or deliberately did not) the agent store.
#[derive(Default)]
struct StoreSpy {
    saved: Option<Vec<ManagedAgentRecord>>,
}

/// Metadata-only `update_team` must pass the TRUE prior roster into the
/// delta, so an unchanged roster is an empty delta and no agent write fires.
/// The `&previous_persona_ids` → `&[]` miswire would drop the prior roster,
/// making the whole roster look "added" and re-pointing the unbound instance.
#[test]
fn commit_team_update_uses_true_prior_roster() {
    let mut teams = vec![team("team-a", &["duncan"])];
    let existing = vec![instance('a', "duncan", None)];
    let spy = RefCell::new(StoreSpy::default());

    let updated = commit_team_update(
        &mut teams,
        "team-a",
        "Team A".to_string(),
        None,
        Some("new instructions".to_string()),
        ids(&["duncan"]),
        "2026-02-02T00:00:00Z".to_string(),
        |_| Ok(()),
        || Ok(existing.clone()),
        |records| {
            spy.borrow_mut().saved = Some(records.to_vec());
            Ok(())
        },
    )
    .expect("metadata-only update succeeds");

    assert_eq!(updated.instructions.as_deref(), Some("new instructions"));
    // Empty delta ⇒ nothing changed ⇒ no save (the true-prior-roster gate).
    assert!(
        spy.borrow().saved.is_none(),
        "metadata-only edit must not write the agent store"
    );
}

/// A metadata-only update keeps its disk-authoritative result when the
/// agent store cannot load. Boot repair restores any missing backfill.
#[test]
fn commit_update_ignores_agent_load_failure_for_metadata_only_edit() {
    let mut teams = vec![team("team-a", &["duncan"])];

    let updated = commit_team_update(
        &mut teams,
        "team-a",
        "Renamed team".to_string(),
        None,
        None,
        ids(&["duncan"]),
        "2026-02-02T00:00:00Z".to_string(),
        |_| Ok(()),
        || Err("corrupt managed-agents.json".to_string()),
        |_| Ok(()),
    )
    .expect("metadata-only update keeps the best-effort policy");

    assert_eq!(updated.name, "Renamed team");
    assert_eq!(teams[0].name, "Renamed team");
}

/// An add-only update reports an agent-store load failure. The staged delta
/// remains available for replay on the next save or launch.
#[test]
fn commit_update_reports_agent_load_failure_for_add_only_edit() {
    let mut teams = vec![team("team-a", &["duncan"])];

    let error = commit_team_update(
        &mut teams,
        "team-a",
        "team-a".to_string(),
        None,
        None,
        ids(&["duncan", "ada"]),
        "2026-02-02T00:00:00Z".to_string(),
        |_| Ok(()),
        || Err("corrupt managed-agents.json".to_string()),
        |_| Ok(()),
    )
    .expect_err("an add-only update must report the lost binding");

    assert!(error.contains("could not update its agents"), "{error}");
    assert_eq!(teams[0].persona_ids, ids(&["duncan", "ada"]));
}

/// A roster removal remains strict when the agent store cannot load. The
/// command cannot prove that it cleared stale bindings in that case.
#[test]
fn commit_update_reports_agent_load_failure_for_removal() {
    let mut teams = vec![team("team-a", &["duncan"])];

    let err = commit_team_update(
        &mut teams,
        "team-a",
        "team-a".to_string(),
        None,
        None,
        ids(&[]),
        "2026-02-02T00:00:00Z".to_string(),
        |_| Ok(()),
        || Err("corrupt managed-agents.json".to_string()),
        |_| Ok(()),
    )
    .expect_err("a removal must report an agent-store load failure");

    assert!(err.contains("could not update its agents"), "{err}");
    assert!(teams[0].persona_ids.is_empty());
}

/// A stale binding makes an otherwise metadata-only update strict. The
/// command must report a failed detach because the delete guard still sees
/// the agent after the team write.
#[test]
fn commit_update_reports_agent_save_failure_for_stale_detach() {
    let mut teams = vec![team("team-a", &[])];

    let err = commit_team_update(
        &mut teams,
        "team-a",
        "Renamed team".to_string(),
        None,
        None,
        ids(&[]),
        "2026-02-02T00:00:00Z".to_string(),
        |_| Ok(()),
        || Ok(vec![instance('a', "duncan", Some("team-a"))]),
        |_| Err("disk full".to_string()),
    )
    .expect_err("a failed stale detach must not report success");

    assert!(err.contains("could not update its agents"), "{err}");
    assert_eq!(teams[0].name, "Renamed team");
}

/// Removing a persona from the roster must reach the detach branch through
/// the command wiring: the instance bound to this team is cleared and saved.
#[test]
fn commit_team_update_removal_detaches_through_wiring() {
    let mut teams = vec![team("team-a", &["duncan"])];
    let existing = vec![instance('a', "duncan", Some("team-a"))];
    let spy = RefCell::new(StoreSpy::default());

    commit_team_update(
        &mut teams,
        "team-a",
        "team-a".to_string(),
        None,
        None,
        ids(&[]),
        "2026-02-02T00:00:00Z".to_string(),
        |_| Ok(()),
        || Ok(existing.clone()),
        |records| {
            spy.borrow_mut().saved = Some(records.to_vec());
            Ok(())
        },
    )
    .expect("removal update succeeds");

    let saved = spy.borrow().saved.clone().expect("detach must save");
    assert_eq!(saved[0].team_id, None, "removed persona detaches from team");
}

/// `create_team` has no prior roster, so its whole roster is the added delta:
/// the unbound instance of a listed persona is bound through the wiring.
#[test]
fn commit_team_create_treats_full_roster_as_added() {
    let mut teams: Vec<TeamRecord> = Vec::new();
    let existing = vec![instance('a', "duncan", None)];
    let spy = RefCell::new(StoreSpy::default());

    let created = commit_team_create(
        &mut teams,
        team("team-a", &["duncan"]),
        |_| Ok(()),
        |_| Ok(()),
        || Ok(existing.clone()),
        |records| {
            spy.borrow_mut().saved = Some(records.to_vec());
            Ok(())
        },
        || Ok(()),
    )
    .expect("create succeeds");

    assert_eq!(created.id, "team-a");
    let saved = spy.borrow().saved.clone().expect("backfill must save");
    assert_eq!(
        saved[0].team_id.as_deref(),
        Some("team-a"),
        "whole roster is the added delta on create"
    );
}

/// A failed create keeps its staged delta. The retry path can bind a persona
/// even when another team also contains that persona.
#[test]
fn failed_create_keeps_a_durable_shared_persona_replay_delta() {
    let pending_file = tempfile::NamedTempFile::new().expect("temporary stage");
    std::fs::write(pending_file.path(), "null").expect("initialize stage");
    let mut teams = vec![team("team-a", &["duncan"])];
    let agents = RefCell::new(vec![instance('a', "duncan", None)]);

    let error = commit_team_create(
        &mut teams,
        team("team-b", &["duncan"]),
        |pending| save_pending_team_membership_at(pending_file.path(), Some(pending)),
        |_| Ok(()),
        || Ok(agents.borrow().clone()),
        |_| Err("disk full".to_string()),
        || save_pending_team_membership_at(pending_file.path(), None),
    )
    .expect_err("a create must report an undurable member binding");
    assert!(
        error.contains("could not update the new team's agents"),
        "{error}"
    );
    assert_eq!(
        teams.len(),
        2,
        "the team write landed before the failed binding"
    );

    let pending = load_pending_team_membership_at(pending_file.path())
        .expect("read staged delta")
        .expect("the failed create keeps its stage");
    assert_eq!(pending.team_id, "team-b");
    propagate_membership(
        &pending.team_id,
        &pending.previous_persona_ids,
        &pending.current_persona_ids,
        || Ok(agents.borrow().clone()),
        |records| {
            *agents.borrow_mut() = records.to_vec();
            Ok(())
        },
    )
    .expect("a launch replay binds the shared persona");
    save_pending_team_membership_at(pending_file.path(), None).expect("clear replayed stage");
    assert_eq!(agents.borrow()[0].team_id.as_deref(), Some("team-b"));
}

/// `update` must NOT swallow an agent-store failure while emptying a roster.
///
/// The removal clears `team_id` on the removed member. If that write fails
/// and the command reports success, the team is empty on disk but the agent
/// still points to it, so `delete_team_with_cascade` refuses the team. That
/// is the empty-and-undeletable state this feature exists to remove, so the
/// command must report the failure. The team write itself has landed, and an
/// update is idempotent, so a retry is safe.
#[test]
fn commit_update_reports_agent_save_failure_when_emptying_a_roster() {
    let mut teams = vec![team("team-a", &["duncan"])];
    let err = commit_team_update(
        &mut teams,
        "team-a",
        "team-a".to_string(),
        None,
        None,
        ids(&[]),
        "2026-02-02T00:00:00Z".to_string(),
        |_| Ok(()),
        || Ok(vec![instance('a', "duncan", Some("team-a"))]),
        |_| Err("disk full".to_string()),
    )
    .expect_err("an update must not report success when the detach is lost");

    assert!(err.contains("could not update its agents"), "{err}");
    assert!(err.contains("Save the team again"), "{err}");
    // The team write is authoritative and already landed.
    assert!(teams[0].persona_ids.is_empty());
}

/// A retry after a lost detach must repair the binding.
///
/// This is the recovery path of the test above. The team is already saved
/// empty, so the prior→current delta is empty and a delta-only pass would do
/// nothing. `detach_agents_outside_roster` reconciles against the current
/// roster instead, so saving the same empty roster again still clears the
/// stale `team_id` and makes the team deletable.
#[test]
fn resaving_an_already_empty_roster_repairs_a_lost_detach() {
    let mut teams = vec![team("team-a", &[])];
    let spy = RefCell::new(StoreSpy::default());

    commit_team_update(
        &mut teams,
        "team-a",
        "team-a".to_string(),
        None,
        None,
        ids(&[]),
        "2026-02-03T00:00:00Z".to_string(),
        |_| Ok(()),
        // The agent kept its binding because the earlier write was lost.
        || Ok(vec![instance('a', "duncan", Some("team-a"))]),
        |records| {
            spy.borrow_mut().saved = Some(records.to_vec());
            Ok(())
        },
    )
    .expect("the retry succeeds");

    let saved = spy
        .borrow()
        .saved
        .clone()
        .expect("the retry must write the agent store");
    assert_eq!(
        saved[0].team_id, None,
        "a stale binding is cleared against the current roster, not a delta"
    );
}

/// End to end at the command seam, measured by the real delete guard.
///
/// This test starts with a bound agent and empties the roster. It applies
/// `agents_referencing_team` — the predicate `delete_team_with_cascade` uses
/// — to the agent store that the command left behind. It pins both halves of
/// the contract:
///
/// 1. The failed save reports an error. It never claims success while the
///    delete guard still sees the agent.
/// 2. The retry succeeds, and the guard then sees no agent. Delete is
///    possible without an app restart.
#[test]
fn update_never_reports_success_while_the_delete_guard_sees_the_agent() {
    let mut teams = vec![team("team-a", &["duncan"])];
    // The store on disk. A failed save leaves it as it was.
    let store = RefCell::new(vec![instance('a', "duncan", Some("team-a"))]);

    // Attempt 1: the agent write fails.
    let err = commit_team_update(
        &mut teams,
        "team-a",
        "team-a".to_string(),
        None,
        None,
        ids(&[]),
        "2026-02-02T00:00:00Z".to_string(),
        |_| Ok(()),
        || Ok(store.borrow().clone()),
        |_| Err("disk full".to_string()),
    )
    .expect_err("the command must report the lost detach");
    assert!(err.contains("could not update its agents"), "{err}");

    // The team is empty on disk, but the guard still refuses deletion. The
    // command reported this state instead of hiding it.
    assert!(teams[0].persona_ids.is_empty());
    assert_eq!(
        crate::managed_agents::agents_referencing_team(&store.borrow(), &teams[0]),
        vec!["duncan"],
        "the guard still sees the agent, so the report was required"
    );

    // Attempt 2: the same save, and now the write lands.
    commit_team_update(
        &mut teams,
        "team-a",
        "team-a".to_string(),
        None,
        None,
        ids(&[]),
        "2026-02-03T00:00:00Z".to_string(),
        |_| Ok(()),
        || Ok(store.borrow().clone()),
        |records| {
            *store.borrow_mut() = records.to_vec();
            Ok(())
        },
    )
    .expect("the retry succeeds");

    assert!(
        crate::managed_agents::agents_referencing_team(&store.borrow(), &teams[0]).is_empty(),
        "the retry must make the team deletable"
    );
}

/// This test starts with a bound persona-less agent and empties the roster. It
/// applies `agents_referencing_team` — the predicate `delete_team_with_cascade`
/// uses — to the agent store that the update saved. The update must clear this
/// direct-command record because the delete guard does not require a persona.
#[test]
fn emptying_a_roster_detaches_a_bound_persona_less_agent() {
    let mut teams = vec![team("team-a", &["duncan"])];
    let store = RefCell::new(vec![instance_without_persona('a', Some("team-a"))]);

    commit_team_update(
        &mut teams,
        "team-a",
        "team-a".to_string(),
        None,
        None,
        ids(&[]),
        "2026-02-02T00:00:00Z".to_string(),
        |_| Ok(()),
        || Ok(store.borrow().clone()),
        |records| {
            *store.borrow_mut() = records.to_vec();
            Ok(())
        },
    )
    .expect("the update must detach a bound persona-less agent");

    assert!(
        crate::managed_agents::agents_referencing_team(&store.borrow(), &teams[0]).is_empty(),
        "the delete guard must not see the detached agent"
    );
}

/// The reconcile is scoped: it clears a binding to *this* team when the
/// persona is absent or unset, and it leaves a listed persona alone.
#[test]
fn detach_outside_roster_is_scoped_to_this_team_and_absent_personas() {
    let mut records = vec![
        instance('a', "duncan", Some("team-a")),
        instance('b', "paul", Some("team-b")),
        instance('c', "ada", Some("team-a")),
        instance_without_persona('d', Some("team-a")),
    ];

    assert!(detach_agents_outside_roster(
        &mut records,
        "team-a",
        &ids(&["ada"]),
    ));

    assert_eq!(records[0].team_id, None, "absent from this team's roster");
    assert_eq!(
        records[3].team_id, None,
        "an unset persona cannot remain bound to this team"
    );
    assert_eq!(
        records[1].team_id.as_deref(),
        Some("team-b"),
        "another team's binding is untouched"
    );
    assert_eq!(
        records[2].team_id.as_deref(),
        Some("team-a"),
        "still on the roster, so the binding stays"
    );
}

/// A replay keeps each staged direction whose evidence still matches the
/// current roster. An inbound extension or reorder does not discard a local
/// add. An inbound reversal does discard the obsolete direction.
#[test]
fn pending_replay_delta_merges_with_an_inbound_roster_change() {
    let pending = PendingTeamMembershipUpdate {
        team_id: "team-a".to_string(),
        previous_persona_ids: ids(&["duncan"]),
        current_persona_ids: ids(&["duncan", "ada"]),
    };

    assert_eq!(
        pending_replay_delta(&pending, &ids(&["ada", "duncan", "paul"])),
        (ids(&[]), ids(&["ada"])),
        "a reorder and an inbound extension preserve the staged add"
    );
    assert_eq!(
        pending_replay_delta(&pending, &ids(&["duncan"])),
        (ids(&[]), ids(&[])),
        "an inbound removal makes the staged add obsolete"
    );
}

/// Writing an empty pending state must preserve the app-facing symlink. On a
/// later launch the worktree sync can retain that link without reviving the old
/// canonical stage.
#[cfg(unix)]
#[test]
fn clearing_a_pending_stage_preserves_the_shared_symlink_on_relaunch() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let canonical = directory.path().join("canonical.json");
    let worktree = directory.path().join("worktree.json");
    std::fs::write(&canonical, "null").expect("create canonical pending file");
    std::os::unix::fs::symlink(&canonical, &worktree).expect("create shared link");
    let pending = PendingTeamMembershipUpdate {
        team_id: "team-a".to_string(),
        previous_persona_ids: ids(&["duncan"]),
        current_persona_ids: ids(&["ada"]),
    };

    save_pending_team_membership_at(&worktree, Some(&pending)).expect("stage through link");
    assert_eq!(
        load_pending_team_membership_at(&canonical).expect("read staged canonical file"),
        Some(pending.clone())
    );
    save_pending_team_membership_at(&worktree, None).expect("clear through link");

    assert!(worktree.is_symlink(), "the worktree path stays a symlink");
    assert_eq!(
        load_pending_team_membership_at(&worktree).expect("read after relaunch"),
        None,
        "the canonical file keeps the cleared state"
    );
}

/// The durable replay uses the original replace delta after the first agent
/// save fails. It binds Ada on retry even though the persisted roster already
/// contains Ada and therefore supplies no new delta.
#[test]
fn replayed_replace_delta_binds_the_added_instance() {
    let previous = ids(&["duncan"]);
    let current = ids(&["ada"]);
    let store = RefCell::new(vec![
        instance('a', "duncan", Some("team-a")),
        instance('b', "ada", None),
    ]);

    let error = propagate_membership(
        "team-a",
        &previous,
        &current,
        || Ok(store.borrow().clone()),
        |_| Err("disk full".to_string()),
    )
    .expect_err("the first save fails");
    assert!(error.to_string().contains("disk full"));

    propagate_membership(
        "team-a",
        &previous,
        &current,
        || Ok(store.borrow().clone()),
        |records| {
            *store.borrow_mut() = records.to_vec();
            Ok(())
        },
    )
    .expect("the durable replay succeeds");

    assert_eq!(store.borrow()[0].team_id, None);
    assert_eq!(store.borrow()[1].team_id.as_deref(), Some("team-a"));
}

/// A roster with every binding already correct writes nothing.
#[test]
fn detach_outside_roster_is_inert_when_nothing_is_stale() {
    let mut records = vec![instance('a', "duncan", Some("team-a"))];
    assert!(!detach_agents_outside_roster(
        &mut records,
        "team-a",
        &ids(&["duncan"]),
    ));
    assert_eq!(records[0].team_id.as_deref(), Some("team-a"));
}

/// A failed removal must replay before a new team accepts the same persona.
/// The replay clears the old binding, so the new team's explicit add becomes
/// the final binding instead of an older stage clearing it after the create.
#[test]
fn replay_before_create_preserves_the_new_team_binding() {
    let agents = RefCell::new(vec![instance('a', "duncan", Some("team-a"))]);

    propagate_membership(
        "team-a",
        &ids(&["duncan"]),
        &ids(&[]),
        || Ok(agents.borrow().clone()),
        |_| Err("disk full".to_string()),
    )
    .expect_err("the team-a removal remains staged after a failed save");

    propagate_membership(
        "team-a",
        &ids(&["duncan"]),
        &ids(&[]),
        || Ok(agents.borrow().clone()),
        |records| {
            *agents.borrow_mut() = records.to_vec();
            Ok(())
        },
    )
    .expect("create replays the earlier removal before its validation");

    let mut teams = Vec::new();
    commit_team_create(
        &mut teams,
        team("team-b", &["duncan"]),
        |_| Ok(()),
        |_| Ok(()),
        || Ok(agents.borrow().clone()),
        |records| {
            *agents.borrow_mut() = records.to_vec();
            Ok(())
        },
        || Ok(()),
    )
    .expect("the new team binds the now-unbound persona");

    assert_eq!(agents.borrow()[0].team_id.as_deref(), Some("team-b"));
}

/// A shared persona keeps the explicit local-add evidence after an inbound
/// extension and reorder. The latest roster still drives stale-binding cleanup.
#[test]
fn replay_after_inbound_extension_binds_the_explicit_shared_add() {
    let pending = PendingTeamMembershipUpdate {
        team_id: "team-a".to_string(),
        previous_persona_ids: ids(&[]),
        current_persona_ids: ids(&["ada"]),
    };
    let current_roster = ids(&["paul", "ada"]);
    let (previous, current) = pending_replay_delta(&pending, &current_roster);
    let agents = RefCell::new(vec![instance('a', "ada", None)]);

    propagate_membership(
        &pending.team_id,
        &previous,
        &current,
        || Ok(agents.borrow().clone()),
        |records| {
            *agents.borrow_mut() = records.to_vec();
            Ok(())
        },
    )
    .expect("the staged add binds the instance despite a shared persona");

    assert_eq!(agents.borrow()[0].team_id.as_deref(), Some("team-a"));
}
