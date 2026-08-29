//! Authenticated Buzz transport metadata for ACP `session/prompt` requests.
//!
//! Rendered prompt text is intentionally not an input to the trusted envelope.
//! The envelope is built only from the already-verified Nostr events and the
//! authoritative channel membership snapshot supplied by the harness.

use anyhow::{anyhow, bail, Context, Result};
use buzz_core::kind::{KIND_JOB_ACCEPTED, KIND_JOB_CANCEL, KIND_JOB_REQUEST, KIND_JOB_RESULT};
use nostr::secp256k1::{Keypair, Message};
use nostr::{Event, Keys, SECP256K1};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const ENVELOPE_DOMAIN: &[u8] = b"buzz-acp-envelope-v1\0";

/// Byte cap on the untrusted `job.title` claim field (rendered into prompts
/// downstream, so it is bounded and control-character-free).
const JOB_TITLE_MAX_BYTES: usize = 512;

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
    let mut chat_events: Vec<&Event> = Vec::new();
    let mut job_event: Option<&Event> = None;
    for event in events {
        event.verify().context("invalid signed Buzz event")?;
        let belongs_to_channel = event.tags.iter().any(|tag| {
            let values = tag.as_slice();
            values.first().map(String::as_str) == Some("h")
                && values.get(1).map(String::as_str) == Some(channel_id.as_str())
        });
        if !belongs_to_channel {
            bail!("signed Buzz event does not belong to the claimed channel");
        }
        let kind = event.kind.as_u16() as u32;
        if kind == 9 {
            if event.pubkey.to_hex().to_ascii_lowercase() != owner_pubkey {
                bail!("Buzz P0 accepts only owner-authored inbound chat events");
            }
            chat_events.push(event);
        } else if job_action(kind).is_some() {
            // Approved scope D3: request/accepted/result/cancel only.
            // Progress (43003) and error (43006) fall through to the bail
            // below. The claim is singular by design — a batch carrying more
            // than one job lifecycle event is ambiguous, so P0 fails the
            // whole delivery closed rather than picking one.
            if job_event.is_some() {
                bail!("Buzz P0 accepts at most one job event per delivery");
            }
            job_event = Some(event);
        } else {
            bail!("Buzz P0 accepts only kind-9 chat events");
        }
    }
    if chat_events.is_empty() {
        // P0 fail-closed: reply routing anchors on the last chat event, and
        // no upstream job-thread reply convention exists yet, so a job-only
        // delivery has no deterministic anchor and is rejected.
        bail!("Buzz delivery must contain at least one kind-9 chat event");
    }
    let job_claim = job_event
        .map(|event| build_job_claim(event, &owner_pubkey, &agent_pubkey))
        .transpose()?;

    let membership_revision = membership_revision(&channel_id, &members)?;
    let channel = json!({
        "id": channel_id,
        "type": "dm",
        "ownerPubkey": owner_pubkey,
        "agentPubkey": agent_pubkey,
        "members": members,
        "membershipRevision": membership_revision,
    });
    let anchor = chat_events
        .last()
        .ok_or_else(|| anyhow!("missing reply anchor"))?;
    let (thread_root, thread_parent) = thread_anchors(anchor)?;
    let anchor_id = anchor.id.to_hex();
    let reply = json!({
        "rootEventId": thread_root.unwrap_or_else(|| anchor_id.clone()),
        "parentEventId": thread_parent,
        "replyToEventId": anchor_id,
    });
    let event_values = chat_events
        .iter()
        .map(serde_json::to_value)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to serialize signed Buzz events")?;
    let event_ids = chat_events
        .iter()
        .map(|event| Value::String(event.id.to_hex()))
        .collect::<Vec<_>>();
    let mut delivery_payload = json!({
        "version": 1,
        "channel": channel,
        "reply": reply,
        "eventIds": event_ids,
    });
    if let Some(job) = &job_claim {
        // The job event never enters `events[]`/`eventIds`, so its id is
        // bound into the deliveryId derivation as an explicit sibling key —
        // a redelivered job batch dedupes identically downstream, and a
        // chat-only delivery's canonical bytes are unchanged (no key added).
        delivery_payload["jobEventId"] = job["eventId"].clone();
    }
    let delivery_id = sha256_hex(canonical_json(&delivery_payload)?.as_bytes());
    let mut envelope = json!({
        "version": 1,
        "deliveryId": delivery_id,
        "channel": delivery_payload["channel"],
        "reply": delivery_payload["reply"],
        "events": event_values,
    });
    if let Some(job) = job_claim {
        // Optional top-level field: present only when the batch carried a job
        // event, so chat-only envelopes stay byte-identical. Inserted before
        // the attestation is computed, so every `job` subfield is covered by
        // the RFC 8785 canonicalization + payloadHash + BIP-340 signature.
        envelope["job"] = job;
    }

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

