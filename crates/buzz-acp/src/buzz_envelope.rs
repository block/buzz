//! Authenticated Buzz transport metadata for ACP `session/prompt` requests.
//!
//! Rendered prompt text is intentionally not an input to the trusted envelope.
//! The envelope is built only from the already-verified Nostr events and the
//! authoritative channel membership snapshot supplied by the harness.

use anyhow::{anyhow, bail, Context, Result};
use nostr::secp256k1::{Keypair, Message};
use nostr::{Event, Keys, SECP256K1};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const ENVELOPE_DOMAIN: &[u8] = b"buzz-acp-envelope-v1\0";

/// Verified inputs used to build one authenticated Buzz ACP prompt envelope.
pub struct VerifiedBuzzPromptInput<'a> {
    /// ACP session receiving the prompt.
    pub session_id: &'a str,
    /// Rendered prompt blocks. These remain untrusted message content.
    pub prompt_blocks: &'a [&'a str],
    /// Authoritative Buzz channel UUID.
    pub channel_id: &'a str,
    /// Authoritative Buzz channel classification.
    pub channel_type: &'a str,
    /// Verified owner public key.
    pub owner_pubkey: &'a str,
    /// Managed agent signing identity.
    pub agent_keys: &'a Keys,
    /// Authoritative channel membership snapshot.
    pub members: &'a [(String, String)],
    /// Ordered, verified inbound Nostr events in this delivery.
    pub events: &'a [Event],
}

/// Build ACP prompt params carrying an authenticated Buzz v1 delivery envelope.
pub fn build_verified_buzz_prompt_params(input: VerifiedBuzzPromptInput<'_>) -> Result<Value> {
    let VerifiedBuzzPromptInput {
        session_id,
        prompt_blocks,
        channel_id,
        channel_type,
        owner_pubkey,
        agent_keys,
        members,
        events,
    } = input;
    if channel_type != "dm" {
        bail!("Buzz P0 accepts owner DMs only");
    }
    let owner_pubkey = normalize_pubkey(owner_pubkey)?;
    let agent_pubkey = agent_keys.public_key().to_hex().to_ascii_lowercase();
    if owner_pubkey == agent_pubkey {
        bail!("owner and managed agent must be distinct identities");
    }
    if events.is_empty() {
        bail!("Buzz delivery must contain at least one signed event");
    }

    let mut members: Vec<Value> = members
        .iter()
        .map(|(pubkey, role)| {
            let pubkey = normalize_pubkey(pubkey)?;
            if !matches!(role.as_str(), "owner" | "admin" | "member") {
                bail!("unsupported Buzz membership role");
            }
            Ok(json!({"pubkey": pubkey, "role": role}))
        })
        .collect::<Result<_>>()?;
    members.sort_by(|left, right| {
        left["pubkey"]
            .as_str()
            .unwrap_or_default()
            .as_bytes()
            .cmp(right["pubkey"].as_str().unwrap_or_default().as_bytes())
    });
    if members.len() != 2
        || !members
            .iter()
            .any(|member| member["pubkey"] == owner_pubkey && member["role"] == "member")
        || !members
            .iter()
            .any(|member| member["pubkey"] == agent_pubkey && member["role"] == "member")
    {
        bail!("Buzz P0 requires the exact owner and managed-agent DM participant set");
    }

    let channel_id = uuid::Uuid::parse_str(channel_id)
        .context("invalid Buzz channel UUID")?
        .to_string();
    for event in events {
        event.verify().context("invalid signed Buzz event")?;
        if event.pubkey.to_hex().to_ascii_lowercase() != owner_pubkey {
            bail!("Buzz P0 accepts only owner-authored inbound events");
        }
        let belongs_to_channel = event.tags.iter().any(|tag| {
            let values = tag.as_slice();
            values.first().map(String::as_str) == Some("h")
                && values.get(1).map(String::as_str) == Some(channel_id.as_str())
        });
        if !belongs_to_channel {
            bail!("signed Buzz event does not belong to the claimed channel");
        }
    }

    let membership_revision = membership_revision(&channel_id, &members)?;
    let channel = json!({
        "id": channel_id,
        "type": "dm",
        "ownerPubkey": owner_pubkey,
        "agentPubkey": agent_pubkey,
        "members": members,
        "membershipRevision": membership_revision,
    });
    let anchor = events
        .last()
        .ok_or_else(|| anyhow!("missing reply anchor"))?;
    let (thread_root, thread_parent) = thread_anchors(anchor);
    let anchor_id = anchor.id.to_hex();
    let reply = json!({
        "rootEventId": thread_root.unwrap_or_else(|| anchor_id.clone()),
        "parentEventId": thread_parent,
        "replyToEventId": anchor_id,
    });
    let event_values = events
        .iter()
        .map(serde_json::to_value)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to serialize signed Buzz events")?;
    let event_ids = events
        .iter()
        .map(|event| Value::String(event.id.to_hex()))
        .collect::<Vec<_>>();
    let delivery_payload = json!({
        "version": 1,
        "channel": channel,
        "reply": reply,
        "eventIds": event_ids,
    });
    let delivery_id = sha256_hex(canonical_json(&delivery_payload)?.as_bytes());
    let mut envelope = json!({
        "version": 1,
        "deliveryId": delivery_id,
        "channel": delivery_payload["channel"],
        "reply": delivery_payload["reply"],
        "events": event_values,
    });

    let mut preimage = ENVELOPE_DOMAIN.to_vec();
    preimage.extend(canonical_json(&envelope)?.as_bytes());
    let digest: [u8; 32] = Sha256::digest(&preimage).into();
    let keypair = Keypair::from_secret_key(SECP256K1, agent_keys.secret_key());
    let signature = SECP256K1.sign_schnorr_no_aux_rand(&Message::from_digest(digest), &keypair);
    envelope["attestation"] = json!({
        "algorithm": "bip340-sha256",
        "signerPubkey": agent_pubkey,
        "payloadHash": hex::encode(digest),
        "signature": signature.to_string(),
    });

    let blocks = prompt_blocks
        .iter()
        .map(|text| json!({"type": "text", "text": text}))
        .collect::<Vec<_>>();
    Ok(json!({
        "sessionId": session_id,
        "prompt": blocks,
        "_meta": {"buzz": envelope},
    }))
}

