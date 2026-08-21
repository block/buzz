use super::*;
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
use std::path::PathBuf;
use tempfile::TempDir;

fn write_activity_snapshot(
    dir: &TempDir,
    capability: &str,
    generated_at: u64,
    expires_at: u64,
) -> PathBuf {
    let path = dir.path().join("activity-ledger-today.json");
    let owner_secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let owner_keys = Keys::parse(owner_secret).unwrap();
    let owner_pubkey = owner_keys.public_key().to_hex();
    let unsigned_snapshot = json!({
        "schema": ACTIVITY_LEDGER_TODAY_SCHEMA,
        "ownerPubkey": owner_pubkey,
        "generatedAt": generated_at,
        "expiresAt": expires_at,
        "capability": capability,
        "surface": {
            "day": "2026-08-21",
            "journals": [
                {
                    "id": "journal-a",
                    "channelId": "chan-a",
                    "agentPubkey": "agent-a",
                    "agentName": "Honey",
                    "status": "completed",
                    "proofState": "RECEIPTED",
                    "endedAt": "2026-08-21T14:00:00.000Z",
                    "claimedCompletionWithoutEvidence": false,
                    "events": [
                        { "id": "event-a", "detail": "receipted activity" }
                    ]
                },
                {
                    "id": "journal-b",
                    "channelId": "chan-b",
                    "agentPubkey": "agent-b",
                    "agentName": "Fizz",
                    "status": "failed",
                    "proofState": "FAILED",
                    "endedAt": "2026-08-21T15:00:00.000Z",
                    "claimedCompletionWithoutEvidence": true,
                    "events": [
                        { "id": "event-b", "detail": "failed activity" }
                    ]
                }
            ],
            "snapshotProjection": {
                "bounded": false,
                "maxBytes": 6291456,
                "originalJournals": 2,
                "includedJournals": 2,
                "omittedJournals": 0,
                "omittedEvents": 0,
                "textFieldsTruncated": 0
            }
        },
        "rawEvents": []
    });
    let canonical_payload = canonical_activity_ledger_snapshot_payload_json(
        unsigned_snapshot.as_object().unwrap(),
        &owner_pubkey,
        generated_at,
        expires_at,
    )
    .unwrap();
    let snapshot_sha256 = hex::encode(sha2::Sha256::digest(canonical_payload.as_bytes()));
    let event = EventBuilder::new(
        Kind::Custom(ACTIVITY_LEDGER_TODAY_SIGNED_KIND),
        canonical_payload,
    )
    .tags([
        Tag::parse(["t", ACTIVITY_LEDGER_TODAY_SIGNED_TAG_MARKER]).unwrap(),
        Tag::parse(["schema", ACTIVITY_LEDGER_TODAY_SCHEMA]).unwrap(),
        Tag::parse(["capability", capability]).unwrap(),
        Tag::parse(["snapshot_sha256", &snapshot_sha256]).unwrap(),
        Tag::parse(["expires_at", &expires_at.to_string()]).unwrap(),
    ])
    .custom_created_at(Timestamp::from(generated_at))
    .sign_with_keys(&owner_keys)
    .unwrap();
    let signed_snapshot = json!({
        "schema": ACTIVITY_LEDGER_TODAY_SCHEMA,
        "ownerPubkey": owner_pubkey,
        "generatedAt": generated_at,
        "expiresAt": expires_at,
        "capability": capability,
        "surface": unsigned_snapshot["surface"].clone(),
        "rawEvents": [],
        "snapshotSha256": snapshot_sha256,
        "eventId": event.id.to_hex(),
        "signature": event.sig.to_string(),
    });
    std::fs::write(&path, serde_json::to_vec(&signed_snapshot).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    path
}

fn write_custom_activity_snapshot(dir: &TempDir, snapshot: Value) -> PathBuf {
    let path = dir.path().join("activity-ledger-today.json");
    std::fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    path
}

fn expected_owner_pubkey() -> String {
    Keys::parse("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        .unwrap()
        .public_key()
        .to_hex()
}

#[test]
fn desktop_canonical_payload_fixture_matches_honey_reconstruction() {
    let fixture = include_str!("../../../test-fixtures/activity-ledger-today-desktop.json").trim();
    let root: Value = serde_json::from_str(fixture).unwrap();
    let object = root.as_object().unwrap();
    let reconstructed = canonical_activity_ledger_snapshot_payload_json(
        object,
        object["ownerPubkey"].as_str().unwrap(),
        object["generatedAt"].as_u64().unwrap(),
        object["expiresAt"].as_u64().unwrap(),
    )
    .unwrap();
    assert_eq!(reconstructed, fixture);
}

#[test]
fn activity_ledger_today_filters_and_strips_events_by_default() {
    let tmp = TempDir::new().unwrap();
    let capability = ACTIVITY_LEDGER_TODAY_CAPABILITY_VALUE;
    let path = write_activity_snapshot(&tmp, capability, 100, 160);

    let output = read_activity_ledger_today(
        path.to_str().unwrap(),
        capability,
        &expected_owner_pubkey(),
        &json!({"agentPubkey": "agent-a", "limit": 10}),
        120,
    )
    .unwrap();
    let result: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(result["counts"]["matchingJournals"], 1);
    assert_eq!(result["counts"]["returnedJournals"], 1);
    assert_eq!(result["journals"][0]["id"], "journal-a");
    assert!(result["journals"][0].get("events").is_none());
    assert_eq!(result["channels"][0]["channelId"], "chan-a");
}

#[test]
fn activity_ledger_today_includes_events_when_requested() {
    let tmp = TempDir::new().unwrap();
    let capability = ACTIVITY_LEDGER_TODAY_CAPABILITY_VALUE;
    let path = write_activity_snapshot(&tmp, capability, 100, 160);

    let output = read_activity_ledger_today(
        path.to_str().unwrap(),
        capability,
        &expected_owner_pubkey(),
        &json!({"channelId": "chan-b", "includeEvents": true, "limit": 10}),
        120,
    )
    .unwrap();
    let result: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(result["journals"][0]["id"], "journal-b");
    assert_eq!(result["journals"][0]["events"][0]["id"], "event-b");
    assert_eq!(result["counts"]["failed"], 1);
    assert_eq!(result["counts"]["claimedWithoutEvidence"], 1);
}

#[test]
fn activity_ledger_today_limit_keeps_newest_matching_journals() {
    let tmp = TempDir::new().unwrap();
    let capability = ACTIVITY_LEDGER_TODAY_CAPABILITY_VALUE;
    let path = write_activity_snapshot(&tmp, capability, 100, 160);

    let output = read_activity_ledger_today(
        path.to_str().unwrap(),
        capability,
        &expected_owner_pubkey(),
        &json!({"limit": 1}),
        120,
    )
    .unwrap();
    let result: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(result["counts"]["matchingJournals"], 2);
    assert_eq!(result["counts"]["returnedJournals"], 1);
    assert_eq!(result["journals"][0]["id"], "journal-b");
    assert_eq!(result["channels"][0]["channelId"], "chan-b");
    assert_eq!(result["truncated"], true);
    assert_eq!(result["sourceProjection"]["bounded"], false);
}

#[test]
fn activity_ledger_today_result_obeys_the_model_text_budget() {
    let result = success_result("x".repeat(4_096), 512);
    let ToolResultContent::Text(text) = &result.content[0] else {
        panic!("Today result must stay text-only");
    };
    assert!(text.len() <= 512);
    assert!(text.contains("bytes elided from tool result"));
}

#[test]
fn activity_ledger_today_rejects_relative_path() {
    let error = read_activity_ledger_today(
        "relative.json",
        ACTIVITY_LEDGER_TODAY_CAPABILITY_VALUE,
        &expected_owner_pubkey(),
        &json!({}),
        120,
    )
    .unwrap_err();
    assert!(
        error.contains("snapshot path must be absolute"),
        "got: {error}"
    );
}

#[test]
fn activity_ledger_today_rejects_oversized_snapshot_before_reading() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("oversized.json");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(ACTIVITY_LEDGER_MAX_SNAPSHOT_BYTES + 1)
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let error = read_activity_ledger_today(
        path.to_str().unwrap(),
        ACTIVITY_LEDGER_TODAY_CAPABILITY_VALUE,
        &expected_owner_pubkey(),
        &json!({}),
        120,
    )
    .unwrap_err();
    assert!(error.contains("snapshot exceeds"), "got: {error}");
}

#[test]
fn activity_ledger_today_rejects_capability_mismatch_and_staleness() {
    let tmp = TempDir::new().unwrap();
    let capability = ACTIVITY_LEDGER_TODAY_CAPABILITY_VALUE;
    let path = write_activity_snapshot(&tmp, "wrong-capability", 100, 160);

    let capability_error = read_activity_ledger_today(
        path.to_str().unwrap(),
        capability,
        &expected_owner_pubkey(),
        &json!({}),
        120,
    )
    .unwrap_err();
    assert!(
        capability_error.contains("capability mismatch"),
        "got: {capability_error}"
    );

    let stale_path = write_activity_snapshot(&tmp, capability, 100, 110);
    let stale_error = read_activity_ledger_today(
        stale_path.to_str().unwrap(),
        capability,
        &expected_owner_pubkey(),
        &json!({}),
        120,
    )
    .unwrap_err();
    assert!(
        stale_error.contains("snapshot expired"),
        "got: {stale_error}"
    );
}

#[test]
fn activity_ledger_today_rejects_future_generated_at_and_uppercase_owner() {
    let tmp = TempDir::new().unwrap();
    let capability = ACTIVITY_LEDGER_TODAY_CAPABILITY_VALUE;

    let future_path = write_activity_snapshot(&tmp, capability, 500, 560);
    let future_error = read_activity_ledger_today(
        future_path.to_str().unwrap(),
        capability,
        &expected_owner_pubkey(),
        &json!({}),
        120,
    )
    .unwrap_err();
    assert!(
        future_error.contains("generatedAt is more than 300 seconds in the future"),
        "got: {future_error}"
    );

    let uppercase_owner_path = write_custom_activity_snapshot(
        &tmp,
        json!({
            "schema": ACTIVITY_LEDGER_TODAY_SCHEMA,
            "ownerPubkey": "ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD",
            "generatedAt": 100,
            "expiresAt": 160,
            "capability": capability,
            "surface": {
                "day": "2026-08-21",
                "journals": []
            },
            "rawEvents": [],
            "snapshotSha256": "a".repeat(64),
            "eventId": "b".repeat(64),
            "signature": "c".repeat(128),
        }),
    );
    let owner_error = read_activity_ledger_today(
        uppercase_owner_path.to_str().unwrap(),
        capability,
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        &json!({}),
        120,
    )
    .unwrap_err();
    assert!(
        owner_error.contains("ownerPubkey must be 64 lowercase hex chars"),
        "got: {owner_error}"
    );
}

#[cfg(unix)]
#[test]
fn activity_ledger_today_rejects_symlink_and_non_0600_mode() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let tmp = TempDir::new().unwrap();
    let capability = ACTIVITY_LEDGER_TODAY_CAPABILITY_VALUE;
    let path = write_activity_snapshot(&tmp, capability, 100, 160);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let mode_error = read_activity_ledger_today(
        path.to_str().unwrap(),
        capability,
        &expected_owner_pubkey(),
        &json!({}),
        120,
    )
    .unwrap_err();
    assert!(
        mode_error.contains("snapshot mode must be 0600"),
        "got: {mode_error}"
    );

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link_path = tmp.path().join("linked.json");
    symlink(&path, &link_path).unwrap();
    let symlink_error = read_activity_ledger_today(
        link_path.to_str().unwrap(),
        capability,
        &expected_owner_pubkey(),
        &json!({}),
        120,
    )
    .unwrap_err();
    assert!(
        symlink_error.contains("must not be a symlink")
            || symlink_error.contains("could not open snapshot"),
        "got: {symlink_error}"
    );
}

#[test]
fn activity_ledger_today_rejects_same_user_forged_rewrite_and_wrong_owner_env() {
    let tmp = TempDir::new().unwrap();
    let capability = ACTIVITY_LEDGER_TODAY_CAPABILITY_VALUE;
    let path = write_activity_snapshot(&tmp, capability, 100, 160);
    let expected_owner = expected_owner_pubkey();
    let mut snapshot: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    snapshot["surface"]["journals"] = json!([{ "id": "forged" }]);
    std::fs::write(&path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
    let forged_error = read_activity_ledger_today(
        path.to_str().unwrap(),
        capability,
        &expected_owner,
        &json!({}),
        120,
    )
    .unwrap_err();
    assert!(
        forged_error.contains("snapshotSha256 does not match")
            || forged_error.contains("signature verification failed"),
        "got: {forged_error}"
    );

    let fresh_path = write_activity_snapshot(&tmp, capability, 100, 160);
    let wrong_owner_error = read_activity_ledger_today(
        fresh_path.to_str().unwrap(),
        capability,
        &"f".repeat(64),
        &json!({}),
        120,
    )
    .unwrap_err();
    assert!(
        wrong_owner_error.contains("ownerPubkey mismatch"),
        "got: {wrong_owner_error}"
    );
}
