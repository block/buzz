use super::mod_tests::{add_sub, candidate, in_memory, run_batch_sync_with_keys};
use super::*;
use nostr::Keys;

fn make_command_brief_event(owner_keys: &Keys) -> nostr::Event {
    use buzz_core_pkg::command_brief::{
        build_command_brief_event, CommandBriefEventPayload, CommandBriefLifecycleState,
        COMMAND_BRIEF_PAYLOAD_VERSION,
    };
    build_command_brief_event(
        owner_keys,
        &CommandBriefEventPayload {
            version: COMMAND_BRIEF_PAYLOAD_VERSION,
            classification: "OFFICIAL".into(),
            run_id: "run-1".into(),
            schedule_id: "daily".into(),
            lifecycle_state: CommandBriefLifecycleState::Completed,
            occurred_at: "2026-07-25T06:00:00Z".into(),
            frozen_snapshot_id: "snapshot-1".into(),
            final_brief: Some(
                buzz_core_pkg::command_brief::CommandBriefWire::try_from(
                    super::super::command_brief::types_tests::brief_value(),
                )
                .expect("strict brief"),
            ),
            failure: None,
            previous_lifecycle_event_id: None,
        },
    )
    .expect("command brief event")
}

#[test]
fn owner_p_44210_routes_persistent_and_decrypts_only_for_owner() {
    let conn = in_memory();
    let owner = Keys::generate();
    let owner_pk = owner.public_key().to_hex();
    let relay_url = "wss://relay.example";
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[44210]");
    let event = make_command_brief_event(&owner);
    let plan = plan_archive(
        vec![candidate(&event, ScopeType::OwnerP, &owner_pk)],
        &owner_pk,
        relay_url,
        &conn,
    )
    .expect("archive plan");
    assert_eq!(plan.buckets.len(), 1);
    assert!(plan.ephemeral.is_empty());
    let result = run_batch_sync_with_keys(
        vec![candidate(&event, ScopeType::OwnerP, &owner_pk)],
        &owner_pk,
        relay_url,
        &conn,
        vec![event.clone()],
        &owner,
    );
    assert_eq!(result.persisted, 1);
    let plaintext: String = conn
        .query_row("SELECT raw_json FROM archived_events", [], |row| row.get(0))
        .expect("archived payload");
    assert!(plaintext.contains("\"classification\":\"OFFICIAL\""));

    let wrong_conn = in_memory();
    add_sub(
        &wrong_conn,
        &owner_pk,
        relay_url,
        "owner_p",
        &owner_pk,
        "[44210]",
    );
    let denied = run_batch_sync_with_keys(
        vec![candidate(&event, ScopeType::OwnerP, &owner_pk)],
        &owner_pk,
        relay_url,
        &wrong_conn,
        vec![event],
        &Keys::generate(),
    );
    assert_eq!(denied.persisted, 0);
    assert_eq!(denied.dropped, 1);
}
