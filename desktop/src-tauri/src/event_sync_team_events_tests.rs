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