/// Validate one e-tag event-id anchor: exactly 64 hex characters.
///
/// Uppercase hex is normalized to lowercase; anything else (wrong length,
/// non-hex) rejects the whole event with `bail!` — these values are signed
/// into the attestation as reply routing, so a malformed anchor must never be
/// forwarded or silently dropped.
fn validate_event_id_anchor(value: &str) -> Result<String> {
    let normalized = value.to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("Buzz event carries a malformed e-tag event-id anchor");
    }
    Ok(normalized)
}

/// Map an accepted job kind to its claim `action` string.
///
/// Approved scope D3: request (43001), accepted (43002), result (43004), and
/// cancel (43005) only. Progress (43003) and error (43006) return `None` and
/// stay rejected upstream.
fn job_action(kind: u32) -> Option<&'static str> {
    match kind {
        KIND_JOB_REQUEST => Some("request"),
        KIND_JOB_ACCEPTED => Some("accepted"),
        KIND_JOB_RESULT => Some("result"),
        KIND_JOB_CANCEL => Some("cancel"),
        _ => None,
    }
}

/// Build the structured `job` claim for one verified job lifecycle event.
///
/// Author roles (frozen here — upstream defines the kinds in
/// `buzz-core/src/kind.rs` but nothing produces or consumes them yet, so
/// there is no upstream enforcement to mirror): the owner requests (43001)
/// and cancels (43005) jobs; the managed agent accepts (43002) and delivers
/// results (43004). Any other author fails the delivery closed.
///
/// Trust boundary of the claim: `jobId` and `title` are ASSERTED transport
/// data. The job event's author is signature-verified and role-checked, but
/// the kind-43001 request event that `jobId` names is NOT resolved or
/// verified by the envelope — a signed 43002/43004/43005 may reference any
/// well-formed 64-hex id, including one from another channel or one that
/// does not exist. Downstream consumers must treat `jobId` as a correlation
/// key, not a verified reference. Relay-side resolution is deferred to
/// Phase 2.
fn build_job_claim(event: &Event, owner_pubkey: &str, agent_pubkey: &str) -> Result<Value> {
    let kind = event.kind.as_u16() as u32;
    let action = job_action(kind)
        .ok_or_else(|| anyhow!("unsupported Buzz job kind reached claim construction"))?;
    let author = event.pubkey.to_hex().to_ascii_lowercase();
    let expected_author = match kind {
        KIND_JOB_REQUEST | KIND_JOB_CANCEL => owner_pubkey,
        _ => agent_pubkey,
    };
    if author != expected_author {
        bail!("Buzz job event author does not match the required role for its kind");
    }
    let event_id = validate_event_id_anchor(&event.id.to_hex())?;
    let job_id = if kind == KIND_JOB_REQUEST {
        event_id.clone()
    } else {
        referenced_job_request_id(event)?
    };
    Ok(json!({
        "jobId": job_id,
        "action": action,
        "eventId": event_id,
        "title": job_title(event)?,
        // No upstream due-date tag convention exists for job kinds (nothing
        // produces them yet), so P0 always emits null. The wire slot is
        // reserved as unix-seconds-integer-or-null.
        "dueAt": Value::Null,
    }))
}

/// Resolve the kind-43001 request id that a 43002/43004/43005 event refers to.
///
/// Upstream defines no referencing convention for these kinds (no producer or
/// consumer exists in buzz-core or the desktop beyond generic timeline
/// rendering), so P0 freezes: the first e-tag whose value is a valid 64-hex
/// event id names the job request. Invalid e-tags are skipped during the
/// scan; if no e-tag qualifies the delivery fails closed below.
fn referenced_job_request_id(event: &Event) -> Result<String> {
    for tag in event.tags.iter() {
        let values = tag.as_slice();
        if values.first().map(String::as_str) != Some("e") {
            continue;
        }
        if let Some(candidate) = values.get(1) {
            if let Ok(anchor) = validate_event_id_anchor(candidate) {
                return Ok(anchor);
            }
        }
    }
    bail!("Buzz job event does not reference a job request via a valid 64-hex e-tag");
}

