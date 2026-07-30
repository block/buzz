use super::*;

/// The archive persistence path retains a closed ACP receipt as the exact
/// signed observer frame after the database is reopened.
#[test]
fn test_closed_turn_receipt_archives_exact_signed_frame() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db_path = tmp.path().join("archive.db");
    let owner_keys = Keys::generate();
    let agent_keys = Keys::generate();
    let owner_pk = owner_keys.public_key().to_hex();
    let agent_pk = agent_keys.public_key().to_hex();
    let relay_url = "wss://relay.example";
    let request_event_id = "11".repeat(32);
    let reply_event_id = "22".repeat(32);

    let conn = store::open_archive_db(&db_path).expect("open archive db");
    add_sub(&conn, &owner_pk, relay_url, "owner_p", &owner_pk, "[24200]");

    let receipt = serde_json::json!({
        "seq": 42,
        "timestamp": "2026-07-22T20:25:04Z",
        "kind": "turn_receipt",
        "agentIndex": 0,
        "channelId": "00000000-0000-4000-8000-000000000001",
        "sessionId": "00000000-0000-4000-8000-000000000002",
        "turnId": "5dc7e9cf-f30f-49bf-b7fc-e424afad2d3f",
        "startedAt": "2026-07-22T20:24:00Z",
        "payload": {
            "status": "closed",
            "schemaVersion": 1,
            "requestEventId": request_event_id.clone(),
            "replyEventId": reply_event_id,
            "replyAnchor": request_event_id,
            "gatewaySessionKey": "agent:main:buzz-private",
            "expectedGatewaySessionKey": "agent:main:buzz-private",
            "runId": "00000000-0000-4000-8000-000000000003",
            "acpSessionId": "00000000-0000-4000-8000-000000000002",
            "turnId": "5dc7e9cf-f30f-49bf-b7fc-e424afad2d3f"
        }
    });
    let encrypted = buzz_core_pkg::observer::encrypt_observer_payload(
        &agent_keys,
        &owner_keys.public_key(),
        &receipt,
    )
    .expect("encrypt receipt");
    let event = buzz_sdk_pkg::build_agent_observer_frame(
        &owner_pk,
        &agent_pk,
        OBSERVER_FRAME_TELEMETRY,
        &encrypted,
    )
    .expect("build receipt frame")
    .sign_with_keys(&agent_keys)
    .expect("sign receipt frame");
    let signed_json = event.as_json();

    let result = run_batch_sync_with_keys(
        vec![candidate(&event, ScopeType::OwnerP, &owner_pk)],
        &owner_pk,
        relay_url,
        &conn,
        Vec::new(),
        &owner_keys,
    );
    assert_eq!(result.persisted, 1);
    assert_eq!(result.dropped, 0);
    drop(conn);

    let read_conn = store::open_archive_db(&db_path).expect("reopen archive db");
    let (kind, author, raw_json): (i64, String, String) = read_conn
        .query_row(
            "SELECT kind, pubkey, raw_json FROM archived_events
             WHERE identity_pubkey = ?1 AND relay_url = ?2 AND id = ?3",
            rusqlite::params![owner_pk, relay_url, event.id.to_hex()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read archived receipt");
    assert_eq!(kind, 24200);
    assert_eq!(author, agent_pk);
    assert_eq!(
        raw_json, signed_json,
        "archive must retain the signed frame"
    );
    let scope_count: i64 = read_conn
        .query_row(
            "SELECT COUNT(*) FROM archived_event_scopes
             WHERE identity_pubkey = ?1 AND relay_url = ?2 AND id = ?3
               AND scope_type = 'owner_p' AND scope_value = ?1",
            rusqlite::params![owner_pk, relay_url, event.id.to_hex()],
            |row| row.get(0),
        )
        .expect("read archived receipt scope");
    assert_eq!(scope_count, 1, "receipt must retain its owner_p scope");

    let stored_event = Event::from_json(&raw_json).expect("parse archived frame");
    buzz_core_pkg::verify_event(&stored_event).expect("verify archived signature");
    assert_eq!(stored_event.id, event.id);
    let decoded: serde_json::Value =
        buzz_core_pkg::observer::decrypt_observer_payload(&owner_keys, &stored_event)
            .expect("decrypt archived receipt");
    assert_eq!(decoded, receipt);
    assert_eq!(decoded["payload"]["status"], "closed");
}
