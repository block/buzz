//! Owner-scoped, read-only Activity Ledger snapshot for local consumers.
//!
//! The frontend already owns the canonical journal projection. This module is
//! deliberately only the secure persistence seam: it validates the projection
//! envelope against the active owner, writes it atomically with mode 0600 on
//! Unix, and revalidates it on read. It never receives or serializes secret
//! identity key material.

use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const TODAY_SNAPSHOT_SCHEMA: &str = "buzz.activity-ledger.today/v1";
pub const TODAY_SNAPSHOT_CAPABILITY: &str = "buzz.activity-ledger.today.read/v1";
pub const TODAY_SNAPSHOT_SIGNED_KIND: u16 = 24202;
const TODAY_SNAPSHOT_TAG_MARKER: &str = "buzz-activity-ledger-today";
const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
const MAX_RAW_EVENTS: usize = 10_000;
const MAX_LIFETIME_SECS: i64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnsignedOwnerTodaySnapshot {
    pub schema: String,
    pub owner_pubkey: String,
    pub relay_url: String,
    pub generated_at: i64,
    pub expires_at: i64,
    pub capability: String,
    pub surface: serde_json::Value,
    pub raw_events: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnerTodaySnapshot {
    #[serde(flatten)]
    pub payload: UnsignedOwnerTodaySnapshot,
    pub snapshot_sha256: String,
    pub event_id: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodaySnapshotReceipt {
    pub path: String,
    pub owner_pubkey: String,
    pub relay_url: String,
    pub generated_at: i64,
    pub expires_at: i64,
    pub byte_length: usize,
    pub sha256: String,
}

pub(crate) fn snapshot_path(nest_dir: &Path, owner_pubkey: &str, relay_url: &str) -> PathBuf {
    let relay_scope = hex::encode(Sha256::digest(relay_url.as_bytes()));
    nest_dir.join("archive").join(format!(
        "activity-ledger-today-{owner_pubkey}-{relay_scope}.json"
    ))
}

fn validate_owner_pubkey(owner_pubkey: &str) -> Result<(), String> {
    if owner_pubkey.len() != 64
        || !owner_pubkey
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("Today snapshot ownerPubkey must be lowercase 64-character hex".into());
    }
    Ok(())
}

fn normalize_relay_url(relay_url: &str) -> Result<String, String> {
    if relay_url.is_empty() || relay_url.len() > 2_048 {
        return Err("Today snapshot relayUrl must contain between 1 and 2048 bytes".into());
    }
    buzz_core_pkg::relay::normalize_relay_url(relay_url)
        .map_err(|error| format!("Today snapshot relayUrl is invalid: {error}"))
}

fn validate_hex(value: &str, label: &str, expected_len: usize) -> Result<(), String> {
    if value.len() != expected_len || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "Today snapshot {label} must be {expected_len}-character lowercase hex"
        ));
    }
    if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(format!(
            "Today snapshot {label} must be {expected_len}-character lowercase hex"
        ));
    }
    Ok(())
}

const REDACTED_IDENTITY_SECRET_TEXT: &str = "[REDACTED: identity secret material]";

