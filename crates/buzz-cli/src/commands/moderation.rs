//! `buzz moderation` — community moderation queue, enforcement, and audit.
//!
//! Mutations (`ban`/`unban`/`timeout`/`untimeout`/`resolve`) are signed
//! command events (kinds 9040–9044) submitted via `POST /events`, mirroring
//! the NIP-43 relay-admin 9030-series: the relay validates, authorizes
//! (owner/admin only), and executes them directly — they are never stored.
//!
//! Reads (`reports`/`restricted`/`audit`) hit dedicated mod-only,
//! NIP-98-authed relay endpoints under `/moderation/*`, because reports and
//! audit rows are structured queue rows, not public nostr events — serving
//! them over a REQ filter would mean synthesizing fake events and threading a
//! privileged authz check into the public read path.
//!
//! The community (tenant) is selected by the relay host — moderation commands
//! carry no channel scope.

use nostr::Timestamp;
use serde_json::{Map, Value};

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{format_npub, normalize_pubkey, validate_hex64};
use crate::{ModerationCmd, OutputFormat};

/// Project a dual-field moderation response onto the compact human/agent-facing
/// npub contract. JSON output stays byte-for-byte relay-compatible for scripts;
/// compact output prefers additive npub fields and removes only their legacy
/// compatibility duplicates. Event IDs and blob hashes stay hex.
fn project_moderation_response(raw: &str) -> Result<String, CliError> {
    let mut value: Value = serde_json::from_str(raw)
        .map_err(|error| CliError::Other(format!("invalid moderation response: {error}")))?;
    let rows = value
        .as_array_mut()
        .ok_or_else(|| CliError::Other("invalid moderation response: expected an array".into()))?;
    for row in rows {
        let object = row.as_object_mut().ok_or_else(|| {
            CliError::Other("invalid moderation response: expected object rows".into())
        })?;
        project_moderation_row(object);
    }
    serde_json::to_string(&value).map_err(|error| {
        CliError::Other(format!("moderation response serialization failed: {error}"))
    })
}

fn render_moderation_response(raw: &str, format: &OutputFormat) -> Result<String, CliError> {
    match format {
        OutputFormat::Json => Ok(raw.to_string()),
        OutputFormat::Compact => project_moderation_response(raw),
    }
}

fn prefer_npub(object: &mut Map<String, Value>, legacy: &str, canonical: &str) {
    if !object.contains_key(legacy) {
        return;
    }
    let canonical_value = object.remove(canonical);
    let identity = canonical_value
        .filter(|value| !value.is_null())
        .or_else(|| object.get(legacy).cloned());
    match identity {
        Some(Value::String(value)) => {
            let npub = format_npub(&value).unwrap_or_else(|_| "<invalid-pubkey>".to_string());
            object.insert(legacy.to_string(), Value::String(npub));
        }
        Some(value) if object.contains_key(legacy) => {
            object.insert(legacy.to_string(), value);
        }
        _ => {}
    }
}

fn project_moderation_row(object: &mut Map<String, Value>) {
    prefer_npub(object, "reporter_pubkey", "reporter_npub");
    prefer_npub(object, "resolved_by", "resolved_by_npub");
    prefer_npub(object, "actor_pubkey", "actor_npub");
    prefer_npub(object, "target_pubkey", "target_npub");
    prefer_npub(object, "pubkey", "npub");
    if object.get("target_kind").and_then(Value::as_str) == Some("pubkey") {
        prefer_npub(object, "target", "target_npub");
    } else {
        object.remove("target_npub");
    }
}

/// Resolve `--expires-in <secs>` / `--expires-at <unix>` into an absolute
/// unix-seconds expiry. At most one may be set (enforced by clap).
fn resolve_expiry(expires_in: Option<u64>, expires_at: Option<u64>) -> Option<u64> {
    match (expires_in, expires_at) {
        (Some(secs), _) => Some(Timestamp::now().as_secs() + secs),
        (None, Some(ts)) => Some(ts),
        (None, None) => None,
    }
}