/// Derive the untrusted `title` claim field for a job event.
///
/// No upstream title convention exists for job kinds, so P0 freezes: a
/// `["title", <text>]` tag wins when present; otherwise the first line of the
/// event content is used. The result rejects control/format/separator
/// characters — see [`is_forbidden_title_char`] — because it will later be
/// rendered into prompts downstream, and is capped at
/// [`JOB_TITLE_MAX_BYTES`] on a UTF-8 char boundary. May be empty.
fn job_title(event: &Event) -> Result<String> {
    let tagged = event.tags.iter().find_map(|tag| {
        let values = tag.as_slice();
        if values.first().map(String::as_str) == Some("title") {
            values.get(1).cloned()
        } else {
            None
        }
    });
    let raw = match tagged {
        Some(value) => value,
        None => event.content.lines().next().unwrap_or_default().to_string(),
    };
    if raw.chars().any(is_forbidden_title_char) {
        bail!("Buzz job title contains control, format, or separator characters");
    }
    Ok(truncate_utf8(raw, JOB_TITLE_MAX_BYTES))
}

/// True for characters that must never appear in an attested job title:
/// Unicode general categories Cc (control), Cf (format — bidi overrides,
/// zero-width characters, isolates, BOM), Zl (LINE SEPARATOR), and Zp
/// (PARAGRAPH SEPARATOR). `char::is_control` covers only Cc; Cf/Zl/Zp would
/// otherwise pass fail-open and reorder, hide, or line-split the title when
/// it is rendered into prompts downstream (U+2028 also survives
/// `str::lines()`). Cf/Zl/Zp are matched dependency-free by explicit range;
/// the table mirrors Unicode 16.0 `DerivedGeneralCategory.txt`.
fn is_forbidden_title_char(c: char) -> bool {
    if c.is_control() {
        return true; // Cc
    }
    matches!(
        c,
        '\u{00AD}' // SOFT HYPHEN
        | '\u{0600}'..='\u{0605}' // Arabic number signs
        | '\u{061C}' // ARABIC LETTER MARK
        | '\u{06DD}'
        | '\u{070F}'
        | '\u{0890}'..='\u{0891}'
        | '\u{08E2}'
        | '\u{180E}' // MONGOLIAN VOWEL SEPARATOR
        | '\u{200B}'..='\u{200F}' // zero-width chars, LRM/RLM
        | '\u{2028}'..='\u{202E}' // Zl, Zp, then bidi embeddings/overrides
        | '\u{2060}'..='\u{2064}' // WORD JOINER..INVISIBLE PLUS
        | '\u{2066}'..='\u{206F}' // isolates + deprecated format chars
        | '\u{FEFF}' // ZERO WIDTH NO-BREAK SPACE / BOM
        | '\u{FFF9}'..='\u{FFFB}' // interlinear annotation controls
        | '\u{110BD}'
        | '\u{110CD}'
        | '\u{13430}'..='\u{1343F}' // Egyptian hieroglyph format controls
        | '\u{1BCA0}'..='\u{1BCA3}' // shorthand format controls
        | '\u{1D173}'..='\u{1D17A}' // musical symbol controls
        | '\u{E0001}' // LANGUAGE TAG
        | '\u{E0020}'..='\u{E007F}' // tag characters
    )
}

/// Truncate a string to at most `max_bytes` bytes on a UTF-8 char boundary.
fn truncate_utf8(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}

fn thread_anchors(event: &Event) -> Result<(Option<String>, Option<String>)> {
    let mut root = None;
    let mut parent = None;
    for tag in event.tags.iter() {
        let values = tag.as_slice();
        if values.first().map(String::as_str) != Some("e") {
            continue;
        }
        // Fail closed (P0): an e-tag we cannot fully interpret — including the
        // legacy positional `["e", <id>]` form without a NIP-10 marker — could
        // silently change the thread routing that gets signed into the
        // attestation, so reject the event instead of dropping the anchor.
        if values.len() < 4 {
            bail!("Buzz event carries an e-tag without a NIP-10 marker; rejecting ambiguous thread anchor");
        }
        let anchor = validate_event_id_anchor(
            values
                .get(1)
                .ok_or_else(|| anyhow!("Buzz event e-tag is missing an event id"))?,
        )?;
        match values.get(3).map(String::as_str) {
            Some("root") => root = Some(anchor),
            Some("reply") => parent = Some(anchor),
            // Non-anchor markers (e.g. "mention") are validated above but do
            // not participate in reply routing.
            _ => {}
        }
    }
    if root.is_none() {
        root = parent.clone();
    }
    Ok((root, parent))
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
pub(crate) fn canonical_json(value: &Value) -> Result<String> {
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