fn redact_identity_secret_text(value: &mut serde_json::Value) -> Result<usize, String> {
    match value {
        serde_json::Value::Object(object) => {
            let mut redactions = 0;
            for (key, child) in object {
                let normalized_key = key
                    .chars()
                    .filter(|ch| ch.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect::<String>();
                if matches!(
                    normalized_key.as_str(),
                    "nsec" | "secretkey" | "privatekey" | "nostrsecretkey"
                ) {
                    return Err("Today snapshot cannot contain identity secret fields".into());
                }
                redactions += redact_identity_secret_text(child)?;
            }
            Ok(redactions)
        }
        serde_json::Value::Array(values) => {
            let mut redactions = 0;
            for child in values {
                redactions += redact_identity_secret_text(child)?;
            }
            Ok(redactions)
        }
        serde_json::Value::String(text) => {
            let lowercase = text.to_ascii_lowercase();
            if lowercase.contains("nsec1") || lowercase.contains("nostr_secret_key=") {
                *text = REDACTED_IDENTITY_SECRET_TEXT.into();
                return Ok(1);
            }
            Ok(0)
        }
        _ => Ok(0),
    }
}

fn clear_identity_secret_redaction_marker(surface: &mut serde_json::Value) -> Result<(), String> {
    let surface = surface
        .as_object_mut()
        .ok_or_else(|| "Today snapshot surface must be a JSON object".to_string())?;
    if let Some(projection) = surface.get_mut("snapshotProjection") {
        let projection = projection.as_object_mut().ok_or_else(|| {
            "Today snapshot surface.snapshotProjection must be a JSON object".to_string()
        })?;
        projection.remove("identitySecretsRedacted");
    }
    Ok(())
}

fn record_identity_secret_redactions(
    surface: &mut serde_json::Value,
    redactions: usize,
) -> Result<(), String> {
    if redactions == 0 {
        return Ok(());
    }
    let surface = surface
        .as_object_mut()
        .ok_or_else(|| "Today snapshot surface must be a JSON object".to_string())?;
    let projection = surface
        .entry("snapshotProjection")
        .or_insert_with(|| serde_json::json!({}));
    let projection = projection.as_object_mut().ok_or_else(|| {
        "Today snapshot surface.snapshotProjection must be a JSON object".to_string()
    })?;
    projection.insert(
        "identitySecretsRedacted".into(),
        serde_json::Value::from(redactions),
    );
    projection.insert("bounded".into(), serde_json::Value::Bool(true));
    Ok(())
}

fn parse_unsigned_snapshot(
    snapshot_json: &str,
    expected_owner_pubkey: &str,
    expected_relay_url: &str,
    now: i64,
    require_unexpired: bool,
    redact_secret_text: bool,
) -> Result<UnsignedOwnerTodaySnapshot, String> {
    if snapshot_json.is_empty() || snapshot_json.len() > MAX_SNAPSHOT_BYTES {
        return Err(format!(
            "Today snapshot must contain between 1 and {MAX_SNAPSHOT_BYTES} bytes"
        ));
    }
    validate_owner_pubkey(expected_owner_pubkey)?;
    let expected_relay_url = normalize_relay_url(expected_relay_url)?;
    let mut snapshot: UnsignedOwnerTodaySnapshot = serde_json::from_str(snapshot_json)
        .map_err(|error| format!("parse Today snapshot: {error}"))?;
    if snapshot.schema != TODAY_SNAPSHOT_SCHEMA {
        return Err("unsupported Today snapshot schema".into());
    }
    if snapshot.capability != TODAY_SNAPSHOT_CAPABILITY {
        return Err("unsupported Today snapshot capability".into());
    }
    if snapshot.owner_pubkey != expected_owner_pubkey {
        return Err("Today snapshot owner does not match the active identity".into());
    }
    let snapshot_relay_url = normalize_relay_url(&snapshot.relay_url)?;
    if snapshot_relay_url != expected_relay_url {
        return Err("Today snapshot relay does not match the active workspace".into());
    }
    snapshot.relay_url = expected_relay_url;
    if snapshot.generated_at < 0 || snapshot.expires_at < 0 {
        return Err("Today snapshot timestamps must be non-negative".into());
    }
    if snapshot.generated_at > now + 300 {
        return Err("Today snapshot generatedAt is too far in the future".into());
    }
    if snapshot.expires_at <= snapshot.generated_at
        || snapshot.expires_at - snapshot.generated_at > MAX_LIFETIME_SECS
    {
        return Err("Today snapshot lifetime must be positive and at most 24 hours".into());
    }
    if require_unexpired && snapshot.expires_at <= now {
        return Err("Today snapshot has expired".into());
    }
    if !snapshot.surface.is_object() {
        return Err("Today snapshot surface must be a JSON object".into());
    }
    if snapshot.raw_events.len() > MAX_RAW_EVENTS {
        return Err(format!(
            "Today snapshot rawEvents exceeds {MAX_RAW_EVENTS} records"
        ));
    }
    if redact_secret_text {
        clear_identity_secret_redaction_marker(&mut snapshot.surface)?;
        let mut redactions = redact_identity_secret_text(&mut snapshot.surface)?;
        for event in &mut snapshot.raw_events {
            redactions += redact_identity_secret_text(event)?;
        }
        record_identity_secret_redactions(&mut snapshot.surface, redactions)?;
    } else {
        let mut sanitized_surface = snapshot.surface.clone();
        let mut redactions = redact_identity_secret_text(&mut sanitized_surface)?;
        for event in &snapshot.raw_events {
            let mut sanitized_event = event.clone();
            redactions += redact_identity_secret_text(&mut sanitized_event)?;
        }
        if redactions > 0 {
            return Err("Today snapshot contains unsanitized identity secret material".into());
        }
    }
    Ok(snapshot)
}

fn canonical_payload_json(snapshot: &UnsignedOwnerTodaySnapshot) -> Result<String, String> {
    serde_json::to_string(snapshot)
        .map_err(|error| format!("serialize canonical Today snapshot: {error}"))
}

fn snapshot_sha256(snapshot: &UnsignedOwnerTodaySnapshot) -> Result<String, String> {
    let canonical_json = canonical_payload_json(snapshot)?;
    Ok(hex::encode(Sha256::digest(canonical_json.as_bytes())))
}

fn tag(name: &str, value: &str) -> Result<Tag, String> {
    Tag::parse([name, value]).map_err(|error| format!("build Today snapshot {name} tag: {error}"))
}

fn signed_snapshot_tags(
    snapshot: &UnsignedOwnerTodaySnapshot,
    snapshot_sha256: &str,
) -> Result<Vec<Tag>, String> {
    Ok(vec![
        tag("t", TODAY_SNAPSHOT_TAG_MARKER)?,
        tag("schema", TODAY_SNAPSHOT_SCHEMA)?,
        tag("capability", TODAY_SNAPSHOT_CAPABILITY)?,
        tag("snapshot_sha256", snapshot_sha256)?,
        tag("expires_at", &snapshot.expires_at.to_string())?,
    ])
}

fn build_signed_snapshot(
    owner_keys: &Keys,
    snapshot: UnsignedOwnerTodaySnapshot,
) -> Result<OwnerTodaySnapshot, String> {
    let owner_pubkey = owner_keys.public_key().to_hex();
    if owner_pubkey != snapshot.owner_pubkey {
        return Err("Today snapshot signer does not match the active owner identity".into());
    }
    let snapshot_sha256 = snapshot_sha256(&snapshot)?;
    let content = canonical_payload_json(&snapshot)?;
    let event = EventBuilder::new(Kind::Custom(TODAY_SNAPSHOT_SIGNED_KIND), content)
        .tags(signed_snapshot_tags(&snapshot, &snapshot_sha256)?)
        .custom_created_at(Timestamp::from(snapshot.generated_at as u64))
        .sign_with_keys(owner_keys)
        .map_err(|error| format!("sign Today snapshot: {error}"))?;
    if event.pubkey.to_hex() != snapshot.owner_pubkey {
        return Err("Today snapshot signer does not match the active owner identity".into());
    }
    Ok(OwnerTodaySnapshot {
        payload: snapshot,
        snapshot_sha256,
        event_id: event.id.to_hex(),
        signature: event.sig.to_string(),
    })
}

fn signed_event_json(snapshot: &OwnerTodaySnapshot) -> Result<String, String> {
    let content = canonical_payload_json(&snapshot.payload)?;
    let tags = signed_snapshot_tags(&snapshot.payload, &snapshot.snapshot_sha256)?
        .into_iter()
        .map(|tag| {
            serde_json::Value::Array(
                tag.as_slice()
                    .iter()
                    .map(|value| serde_json::Value::String(value.clone()))
                    .collect(),
            )
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({
        "id": snapshot.event_id,
        "pubkey": snapshot.payload.owner_pubkey,
        "created_at": snapshot.payload.generated_at,
        "kind": TODAY_SNAPSHOT_SIGNED_KIND,
        "tags": tags,
        "content": content,
        "sig": snapshot.signature,
    })
    .to_string())
}

fn parse_and_validate_signed_snapshot(
    snapshot_json: &str,
    expected_owner_pubkey: &str,
    expected_relay_url: &str,
    now: i64,
    require_unexpired: bool,
) -> Result<OwnerTodaySnapshot, String> {
    if snapshot_json.is_empty() || snapshot_json.len() > MAX_SNAPSHOT_BYTES {
        return Err(format!(
            "Today snapshot must contain between 1 and {MAX_SNAPSHOT_BYTES} bytes"
        ));
    }
    validate_owner_pubkey(expected_owner_pubkey)?;
    let snapshot: OwnerTodaySnapshot = serde_json::from_str(snapshot_json)
        .map_err(|error| format!("parse Today snapshot: {error}"))?;
    let payload_json = canonical_payload_json(&snapshot.payload)?;
    parse_unsigned_snapshot(
        &payload_json,
        expected_owner_pubkey,
        expected_relay_url,
        now,
        require_unexpired,
        false,
    )?;
    validate_hex(&snapshot.snapshot_sha256, "snapshotSha256", 64)?;
    validate_hex(&snapshot.event_id, "eventId", 64)?;
    validate_hex(&snapshot.signature, "signature", 128)?;
    let expected_snapshot_sha256 = hex::encode(Sha256::digest(payload_json.as_bytes()));
    if snapshot.snapshot_sha256 != expected_snapshot_sha256 {
        return Err("Today snapshot snapshotSha256 does not match the canonical payload".into());
    }
    let event = Event::from_json(signed_event_json(&snapshot)?)
        .map_err(|error| format!("parse signed Today snapshot event: {error}"))?;
    event
        .verify()
        .map_err(|error| format!("Today snapshot signature verification failed: {error}"))?;
    if event.kind.as_u16() != TODAY_SNAPSHOT_SIGNED_KIND {
        return Err(format!(
            "Today snapshot event must use kind {TODAY_SNAPSHOT_SIGNED_KIND}"
        ));
    }
    if event.pubkey.to_hex() != expected_owner_pubkey {
        return Err("Today snapshot signer is not the active owner identity".into());
    }
    if event.id.to_hex() != snapshot.event_id {
        return Err("Today snapshot eventId does not match the signed event".into());
    }
    Ok(snapshot)
}

#[cfg(unix)]
fn enforce_private_permissions(path: &Path, directory: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if directory { 0o700 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|error| format!("set private Today snapshot permissions: {error}"))
}

#[cfg(not(unix))]
fn enforce_private_permissions(_path: &Path, _directory: bool) -> Result<(), String> {
    Ok(())
}

pub fn write_owner_today_snapshot(
    nest_dir: &Path,
    owner_keys: &Keys,
    expected_owner_pubkey: &str,
    expected_relay_url: &str,
    snapshot_json: &str,
    now: i64,
) -> Result<TodaySnapshotReceipt, String> {
    let snapshot = parse_unsigned_snapshot(
        snapshot_json,
        expected_owner_pubkey,
        expected_relay_url,
        now,
        true,
        true,
    )?;
    let signed_snapshot = build_signed_snapshot(owner_keys, snapshot)?;
    let canonical_json = serde_json::to_string(&signed_snapshot)
        .map_err(|error| format!("serialize signed Today snapshot: {error}"))?;
    let archive_dir = nest_dir.join("archive");
    std::fs::create_dir_all(&archive_dir)
        .map_err(|error| format!("create Today snapshot directory: {error}"))?;
    enforce_private_permissions(&archive_dir, true)?;

    let destination = snapshot_path(
        nest_dir,
        expected_owner_pubkey,
        &signed_snapshot.payload.relay_url,
    );
    // Keep the temporary file in the destination directory. Besides making
    // replacement atomic, this rules out Windows' cross-volume copy/delete
    // path; file contents are synced below before the safe replace-existing
    // persist call publishes the new name.
    let mut temp_file = tempfile::Builder::new()
        .prefix(".activity-ledger-today-")
        .tempfile_in(&archive_dir)
        .map_err(|error| format!("create Today snapshot temp file: {error}"))?;
    enforce_private_permissions(temp_file.path(), false)?;
    temp_file
        .write_all(canonical_json.as_bytes())
        .map_err(|error| format!("write Today snapshot: {error}"))?;
    temp_file
        .as_file()
        .sync_all()
        .map_err(|error| format!("sync Today snapshot: {error}"))?;
    temp_file
        .persist(&destination)
        .map_err(|error| format!("atomically publish Today snapshot: {}", error.error))?;
    enforce_private_permissions(&destination, false)?;
    #[cfg(unix)]
    std::fs::File::open(&archive_dir)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("sync Today snapshot directory: {error}"))?;

    let sha256 = hex::encode(Sha256::digest(canonical_json.as_bytes()));
    Ok(TodaySnapshotReceipt {
        path: destination.to_string_lossy().into_owned(),
        owner_pubkey: signed_snapshot.payload.owner_pubkey,
        relay_url: signed_snapshot.payload.relay_url,
        generated_at: signed_snapshot.payload.generated_at,
        expires_at: signed_snapshot.payload.expires_at,
        byte_length: canonical_json.len(),
        sha256,
    })
}

pub fn read_owner_today_snapshot(
    nest_dir: &Path,
    expected_owner_pubkey: &str,
    expected_relay_url: &str,
    now: i64,
) -> Result<String, String> {
    let expected_relay_url = normalize_relay_url(expected_relay_url)?;
    let path = snapshot_path(nest_dir, expected_owner_pubkey, &expected_relay_url);
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| format!("read owner Today snapshot: {error}"))?;
    parse_and_validate_signed_snapshot(
        &raw,
        expected_owner_pubkey,
        &expected_relay_url,
        now,
        true,
    )?;
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    const TEST_RELAY: &str = "wss://relay-a.test";

    fn snapshot_for_relay(owner: &str, relay_url: &str, generated_at: i64) -> String {
        serde_json::json!({
            "schema": TODAY_SNAPSHOT_SCHEMA,
            "ownerPubkey": owner,
            "relayUrl": relay_url,
            "generatedAt": generated_at,
            "expiresAt": generated_at + 3600,
            "capability": TODAY_SNAPSHOT_CAPABILITY,
            "surface": {"date": "2026-08-21", "journals": []},
            "rawEvents": [{"journalId": "j-1", "proofState": "OBSERVED"}]
        })
        .to_string()
    }

    fn snapshot(owner: &str, generated_at: i64) -> String {
        snapshot_for_relay(owner, TEST_RELAY, generated_at)
    }

    #[test]
    fn desktop_canonical_payload_matches_shared_consumer_fixture() {
        let fixture =
            include_str!("../../../../test-fixtures/activity-ledger-today-desktop.json").trim();
        let snapshot: UnsignedOwnerTodaySnapshot = serde_json::from_str(fixture).unwrap();
        assert_eq!(canonical_payload_json(&snapshot).unwrap(), fixture);
    }

    #[test]
    fn snapshot_is_atomic_private_and_owner_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::parse(&"a".repeat(64)).unwrap();
        let owner = keys.public_key().to_hex();
        let receipt = write_owner_today_snapshot(
            dir.path(),
            &keys,
            &owner,
            TEST_RELAY,
            &snapshot(&owner, 1000),
            1000,
        )
        .unwrap();
        assert_eq!(receipt.owner_pubkey, owner);
        assert_eq!(receipt.sha256.len(), 64);
        let raw = read_owner_today_snapshot(dir.path(), &owner, TEST_RELAY, 1001).unwrap();
        assert!(raw.contains(TODAY_SNAPSHOT_CAPABILITY));
        assert!(raw.contains("\"snapshotSha256\""));
        assert!(raw.contains("\"eventId\""));
        assert!(raw.contains("\"signature\""));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&receipt.path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn snapshot_wrong_owner_expiry_and_capability_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let owner_keys = Keys::parse(&"a".repeat(64)).unwrap();
        let owner = owner_keys.public_key().to_hex();
        let other = Keys::parse(&"b".repeat(64)).unwrap().public_key().to_hex();
        assert!(write_owner_today_snapshot(
            dir.path(),
            &owner_keys,
            &other,
            TEST_RELAY,
            &snapshot(&owner, 1000),
            1000
        )
        .unwrap_err()
        .contains("owner does not match"));

        let mut value: serde_json::Value = serde_json::from_str(&snapshot(&owner, 1000)).unwrap();
        value["capability"] = "write-anything".into();
        assert!(write_owner_today_snapshot(
            dir.path(),
            &owner_keys,
            &owner,
            TEST_RELAY,
            &value.to_string(),
            1000
        )
        .unwrap_err()
        .contains("capability"));

        assert!(write_owner_today_snapshot(
            dir.path(),
            &owner_keys,
            &owner,
            TEST_RELAY,
            &snapshot(&owner, 1000),
            5000
        )
        .unwrap_err()
        .contains("expired"));
    }

    #[test]
    fn snapshot_paths_and_validation_are_relay_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::parse(&"a".repeat(64)).unwrap();
        let owner = keys.public_key().to_hex();
        let other_relay = "wss://relay-b.test";
        let first = write_owner_today_snapshot(
            dir.path(),
            &keys,
            &owner,
            TEST_RELAY,
            &snapshot_for_relay(&owner, TEST_RELAY, 1000),
            1000,
        )
        .unwrap();
        let second = write_owner_today_snapshot(
            dir.path(),
            &keys,
            &owner,
            other_relay,
            &snapshot_for_relay(&owner, other_relay, 1000),
            1000,
        )
        .unwrap();
        assert_ne!(first.path, second.path);

        let equivalent = write_owner_today_snapshot(
            dir.path(),
            &keys,
            &owner,
            "wss://RELAY-A.TEST:443/",
            &snapshot_for_relay(&owner, TEST_RELAY, 1001),
            1001,
        )
        .unwrap();
        assert_eq!(first.path, equivalent.path);
        assert_eq!(equivalent.relay_url, TEST_RELAY);

        std::fs::copy(&second.path, &first.path).unwrap();
        let error = read_owner_today_snapshot(dir.path(), &owner, TEST_RELAY, 1001).unwrap_err();
        assert!(error.contains("relay does not match"), "got: {error}");
    }

    #[test]
    fn snapshot_read_revalidates_tampering_and_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::parse(&"a".repeat(64)).unwrap();
        let owner = keys.public_key().to_hex();
        let first = write_owner_today_snapshot(
            dir.path(),
            &keys,
            &owner,
            TEST_RELAY,
            &snapshot(&owner, 1000),
            1000,
        )
        .unwrap();
        let second = write_owner_today_snapshot(
            dir.path(),
            &keys,
            &owner,
            TEST_RELAY,
            &snapshot(&owner, 1001),
            1001,
        )
        .unwrap();
        assert_eq!(first.path, second.path);
        assert_ne!(first.sha256, second.sha256);
        let replaced: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&second.path).unwrap()).unwrap();
        assert_eq!(replaced["generatedAt"], 1001);
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&second.path).unwrap()).unwrap();
        value["ownerPubkey"] = "b".repeat(64).into();
        std::fs::write(&second.path, value.to_string()).unwrap();
        assert!(
            read_owner_today_snapshot(dir.path(), &owner, TEST_RELAY, 1002)
                .unwrap_err()
                .contains("owner does not match")
        );
    }

    #[test]
    fn snapshot_redacts_identity_secret_fields_and_values_without_suppressing_today() {
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::parse(&"a".repeat(64)).unwrap();
        let owner = keys.public_key().to_hex();
        let mut field: serde_json::Value = serde_json::from_str(&snapshot(&owner, 1000)).unwrap();
        field["surface"]["secretKey"] = "do-not-export".into();
        assert!(write_owner_today_snapshot(
            dir.path(),
            &keys,
            &owner,
            TEST_RELAY,
            &field.to_string(),
            1000
        )
        .unwrap_err()
        .contains("identity secret"));

        let mut value: serde_json::Value = serde_json::from_str(&snapshot(&owner, 1000)).unwrap();
        value["surface"]["journals"] = serde_json::json!([{
            "id": "journal-with-example",
            "summary": "Documentation example: NoStR_SeCrEt_KeY=redacted-placeholder",
            "detail": "A generic secret-handling note is safe"
        }]);
        value["rawEvents"][0]["detail"] = "nsec1should-never-leave-the-owner-boundary".into();
        let receipt = write_owner_today_snapshot(
            dir.path(),
            &keys,
            &owner,
            TEST_RELAY,
            &value.to_string(),
            1000,
        )
        .unwrap();
        let raw = read_owner_today_snapshot(dir.path(), &owner, TEST_RELAY, 1001).unwrap();
        assert!(!raw.contains("do-not-export"));
        assert!(!raw.to_ascii_lowercase().contains("nsec1"));
        assert!(!raw.to_ascii_lowercase().contains("nostr_secret_key="));
        let stored: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            stored["surface"]["snapshotProjection"]["identitySecretsRedacted"],
            2
        );
        assert_eq!(
            stored["surface"]["journals"][0]["summary"],
            "[REDACTED: identity secret material]"
        );
        assert_eq!(
            stored["surface"]["journals"][0]["detail"],
            "A generic secret-handling note is safe"
        );
        assert_eq!(
            stored["rawEvents"][0]["detail"],
            "[REDACTED: identity secret material]"
        );
        assert_eq!(receipt.owner_pubkey, owner);
    }

    #[test]
    fn snapshot_read_rejects_same_user_forged_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::parse(&"a".repeat(64)).unwrap();
        let owner = keys.public_key().to_hex();
        let receipt = write_owner_today_snapshot(
            dir.path(),
            &keys,
            &owner,
            TEST_RELAY,
            &snapshot(&owner, 1000),
            1000,
        )
        .unwrap();
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&receipt.path).unwrap()).unwrap();
        value["surface"]["journals"] = serde_json::json!([{ "id": "forged" }]);
        std::fs::write(&receipt.path, value.to_string()).unwrap();
        let error = read_owner_today_snapshot(dir.path(), &owner, TEST_RELAY, 1001).unwrap_err();
        assert!(
            error.contains("snapshotSha256 does not match")
                || error.contains("signature verification failed"),
            "got: {error}"
        );
    }
}
