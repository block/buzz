use super::*;

/// Helper: write a `teams.json` directly in `base_dir` (the migration reads
/// `base_dir/teams.json`, where `base_dir` is the `agents` dir).
fn write_base_teams(base_dir: &Path, records: &serde_json::Value) {
    std::fs::write(
        base_dir.join("teams.json"),
        serde_json::to_string_pretty(records).unwrap(),
    )
    .unwrap();
}

fn one_team() -> serde_json::Value {
    serde_json::json!([{
        "id": "team-alpha",
        "name": "Alpha",
        "description": "The alpha team",
        "persona_ids": ["code-reviewer"],
        "is_builtin": false,
        "created_at": "2025-01-01T00:00:00Z",
        "updated_at": "2025-01-01T00:00:00Z"
    }])
}

#[test]
fn migrate_teams_writes_signed_retention_rows() {
    use crate::managed_agents::retention::{get_retained_event, open_retention_db};
    use buzz_core_pkg::kind::KIND_TEAM;

    let base = tempfile::tempdir().unwrap();
    write_base_teams(base.path(), &one_team());
    let keys = nostr::Keys::generate();
    let pubkey = keys.public_key().to_hex();

    assert_eq!(migrate_teams_in_dir(base.path(), &keys).unwrap(), 1);

    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    let row = get_retained_event(&conn, KIND_TEAM, &pubkey, "team-alpha")
        .unwrap()
        .unwrap();
    let event: nostr::Event = nostr::JsonUtil::from_json(&row.raw_event).unwrap();
    assert!(event.verify().is_ok());
    assert!(row.pending_sync);
    assert!(row.content.contains("Alpha"));
}

#[test]
fn migrate_teams_skips_builtins() {
    use crate::managed_agents::retention::{get_retained_event, open_retention_db};
    use buzz_core_pkg::kind::KIND_TEAM;

    let base = tempfile::tempdir().unwrap();
    write_base_teams(
        base.path(),
        &serde_json::json!([{
            "id": "builtin-team",
            "name": "Builtin",
            "persona_ids": [],
            "is_builtin": true,
            "created_at": "2025-01-01T00:00:00Z",
            "updated_at": "2025-01-01T00:00:00Z"
        }]),
    );
    let keys = nostr::Keys::generate();

    assert_eq!(migrate_teams_in_dir(base.path(), &keys).unwrap(), 0);

    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    assert!(get_retained_event(
        &conn,
        KIND_TEAM,
        &keys.public_key().to_hex(),
        "builtin-team"
    )
    .unwrap()
    .is_none());
}

#[test]
fn migrate_teams_unchanged_second_run_is_noop() {
    let base = tempfile::tempdir().unwrap();
    write_base_teams(base.path(), &one_team());
    let keys = nostr::Keys::generate();

    assert_eq!(migrate_teams_in_dir(base.path(), &keys).unwrap(), 1);
    assert_eq!(migrate_teams_in_dir(base.path(), &keys).unwrap(), 0);
}

#[test]
fn migrate_teams_edited_team_re_retains_pending() {
    use crate::managed_agents::retention::{get_retained_event, mark_synced, open_retention_db};
    use buzz_core_pkg::kind::KIND_TEAM;

    let base = tempfile::tempdir().unwrap();
    write_base_teams(base.path(), &one_team());
    let keys = nostr::Keys::generate();
    let pubkey = keys.public_key().to_hex();

    assert_eq!(migrate_teams_in_dir(base.path(), &keys).unwrap(), 1);

    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    let row = get_retained_event(&conn, KIND_TEAM, &pubkey, "team-alpha")
        .unwrap()
        .unwrap();
    mark_synced(
        &conn,
        KIND_TEAM,
        &pubkey,
        "team-alpha",
        row.created_at,
        &row.content,
    )
    .unwrap();
    drop(conn);

    let mut edited = one_team();
    edited.as_array_mut().unwrap()[0]["description"] = serde_json::json!("Renamed team");
    write_base_teams(base.path(), &edited);

    assert_eq!(migrate_teams_in_dir(base.path(), &keys).unwrap(), 1);

    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    let row = get_retained_event(&conn, KIND_TEAM, &pubkey, "team-alpha")
        .unwrap()
        .unwrap();
    assert!(row.pending_sync);
    assert!(row.content.contains("Renamed team"));
}

#[test]
fn migrate_teams_no_file_is_noop() {
    let base = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    assert_eq!(migrate_teams_in_dir(base.path(), &keys).unwrap(), 0);
}

/// Error-contract for the fatal team leg. `run_event_sync` propagates a team
/// leg failure via `?`, and `apply_workspace` returns that `Err` so the
/// frontend never exposes the community against an un-superseded disk state.
/// This proves the leg genuinely surfaces failure (rather than logging and
/// swallowing) on an unreadable store — the precondition that made the
/// propagation load-bearing.
#[test]
fn migrate_teams_surfaces_error_on_unparseable_store() {
    let base = tempfile::tempdir().unwrap();
    std::fs::write(base.path().join("teams.json"), "{ not valid json").unwrap();
    let keys = nostr::Keys::generate();
    assert!(migrate_teams_in_dir(base.path(), &keys).is_err());
}