async fn cmd_ban(
    client: &BuzzClient,
    pubkey: &str,
    expires_in: Option<u64>,
    expires_at: Option<u64>,
    reason: Option<&str>,
) -> Result<(), CliError> {
    let pubkey = normalize_pubkey(pubkey)?;
    let expiry = resolve_expiry(expires_in, expires_at);
    let builder = buzz_sdk::build_moderation_ban(&pubkey, expiry, reason)
        .map_err(|e| CliError::Usage(format!("invalid ban: {e}")))?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

async fn cmd_unban(client: &BuzzClient, pubkey: &str) -> Result<(), CliError> {
    let pubkey = normalize_pubkey(pubkey)?;
    let builder = buzz_sdk::build_moderation_unban(&pubkey)
        .map_err(|e| CliError::Usage(format!("invalid unban: {e}")))?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

async fn cmd_timeout(
    client: &BuzzClient,
    pubkey: &str,
    expires_in: Option<u64>,
    expires_at: Option<u64>,
    reason: Option<&str>,
) -> Result<(), CliError> {
    let pubkey = normalize_pubkey(pubkey)?;
    let expiry = resolve_expiry(expires_in, expires_at)
        .ok_or_else(|| CliError::Usage("timeout requires --expires-in or --expires-at".into()))?;
    let builder = buzz_sdk::build_moderation_timeout(&pubkey, expiry, reason)
        .map_err(|e| CliError::Usage(format!("invalid timeout: {e}")))?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

async fn cmd_untimeout(client: &BuzzClient, pubkey: &str) -> Result<(), CliError> {
    let pubkey = normalize_pubkey(pubkey)?;
    let builder = buzz_sdk::build_moderation_untimeout(&pubkey)
        .map_err(|e| CliError::Usage(format!("invalid untimeout: {e}")))?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

async fn cmd_resolve(
    client: &BuzzClient,
    report: &str,
    status: &str,
    action: &str,
    reason: Option<&str>,
) -> Result<(), CliError> {
    validate_hex64(report)?;
    let builder = buzz_sdk::build_moderation_resolve_report(report, status, action, reason)
        .map_err(|e| CliError::Usage(format!("invalid resolution: {e}")))?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

async fn cmd_reports(
    client: &BuzzClient,
    status: Option<&str>,
    limit: i64,
    format: &OutputFormat,
) -> Result<(), CliError> {
    let mut path = format!("/moderation/reports?limit={limit}");
    if let Some(s) = status {
        path.push_str(&format!("&status={s}"));
    }
    let resp = client.get_authed(&path).await?;
    println!("{}", render_moderation_response(&resp, format)?);
    Ok(())
}

async fn cmd_restricted(client: &BuzzClient, format: &OutputFormat) -> Result<(), CliError> {
    let resp = client.get_authed("/moderation/restricted").await?;
    println!("{}", render_moderation_response(&resp, format)?);
    Ok(())
}

async fn cmd_audit(client: &BuzzClient, limit: i64, format: &OutputFormat) -> Result<(), CliError> {
    let resp = client
        .get_authed(&format!("/moderation/audit?limit={limit}"))
        .await?;
    println!("{}", render_moderation_response(&resp, format)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "ea9b4d7a7a78a3e3729e5568b14d764d4962be0e1f20f749bcf8d9dbbf9a9328";
    const NPUB: &str = "npub1a2d567n60z37xu57245tzntkf4yk90swrus0wjdulrvah0u6jv5qusyp60";

    #[test]
    fn compact_read_projection_prefers_npub_and_omits_legacy_identity_hex() {
        let raw = serde_json::json!([{
            "report_event_id": "11".repeat(32),
            "reporter_pubkey": HEX,
            "reporter_npub": NPUB,
            "target_kind": "pubkey",
            "target": HEX,
            "target_npub": NPUB,
            "resolved_by": HEX,
            "resolved_by_npub": NPUB
        }])
        .to_string();

        let projected =
            render_moderation_response(&raw, &OutputFormat::Compact).expect("project response");
        let value: Value = serde_json::from_str(&projected).expect("parse projection");
        assert_eq!(value[0]["reporter_pubkey"], NPUB);
        assert_eq!(value[0]["target"], NPUB);
        assert_eq!(value[0]["resolved_by"], NPUB);
        assert_eq!(value[0]["report_event_id"], "11".repeat(32));
        assert!(!projected.contains(HEX));
        assert!(value[0].get("reporter_npub").is_none());
        assert!(value[0].get("target_npub").is_none());
    }

    #[test]
    fn compact_read_projection_canonicalizes_identity_fields_from_older_relays() {
        let raw = serde_json::json!([{
            "reporter_pubkey": HEX,
            "target_kind": "event",
            "target": "11".repeat(32)
        }])
        .to_string();

        let projected =
            render_moderation_response(&raw, &OutputFormat::Compact).expect("project response");
        let value: Value = serde_json::from_str(&projected).expect("parse projection");
        assert_eq!(value[0]["reporter_pubkey"], NPUB);
        assert_eq!(value[0]["target"], "11".repeat(32));
        assert!(!projected.contains(HEX));
    }

    #[test]
    fn json_read_preserves_relay_dual_field_bytes() {
        let raw =
            format!("[ {{ \"reporter_pubkey\": \"{HEX}\", \"reporter_npub\": \"{NPUB}\" }} ]");

        let rendered =
            render_moderation_response(&raw, &OutputFormat::Json).expect("render response");
        assert_eq!(rendered, raw);
    }
}

pub async fn dispatch(
    cmd: ModerationCmd,
    client: &BuzzClient,
    format: &OutputFormat,
) -> Result<(), CliError> {
    match cmd {
        ModerationCmd::Reports { status, limit } => {
            cmd_reports(client, status.as_deref(), limit, format).await
        }
        ModerationCmd::Resolve {
            report,
            status,
            action,
            reason,
        } => cmd_resolve(client, &report, &status, &action, reason.as_deref()).await,
        ModerationCmd::Ban {
            pubkey,
            expires_in,
            expires_at,
            reason,
        } => cmd_ban(client, &pubkey, expires_in, expires_at, reason.as_deref()).await,
        ModerationCmd::Unban { pubkey } => cmd_unban(client, &pubkey).await,
        ModerationCmd::Timeout {
            pubkey,
            expires_in,
            expires_at,
            reason,
        } => cmd_timeout(client, &pubkey, expires_in, expires_at, reason.as_deref()).await,
        ModerationCmd::Untimeout { pubkey } => cmd_untimeout(client, &pubkey).await,
        ModerationCmd::Restricted => cmd_restricted(client, format).await,
        ModerationCmd::Audit { limit } => cmd_audit(client, limit, format).await,
    }
}