fn normalize_pubkey(value: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase();
    nostr::PublicKey::from_hex(&normalized).context("invalid Buzz pubkey")?;
    Ok(normalized)
}

fn thread_anchors(event: &Event) -> (Option<String>, Option<String>) {
    let mut root = None;
    let mut parent = None;
    for tag in event.tags.iter() {
        let values = tag.as_slice();
        if values.first().map(String::as_str) != Some("e") || values.len() < 4 {
            continue;
        }
        match values.get(3).map(String::as_str) {
            Some("root") => root = values.get(1).cloned(),
            Some("reply") => parent = values.get(1).cloned(),
            _ => {}
        }
    }
    if root.is_none() {
        root = parent.clone();
    }
    (root, parent)
}

fn membership_revision(channel_id: &str, members: &[Value]) -> Result<String> {
    let payload = json!({
        "version": 1,
        "channelId": channel_id,
        "members": members,
    });
    Ok(format!(
        "v1:{}",
        sha256_hex(canonical_json(&payload)?.as_bytes())
    ))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Canonicalize the integer-only JSON subset used by the Buzz wire contract.
fn canonical_json(value: &Value) -> Result<String> {
    match value {
        Value::Null => Ok("null".to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) if value.is_i64() || value.is_u64() => Ok(value.to_string()),
        Value::Number(_) => bail!("non-integer numbers are not allowed in Buzz canonical JSON"),
        Value::String(value) => serde_json::to_string(value).context("canonical JSON string"),
        Value::Array(values) => {
            let values = values
                .iter()
                .map(canonical_json)
                .collect::<Result<Vec<_>>>()?;
            Ok(format!("[{}]", values.join(",")))
        }
        Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
            let fields = keys
                .into_iter()
                .map(|key| {
                    Ok(format!(
                        "{}:{}",
                        serde_json::to_string(key).context("canonical JSON key")?,
                        canonical_json(&values[key])?
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(format!("{{{}}}", fields.join(",")))
        }
    }
}