/// Build a signed inbound team head at an explicit `created_at`, mirroring a
/// relay replay of a stale, pre-namespacing roster.
fn stale_inbound_head(
    keys: &nostr::Keys,
    id: &str,
    bare_persona_ids: &[&str],
    created_at: i64,
) -> crate::managed_agents::retention::RetainedEvent {
    use crate::managed_agents::{team_events::build_team_event, TeamRecord};
    use buzz_core_pkg::kind::KIND_TEAM;
    use nostr::JsonUtil;

    let record = TeamRecord {
        id: id.to_string(),
        name: "Sietch Tabr".to_string(),
        description: None,
        instructions: None,
        persona_ids: bare_persona_ids.iter().map(|s| s.to_string()).collect(),
        is_builtin: false,
        shared: false,
        catalog_source: None,
        source_dir: None,
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    };
    let event = build_team_event(&record)
        .unwrap()
        .custom_created_at(nostr::Timestamp::from(created_at as u64))
        .sign_with_keys(keys)
        .unwrap();
    crate::managed_agents::retention::RetainedEvent {
        kind: KIND_TEAM,
        pubkey: keys.public_key().to_hex(),
        d_tag: id.to_string(),
        content: event.content.to_string(),
        created_at: event.created_at.as_secs() as i64,
        raw_event: event.as_json(),
        pending_sync: false,
    }
}

/// Finding-1 retention-precedence guarantee. This proves the *mechanic* the
/// awaited-reconcile ordering relies on — it does not itself exercise
/// `apply_workspace` (an `AppHandle`-level path). Given the boot reconcile has
/// retained the repaired namespaced roster with a monotonic `created_at`
/// (reconcile-first), a stale relay head replayed afterward is older, so
/// `retain_inbound_event` skips it and the repaired roster stays. The
/// inbound-first lane is the counterfactual the ordering closes: with no
/// repaired head retained yet, the very same stale head is applied and restores
/// bare membership. Retention order is the only difference between the lanes;
/// `apply_workspace` awaiting the reconcile (see `commands/workspace.rs`) is
/// what forces the reconcile-first order in production.
#[test]
fn reconcile_first_makes_stale_inbound_team_head_lose() {
    use crate::managed_agents::retention::{
        get_retained_event, open_retention_db, retain_inbound_event, InboundOutcome,
    };
    use buzz_core_pkg::kind::KIND_TEAM;

    let keys = nostr::Keys::generate();
    let pubkey = keys.public_key().to_hex();
    let repaired = serde_json::json!([{
        "id": "sietch-tabr",
        "name": "Sietch Tabr",
        "persona_ids": ["sietch-tabr:thufir", "sietch-tabr:paul", "sietch-tabr:duncan"],
        "is_builtin": false,
        "created_at": "2025-01-01T00:00:00Z",
        "updated_at": "2025-01-01T00:00:00Z"
    }]);
    let bare = ["thufir", "paul", "duncan"];

    // Reconcile-first lane (the fix): the awaited boot reconcile retains the
    // repaired namespaced roster with a monotonic `created_at`; a stale relay
    // head replayed afterward is older, so `retain_inbound_event` skips it and
    // the retained roster stays repaired.
    let ordered = tempfile::tempdir().unwrap();
    let ordered_db = ordered.path().join("retention.db");
    write_base_teams(ordered.path(), &repaired);
    assert_eq!(
        migrate_teams_in_dir_at(ordered.path(), &keys, &ordered_db).unwrap(),
        1
    );
    let conn = open_retention_db(&ordered_db).unwrap();
    let repaired_head = get_retained_event(&conn, KIND_TEAM, &pubkey, "sietch-tabr")
        .unwrap()
        .unwrap();
    let stale = stale_inbound_head(&keys, "sietch-tabr", &bare, repaired_head.created_at - 1);
    assert_eq!(
        retain_inbound_event(&conn, &stale).unwrap(),
        InboundOutcome::Skipped
    );
    let head = get_retained_event(&conn, KIND_TEAM, &pubkey, "sietch-tabr")
        .unwrap()
        .unwrap();
    assert!(head.content.contains("sietch-tabr:thufir"));
    assert!(!head.content.contains("\"thufir\""));

    // Inbound-first lane (the race the fix closes): with no repaired head
    // retained yet, the very same stale relay head is applied, restoring the
    // bare pre-namespacing roster. Ordering is the only difference.
    let raced = tempfile::tempdir().unwrap();
    let raced_db = raced.path().join("retention.db");
    let raced_conn = open_retention_db(&raced_db).unwrap();
    let stale = stale_inbound_head(&keys, "sietch-tabr", &bare, repaired_head.created_at - 1);
    assert_eq!(
        retain_inbound_event(&raced_conn, &stale).unwrap(),
        InboundOutcome::Applied
    );
    let head = get_retained_event(&raced_conn, KIND_TEAM, &pubkey, "sietch-tabr")
        .unwrap()
        .unwrap();
    assert!(head.content.contains("\"thufir\""));
}
