use super::*;
use crate::managed_agents::persona_events::monotonic_created_at;
use crate::managed_agents::retention::{
    get_pending_sync, get_retained_event, open_retention_db, retain_event,
    scoped_retention_db_path, tombstone_retention_d_tag, RetainedEvent,
};
use crate::managed_agents::team_events::build_team_event;
use buzz_core_pkg::kind::KIND_TEAM;
use nostr::JsonUtil;
use std::path::{Path, PathBuf};

fn team() -> TeamRecord {
    TeamRecord {
        id: "team-abc".to_string(),
        name: "Catalog Team".to_string(),
        description: Some("A shared team".to_string()),
        instructions: None,
        persona_ids: vec!["m1".to_string()],
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

fn scoped_db(dir: &Path, relay_url: &str, owner: &str) -> PathBuf {
    let db_path = scoped_retention_db_path(dir, relay_url, owner);
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    db_path
}

/// Seed a retained 30176 head dated `created_at` seconds since epoch.
fn seed_team_head(db_path: &Path, keys: &nostr::Keys, created_at: i64) {
    let event = build_team_event(&team())
        .unwrap()
        .custom_created_at(nostr::Timestamp::from(created_at as u64))
        .sign_with_keys(keys)
        .unwrap();
    let conn = open_retention_db(db_path).unwrap();
    retain_event(
        &conn,
        &RetainedEvent {
            kind: KIND_TEAM,
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

#[test]
fn test_team_tombstone_created_at_strictly_dominates_a_future_dated_head() {
    // 30176 analog of the 30178 defect (Wes P1): retain_team_pending signs the
    // team head with monotonic_created_at, so it can be future-dated. The kind:5
    // must dominate it or the relay's created_at <= gate leaves the head live.
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    let future = nostr::Timestamp::now().as_secs() as i64 + 86_400;
    seed_team_head(&db_path, &keys, future);

    tombstone_team_at(&db_path, &keys, "team-abc").unwrap();

    let conn = open_retention_db(&db_path).unwrap();
    assert!(
        get_retained_event(&conn, KIND_TEAM, &owner, "team-abc")
            .unwrap()
            .is_none(),
        "the 30176 head is purged"
    );
    let tombstone = get_pending_sync(&conn)
        .unwrap()
        .into_iter()
        .find(|row| row.kind == 5)
        .expect("a kind:5 tombstone is enqueued");
    assert_eq!(
        tombstone.d_tag,
        tombstone_retention_d_tag(KIND_TEAM, "team-abc")
    );
    assert!(
        tombstone.created_at > future,
        "tombstone created_at ({}) must strictly dominate the future-dated 30176 head ({future})",
        tombstone.created_at
    );
}

#[test]
fn test_team_tombstone_with_no_head_falls_back_to_wall_clock() {
    let dir = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    let owner = keys.public_key().to_hex();
    let db_path = scoped_db(dir.path(), "wss://a.example", &owner);

    let before = nostr::Timestamp::now().as_secs() as i64;
    tombstone_team_at(&db_path, &keys, "team-abc").unwrap();
    let after = nostr::Timestamp::now().as_secs() as i64;

    let conn = open_retention_db(&db_path).unwrap();
    let tombstone = get_pending_sync(&conn)
        .unwrap()
        .into_iter()
        .find(|row| row.kind == 5)
        .expect("a kind:5 tombstone is enqueued even with no head");
    assert!(
        tombstone.created_at >= before && tombstone.created_at <= after,
        "no-head 30176 tombstone is dated at wall clock; got {}",
        tombstone.created_at
    );
    // Sanity: with no head, the floor is 0 so the result is exactly `now`.
    assert!(monotonic_created_at(None).as_secs() as i64 >= before);
}

/// Membership-propagation wiring (#5904). Nested to keep its `team`/`instance`
/// helpers isolated from this file's catalog-oriented `team()` fixture.
mod membership_wiring {
    use super::super::{apply_team_membership_delta, commit_team_create, commit_team_update};
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
            || Ok(existing.clone()),
            |records| {
                spy.borrow_mut().saved = Some(records.to_vec());
                Ok(())
            },
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

    /// A failing secondary agent write after successful `save_teams` is
    /// swallowed: both commits still return the persisted team. Otherwise a UI
    /// retry of a create whose team already landed would mint a duplicate.
    #[test]
    fn commit_returns_ok_when_agent_save_fails() {
        let mut teams: Vec<TeamRecord> = Vec::new();
        let created = commit_team_create(
            &mut teams,
            team("team-a", &["duncan"]),
            |_| Ok(()),
            || Ok(vec![instance('a', "duncan", None)]),
            |_| Err("disk full".to_string()),
        )
        .expect("create swallows secondary-store failure");
        assert_eq!(created.id, "team-a");

        let mut teams = vec![team("team-a", &["duncan"])];
        let updated = commit_team_update(
            &mut teams,
            "team-a",
            "team-a".to_string(),
            None,
            None,
            ids(&[]),
            "2026-02-02T00:00:00Z".to_string(),
            |_| Ok(()),
            || Err("agent store unreadable".to_string()),
            |_| Ok(()),
        )
        .expect("update swallows secondary-store failure");
        assert_eq!(updated.persona_ids, Vec::<String>::new());
    }
}
