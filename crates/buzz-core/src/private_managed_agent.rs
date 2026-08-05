//! NIP-PMA private managed-agent wire codec.
//!
//! This module defines and validates the inert wire format only. Relays must
//! not accept [`KIND_PRIVATE_MANAGED_AGENT`](crate::kind::KIND_PRIVATE_MANAGED_AGENT)
//! until the dedicated privacy and aggregate-CAS transactions are deployed.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::str::FromStr;

use nostr::nips::nip44::{self, Version};
use nostr::secp256k1::schnorr::Signature;
use nostr::secp256k1::Message;
use nostr::{Event, EventBuilder, EventId, Keys, Kind, PublicKey, Tag, SECP256K1};
use serde::de::{DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::kind::{KIND_MANAGED_AGENT, KIND_PERSONA, KIND_PRIVATE_MANAGED_AGENT};

/// Wire-format discriminator for decrypted private managed-agent payloads.
pub const FORMAT: &str = "buzz-private-managed-agent";
/// Current decrypted payload schema version.
pub const VERSION: u32 = 1;
/// NIP-44 v2 plaintext limit.
pub const MAX_PLAINTEXT_BYTES: usize = 65_535;
/// Maximum plausible NIP-44 v2 ciphertext length.
pub const MAX_CIPHERTEXT_BYTES: usize = 87_472;
/// Largest integer represented exactly by interoperable JSON implementations.
pub const MAX_SAFE_GENERATION: u64 = (1_u64 << 53) - 1;
/// Maximum number of environment variables in one private payload.
pub const MAX_ENV_VARS: usize = 256;
/// Maximum UTF-8 bytes in one environment-variable key.
pub const MAX_ENV_KEY_BYTES: usize = 256;
/// Maximum UTF-8 bytes in one environment-variable value.
pub const MAX_ENV_VALUE_BYTES: usize = 16_384;
/// Maximum number of explicit agent arguments.
pub const MAX_AGENT_ARGS: usize = 256;
/// Maximum UTF-8 bytes in one argument.
pub const MAX_AGENT_ARG_BYTES: usize = 8_192;
/// Maximum serialized bytes accepted for an extension/recovery/config value.
pub const MAX_VALUE_BYTES: usize = 32_768;

/// Errors returned by the private managed-agent codec.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    /// The signed outer event is malformed or does not match the expected owner.
    #[error("invalid private managed-agent envelope: {0}")]
    InvalidEnvelope(String),
    /// The ciphertext could not be authenticated/decrypted. Deliberately redacted.
    #[error("private managed-agent payload could not be decrypted")]
    Decrypt,
    /// The decrypted JSON is malformed, ambiguous, or semantically invalid.
    #[error("invalid private managed-agent payload: {0}")]
    InvalidPayload(String),
    /// Encryption failed.
    #[error("private managed-agent encryption failed")]
    Encrypt,
    /// Event signing failed.
    #[error("private managed-agent signing failed")]
    Sign,
}

/// Authoritative lifecycle state repeated in the outer tags and ciphertext.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// Runnable aggregate.
    Active,
    /// Anti-resurrection tombstone.
    Deleted,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Deleted => "deleted",
        }
    }
}

/// Versioned signed-event recovery material for a bound public projection.
///
/// Retaining the complete signed event makes reconstruction unambiguous: its
/// signature, ID, author, kind, coordinate, and exact content bytes can all be
/// checked without trusting replaceable-event history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionRecoveryV1 {
    /// Recovery schema version. Version 1 stores one complete signed event.
    pub version: u32,
    /// Exact signed public projection event.
    pub signed_event: Event,
}

/// Complete definition projection binding and recovery material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefinitionBinding {
    /// CAS-managed definition revision pinned by this aggregate.
    pub revision: u64,
    /// Exact signed kind:30175 event ID.
    pub event_id: String,
    /// Lowercase SHA-256 of the exact projection content bytes.
    pub content_sha256: String,
    /// Versioned signed event sufficient to reproduce the projection.
    pub recovery: ProjectionRecoveryV1,
}

/// Complete kind:30177 projection binding and recovery material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceBinding {
    /// Exact signed kind:30177 event ID.
    pub event_id: String,
    /// Lowercase SHA-256 of the exact projection content bytes.
    pub content_sha256: String,
    /// Versioned signed event sufficient to reproduce the projection.
    pub recovery: ProjectionRecoveryV1,
}

/// Secret agent identity material. It never appears in public projections.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateIdentity {
    /// Agent private key in nsec form.
    pub private_key_nsec: String,
    /// Optional NIP-OA owner attestation JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_tag: Option<String>,
}

impl fmt::Debug for PrivateIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivateIdentity")
            .field("private_key_nsec", &"<redacted>")
            .field("auth_tag", &self.auth_tag.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Portable private runnable configuration.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateConfig {
    /// Explicit kind:30175 coordinate, when definition-backed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_coordinate: Option<String>,
    /// Intended relay endpoint; validated again on each device before use.
    pub relay_url: String,
    /// Explicit harness override; never launched without local validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_command_override: Option<String>,
    /// Explicit harness arguments; validated again on each device.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_args: Vec<String>,
    /// Idle timeout in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_timeout_seconds: Option<u64>,
    /// Absolute turn timeout in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turn_duration_seconds: Option<u64>,
    /// Secret environment overrides.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env_vars: BTreeMap<String, String>,
    /// Versioned backend configuration. Device/provider validation is required.
    pub backend: Value,
    /// Durable remote backend identity; ownership/existence is device-validated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_agent_id: Option<String>,
    /// Portable team linkage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// Portable identity within a team.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_name_in_team: Option<String>,
    /// Versioned provider/definition relay-mesh marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_mesh: Option<Value>,
}

impl fmt::Debug for PrivateConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivateConfig")
            .field("contents", &"<redacted>")
            .finish()
    }
}

/// Fields present only when [`Payload::state`] is [`State::Active`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivePayload {
    /// Exact definition projection binding.
    pub definition: DefinitionBinding,
    /// Exact public instance projection binding.
    pub instance_projection: InstanceBinding,
    /// Secret identity material.
    pub identity: PrivateIdentity,
    /// Private portable/device-validated configuration.
    pub config: PrivateConfig,
}

/// Decrypted private managed-agent payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    /// Always [`FORMAT`].
    pub format: String,
    /// Always [`VERSION`].
    pub version: u32,
    /// Agent pubkey and event `d` coordinate.
    pub agent_pubkey: String,
    /// Owner pubkey and signed event author.
    pub owner_pubkey: String,
    /// Monotonic CAS generation.
    pub generation: u64,
    /// Exact predecessor event ID; absent only for generation one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_event_id: Option<String>,
    /// Lifecycle state, repeated in the outer `state` tag.
    pub state: State,
    /// RFC3339 bookkeeping timestamp; never used for conflict resolution.
    pub updated_at: String,
    /// Required for active records and forbidden for tombstones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<ActivePayload>,
    /// Required for tombstones and forbidden for active records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
    /// Forward-compatible namespaced data. Core semantics must never depend on it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
}

/// Validated public metadata from a private managed-agent event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// Agent pubkey from `d`.
    pub agent_pubkey: PublicKey,
    /// Owner pubkey from the signed event author.
    pub owner_pubkey: PublicKey,
    /// CAS generation from `g`.
    pub generation: u64,
    /// CAS predecessor from `prev`.
    pub previous_event_id: Option<EventId>,
    /// Lifecycle state from `state`.
    pub state: State,
}

/// Compute the lowercase SHA-256 binding for exact projection content bytes.
pub fn content_sha256(content: &[u8]) -> String {
    hex::encode(Sha256::digest(content))
}

/// Validate a signed outer envelope before any decryption.
pub fn validate_envelope(event: &Event, expected_owner: &PublicKey) -> Result<Envelope, Error> {
    if event.kind.as_u16() as u32 != KIND_PRIVATE_MANAGED_AGENT {
        return Err(Error::InvalidEnvelope("wrong kind".into()));
    }
    if &event.pubkey != expected_owner {
        return Err(Error::InvalidEnvelope(
            "author is not expected owner".into(),
        ));
    }
    if !event.verify_id() || !event.verify_signature() {
        return Err(Error::InvalidEnvelope(
            "invalid event id or signature".into(),
        ));
    }
    if event.content.is_empty() || event.content.len() > MAX_CIPHERTEXT_BYTES {
        return Err(Error::InvalidEnvelope("invalid ciphertext length".into()));
    }

    let mut d = None;
    let mut g = None;
    let mut prev = None;
    let mut state = None;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.len() != 2 {
            return Err(Error::InvalidEnvelope(
                "every tag must have exactly one value".into(),
            ));
        }
        let slot = match parts[0].as_str() {
            "d" => &mut d,
            "g" => &mut g,
            "prev" => &mut prev,
            "state" => &mut state,
            name => return Err(Error::InvalidEnvelope(format!("unexpected tag: {name}"))),
        };
        if slot.replace(parts[1].clone()).is_some() {
            return Err(Error::InvalidEnvelope(format!(
                "duplicate {} tag",
                parts[0]
            )));
        }
    }

    let agent_pubkey = parse_canonical_pubkey(
        "d",
        d.as_deref()
            .ok_or_else(|| Error::InvalidEnvelope("missing d tag".into()))?,
    )?;
    let owner_pubkey = *expected_owner;
    let generation = parse_generation(
        g.as_deref()
            .ok_or_else(|| Error::InvalidEnvelope("missing g tag".into()))?,
    )?;
    let previous_event_id = match prev {
        Some(value) => Some(parse_event_id("prev", &value)?),
        None => None,
    };
    if (generation == 1) != previous_event_id.is_none() {
        return Err(Error::InvalidEnvelope(
            "prev must be absent exactly at generation 1".into(),
        ));
    }
    let state = match state.as_deref() {
        Some("active") => State::Active,
        Some("deleted") => State::Deleted,
        Some(_) => return Err(Error::InvalidEnvelope("invalid state tag".into())),
        None => return Err(Error::InvalidEnvelope("missing state tag".into())),
    };
    Ok(Envelope {
        agent_pubkey,
        owner_pubkey,
        generation,
        previous_event_id,
        state,
    })
}

/// Encrypt and sign an inert private managed-agent event candidate.
pub fn build_event(owner_keys: &Keys, payload: &Payload, created_at: u64) -> Result<Event, Error> {
    validate_payload(payload)?;
    if payload.owner_pubkey != owner_keys.public_key().to_hex() {
        return Err(Error::InvalidPayload(
            "owner_pubkey does not match signing key".into(),
        ));
    }
    let plaintext = serde_json::to_vec(payload).map_err(|_| Error::Encrypt)?;
    if plaintext.len() > MAX_PLAINTEXT_BYTES {
        return Err(Error::InvalidPayload(
            "plaintext exceeds NIP-44 limit".into(),
        ));
    }
    let plaintext = std::str::from_utf8(&plaintext).map_err(|_| Error::Encrypt)?;
    let ciphertext = nip44::encrypt(
        owner_keys.secret_key(),
        &owner_keys.public_key(),
        plaintext,
        Version::V2,
    )
    .map_err(|_| Error::Encrypt)?;
    let mut tags = vec![
        parse_tag(["d", payload.agent_pubkey.as_str()])?,
        parse_tag(["g", payload.generation.to_string().as_str()])?,
        parse_tag(["state", payload.state.as_str()])?,
    ];
    if let Some(previous) = payload.previous_event_id.as_deref() {
        tags.push(parse_tag(["prev", previous])?);
    }
    EventBuilder::new(Kind::Custom(KIND_PRIVATE_MANAGED_AGENT as u16), ciphertext)
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from(created_at))
        .sign_with_keys(owner_keys)
        .map_err(|_| Error::Sign)
}

/// Validate, owner-self decrypt, strictly parse, and cross-check a payload.
pub fn validate_and_decrypt(
    event: &Event,
    owner_keys: &Keys,
) -> Result<(Envelope, Payload), Error> {
    let envelope = validate_envelope(event, &owner_keys.public_key())?;
    let plaintext = nip44::decrypt(
        owner_keys.secret_key(),
        &owner_keys.public_key(),
        &event.content,
    )
    .map_err(|_| Error::Decrypt)?;
    if plaintext.len() > MAX_PLAINTEXT_BYTES {
        return Err(Error::Decrypt);
    }
    let value = parse_strict_json(plaintext.as_bytes())?;
    let payload: Payload =
        serde_json::from_value(value).map_err(|e| Error::InvalidPayload(format!("schema: {e}")))?;
    validate_payload(&payload)?;
    if payload.agent_pubkey != envelope.agent_pubkey.to_hex()
        || payload.owner_pubkey != envelope.owner_pubkey.to_hex()
        || payload.generation != envelope.generation
        || payload.state != envelope.state
        || payload.previous_event_id.as_deref()
            != envelope
                .previous_event_id
                .as_ref()
                .map(EventId::to_hex)
                .as_deref()
    {
        return Err(Error::InvalidPayload(
            "outer/inner metadata mismatch".into(),
        ));
    }
    Ok((envelope, payload))
}

/// Validate decrypted payload semantics independently of encryption.
pub fn validate_payload(payload: &Payload) -> Result<(), Error> {
    if payload.format != FORMAT || payload.version != VERSION {
        return Err(Error::InvalidPayload(
            "unsupported format or version".into(),
        ));
    }
    let agent = parse_canonical_pubkey("agent_pubkey", &payload.agent_pubkey)
        .map_err(|e| Error::InvalidPayload(e.to_string()))?;
    parse_canonical_pubkey("owner_pubkey", &payload.owner_pubkey)
        .map_err(|e| Error::InvalidPayload(e.to_string()))?;
    validate_generation_and_prev(payload.generation, payload.previous_event_id.as_deref())?;
    parse_rfc3339("updated_at", &payload.updated_at)?;
    for (key, value) in &payload.extensions {
        if key.is_empty() || key.len() > 128 || !key.contains(':') {
            return Err(Error::InvalidPayload(
                "extension keys must be non-empty namespaced strings <= 128 bytes".into(),
            ));
        }
        validate_value_size("extension", value)?;
    }
    match payload.state {
        State::Active => {
            if payload.deleted_at.is_some() {
                return Err(Error::InvalidPayload(
                    "active payload must not contain deleted_at".into(),
                ));
            }
            let active = payload.active.as_ref().ok_or_else(|| {
                Error::InvalidPayload("active payload missing active body".into())
            })?;
            validate_active(active, &agent, &payload.owner_pubkey)?;
        }
        State::Deleted => {
            if payload.active.is_some() {
                return Err(Error::InvalidPayload(
                    "deleted payload must not contain active body".into(),
                ));
            }
            parse_rfc3339(
                "deleted_at",
                payload.deleted_at.as_deref().ok_or_else(|| {
                    Error::InvalidPayload("deleted payload missing deleted_at".into())
                })?,
            )?;
        }
    }
    Ok(())
}

fn validate_active(
    active: &ActivePayload,
    agent: &PublicKey,
    owner_pubkey: &str,
) -> Result<(), Error> {
    if active.definition.revision == 0 || active.definition.revision > MAX_SAFE_GENERATION {
        return Err(Error::InvalidPayload("invalid definition revision".into()));
    }
    let definition_d =
        parse_definition_coordinate(active.config.definition_coordinate.as_deref(), owner_pubkey)?;
    validate_binding(
        "definition",
        KIND_PERSONA,
        owner_pubkey,
        Some(&definition_d),
        &active.definition.event_id,
        &active.definition.content_sha256,
        &active.definition.recovery,
    )?;
    validate_binding(
        "instance_projection",
        KIND_MANAGED_AGENT,
        owner_pubkey,
        Some(&agent.to_hex()),
        &active.instance_projection.event_id,
        &active.instance_projection.content_sha256,
        &active.instance_projection.recovery,
    )?;
    let agent_keys = Keys::parse(active.identity.private_key_nsec.trim())
        .map_err(|_| Error::InvalidPayload("invalid agent nsec".into()))?;
    if agent_keys.public_key() != *agent {
        return Err(Error::InvalidPayload(
            "agent nsec does not derive agent_pubkey".into(),
        ));
    }
    if let Some(auth_tag) = &active.identity.auth_tag {
        validate_auth_tag(auth_tag, owner_pubkey, agent)?;
    }
    let config = &active.config;
    if config.relay_url.is_empty() || config.relay_url.len() > 4096 {
        return Err(Error::InvalidPayload("invalid relay_url length".into()));
    }
    if config.agent_args.len() > MAX_AGENT_ARGS
        || config
            .agent_args
            .iter()
            .any(|arg| arg.len() > MAX_AGENT_ARG_BYTES)
    {
        return Err(Error::InvalidPayload("agent_args exceed limits".into()));
    }
    if config.env_vars.len() > MAX_ENV_VARS
        || config.env_vars.iter().any(|(k, v)| {
            k.is_empty() || k.len() > MAX_ENV_KEY_BYTES || v.len() > MAX_ENV_VALUE_BYTES
        })
    {
        return Err(Error::InvalidPayload("env_vars exceed limits".into()));
    }
    validate_value_size("backend", &config.backend)?;
    if let Some(mesh) = &config.relay_mesh {
        validate_value_size("relay_mesh", mesh)?;
    }
    Ok(())
}

fn validate_auth_tag(auth_tag: &str, expected_owner: &str, agent: &PublicKey) -> Result<(), Error> {
    if auth_tag.is_empty() || auth_tag.len() > 4096 {
        return Err(Error::InvalidPayload("invalid auth_tag".into()));
    }
    let parts: Vec<String> = serde_json::from_str(auth_tag)
        .map_err(|_| Error::InvalidPayload("invalid auth_tag".into()))?;
    if parts.len() != 4 || parts[0] != "auth" || parts[1] != expected_owner || !parts[2].is_empty()
    {
        return Err(Error::InvalidPayload(
            "auth_tag must be an unconditional attestation for this owner".into(),
        ));
    }
    parse_canonical_pubkey("auth_tag owner", &parts[1])
        .map_err(|_| Error::InvalidPayload("invalid auth_tag".into()))?;
    if agent.to_hex() == expected_owner {
        return Err(Error::InvalidPayload(
            "auth_tag must attest a distinct agent key".into(),
        ));
    }
    if parts[3].len() != 128
        || !parts[3]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(Error::InvalidPayload("invalid auth_tag".into()));
    }
    let signature = Signature::from_str(&parts[3])
        .map_err(|_| Error::InvalidPayload("invalid auth_tag".into()))?;
    let preimage = format!("nostr:agent-auth:{}:", agent.to_hex());
    let digest = Sha256::digest(preimage.as_bytes());
    let message = Message::from_digest(digest.into());
    let owner = PublicKey::from_hex(&parts[1])
        .map_err(|_| Error::InvalidPayload("invalid auth_tag".into()))?;
    let owner = owner
        .xonly()
        .map_err(|_| Error::InvalidPayload("invalid auth_tag".into()))?;
    SECP256K1
        .verify_schnorr(&signature, &message, &owner)
        .map_err(|_| Error::InvalidPayload("invalid auth_tag signature".into()))
}

fn parse_definition_coordinate(
    coordinate: Option<&str>,
    owner_pubkey: &str,
) -> Result<String, Error> {
    let coordinate = coordinate.ok_or_else(|| {
        Error::InvalidPayload("active payload missing definition_coordinate".into())
    })?;
    let mut parts = coordinate.splitn(3, ':');
    let kind = parts.next();
    let owner = parts.next();
    let d = parts.next();
    if kind != Some("30175") || owner != Some(owner_pubkey) || d.is_none_or(str::is_empty) {
        return Err(Error::InvalidPayload(
            "definition_coordinate must be 30175:<owner>:<non-empty d>".into(),
        ));
    }
    Ok(d.unwrap().to_owned())
}

fn validate_binding(
    label: &str,
    expected_kind: u32,
    owner_pubkey: &str,
    expected_d: Option<&str>,
    event_id: &str,
    hash: &str,
    recovery: &ProjectionRecoveryV1,
) -> Result<(), Error> {
    parse_event_id(label, event_id).map_err(|e| Error::InvalidPayload(e.to_string()))?;
    parse_lower_hex_32(&format!("{label}.content_sha256"), hash)
        .map_err(|e| Error::InvalidPayload(e.to_string()))?;
    if recovery.version != 1 {
        return Err(Error::InvalidPayload(format!(
            "unsupported {label} recovery version"
        )));
    }
    let event = &recovery.signed_event;
    if !event.verify_id() || !event.verify_signature() {
        return Err(Error::InvalidPayload(format!(
            "invalid {label} recovery event"
        )));
    }
    if event.id.to_hex() != event_id
        || event.kind.as_u16() as u32 != expected_kind
        || event.pubkey.to_hex() != owner_pubkey
        || content_sha256(event.content.as_bytes()) != hash
    {
        return Err(Error::InvalidPayload(format!(
            "{label} recovery does not match binding"
        )));
    }
    let d_tags: Vec<_> = event
        .tags
        .iter()
        .filter_map(|tag| {
            let parts = tag.as_slice();
            (parts.first().map(String::as_str) == Some("d")).then_some(parts)
        })
        .collect();
    if d_tags.len() != 1 || d_tags[0].len() != 2 || d_tags[0][1].is_empty() {
        return Err(Error::InvalidPayload(format!(
            "{label} recovery must have exactly one non-empty d tag"
        )));
    }
    if expected_d.is_some_and(|expected| d_tags[0][1] != expected) {
        return Err(Error::InvalidPayload(format!(
            "{label} recovery has wrong coordinate"
        )));
    }
    validate_value_size(
        label,
        &serde_json::to_value(recovery)
            .map_err(|_| Error::InvalidPayload(format!("invalid {label}")))?,
    )
}

fn validate_generation_and_prev(generation: u64, previous: Option<&str>) -> Result<(), Error> {
    if generation == 0 || generation > MAX_SAFE_GENERATION {
        return Err(Error::InvalidPayload(
            "generation must be a positive safe integer".into(),
        ));
    }
    if (generation == 1) != previous.is_none() {
        return Err(Error::InvalidPayload(
            "previous_event_id must be absent exactly at generation 1".into(),
        ));
    }
    if let Some(value) = previous {
        parse_event_id("previous_event_id", value)
            .map_err(|e| Error::InvalidPayload(e.to_string()))?;
    }
    Ok(())
}

fn validate_value_size(label: &str, value: &Value) -> Result<(), Error> {
    let len = serde_json::to_vec(value)
        .map_err(|_| Error::InvalidPayload(format!("invalid {label}")))?
        .len();
    if len > MAX_VALUE_BYTES {
        return Err(Error::InvalidPayload(format!("{label} exceeds size limit")));
    }
    Ok(())
}

fn parse_rfc3339(label: &str, value: &str) -> Result<(), Error> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|_| Error::InvalidPayload(format!("{label} must be RFC3339")))
}

fn parse_generation(value: &str) -> Result<u64, Error> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(Error::InvalidEnvelope("g must be canonical decimal".into()));
    }
    let generation = value
        .parse::<u64>()
        .map_err(|_| Error::InvalidEnvelope("invalid g tag".into()))?;
    if generation == 0 || generation > MAX_SAFE_GENERATION {
        return Err(Error::InvalidEnvelope(
            "g must be a positive safe integer".into(),
        ));
    }
    Ok(generation)
}

fn parse_canonical_pubkey(label: &str, value: &str) -> Result<PublicKey, Error> {
    parse_lower_hex_32(label, value)?;
    let key = PublicKey::from_hex(value)
        .map_err(|_| Error::InvalidEnvelope(format!("invalid {label}")))?;
    key.xonly()
        .map_err(|_| Error::InvalidEnvelope(format!("invalid {label} curve point")))?;
    Ok(key)
}

fn parse_event_id(label: &str, value: &str) -> Result<EventId, Error> {
    parse_lower_hex_32(label, value)?;
    EventId::from_hex(value).map_err(|_| Error::InvalidEnvelope(format!("invalid {label}")))
}

fn parse_lower_hex_32(label: &str, value: &str) -> Result<(), Error> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(Error::InvalidEnvelope(format!(
            "{label} must be 64 lowercase hex chars"
        )));
    }
    Ok(())
}

fn parse_tag<const N: usize>(parts: [&str; N]) -> Result<Tag, Error> {
    Tag::parse(parts).map_err(|_| Error::InvalidEnvelope("failed to build tag".into()))
}

fn parse_strict_json(bytes: &[u8]) -> Result<Value, Error> {
    struct StrictValue;
    impl<'de> DeserializeSeed<'de> for StrictValue {
        type Value = Value;
        fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
            d.deserialize_any(self)
        }
    }
    impl<'de> Visitor<'de> for StrictValue {
        type Value = Value;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("valid JSON with unique object keys")
        }
        fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
            Ok(Value::Bool(v))
        }
        fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
            Ok(Value::Number(v.into()))
        }
        fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
            Ok(Value::Number(v.into()))
        }
        fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Value, E> {
            serde_json::Number::from_f64(v)
                .map(Value::Number)
                .ok_or_else(|| E::custom("non-finite float"))
        }
        fn visit_str<E>(self, v: &str) -> Result<Value, E> {
            Ok(Value::String(v.to_owned()))
        }
        fn visit_string<E>(self, v: String) -> Result<Value, E> {
            Ok(Value::String(v))
        }
        fn visit_unit<E>(self) -> Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_none<E>(self) -> Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
            d.deserialize_any(self)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
            let mut out = Vec::new();
            while let Some(value) = seq.next_element_seed(StrictValue)? {
                out.push(value);
            }
            Ok(Value::Array(out))
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
            let mut seen = HashSet::new();
            let mut out = serde_json::Map::new();
            while let Some(key) = map.next_key::<String>()? {
                if !seen.insert(key.clone()) {
                    return Err(serde::de::Error::custom(format!("duplicate key: {key}")));
                }
                out.insert(key, map.next_value_seed(StrictValue)?);
            }
            Ok(Value::Object(out))
        }
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = StrictValue
        .deserialize(&mut deserializer)
        .map_err(|e| Error::InvalidPayload(e.to_string()))?;
    deserializer
        .end()
        .map_err(|e| Error::InvalidPayload(e.to_string()))?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::ToBech32;

    fn auth_tag(owner: &Keys, agent: &Keys) -> String {
        let preimage = format!("nostr:agent-auth:{}:", agent.public_key().to_hex());
        let digest = Sha256::digest(preimage.as_bytes());
        let signature = owner.sign_schnorr(&Message::from_digest(digest.into()));
        serde_json::json!([
            "auth",
            owner.public_key().to_hex(),
            "",
            signature.to_string()
        ])
        .to_string()
    }

    fn payload(owner: &Keys, agent: &Keys) -> Payload {
        let definition_event = EventBuilder::new(Kind::Custom(KIND_PERSONA as u16), "definition")
            .tags(vec![Tag::parse(["d", "test-agent"]).unwrap()])
            .custom_created_at(nostr::Timestamp::from(1_785_780_000))
            .sign_with_keys(owner)
            .unwrap();
        let instance_event = EventBuilder::new(Kind::Custom(KIND_MANAGED_AGENT as u16), "instance")
            .tags(vec![Tag::parse([
                "d",
                agent.public_key().to_hex().as_str(),
            ])
            .unwrap()])
            .custom_created_at(nostr::Timestamp::from(1_785_780_000))
            .sign_with_keys(owner)
            .unwrap();
        Payload {
            format: FORMAT.into(),
            version: VERSION,
            agent_pubkey: agent.public_key().to_hex(),
            owner_pubkey: owner.public_key().to_hex(),
            generation: 1,
            previous_event_id: None,
            state: State::Active,
            updated_at: "2026-08-03T18:00:00Z".into(),
            active: Some(ActivePayload {
                definition: DefinitionBinding {
                    revision: 1,
                    event_id: definition_event.id.to_hex(),
                    content_sha256: content_sha256(definition_event.content.as_bytes()),
                    recovery: ProjectionRecoveryV1 {
                        version: 1,
                        signed_event: definition_event,
                    },
                },
                instance_projection: InstanceBinding {
                    event_id: instance_event.id.to_hex(),
                    content_sha256: content_sha256(instance_event.content.as_bytes()),
                    recovery: ProjectionRecoveryV1 {
                        version: 1,
                        signed_event: instance_event,
                    },
                },
                identity: PrivateIdentity {
                    private_key_nsec: agent.secret_key().to_bech32().unwrap(),
                    auth_tag: None,
                },
                config: PrivateConfig {
                    definition_coordinate: Some(format!(
                        "30175:{}:test-agent",
                        owner.public_key().to_hex()
                    )),
                    relay_url: "wss://relay.example".into(),
                    agent_command_override: None,
                    agent_args: vec![],
                    idle_timeout_seconds: Some(300),
                    max_turn_duration_seconds: None,
                    env_vars: BTreeMap::from([("SECRET".into(), "not-public".into())]),
                    backend: serde_json::json!({"type": "local"}),
                    backend_agent_id: None,
                    team_id: None,
                    persona_name_in_team: None,
                    relay_mesh: None,
                },
            }),
            deleted_at: None,
            extensions: BTreeMap::new(),
        }
    }

    #[test]
    fn owner_self_round_trip_binds_outer_and_inner() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let expected = payload(&owner, &agent);
        let event = build_event(&owner, &expected, 1_785_780_000).unwrap();
        let (envelope, actual) = validate_and_decrypt(&event, &owner).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(envelope.agent_pubkey, agent.public_key());
        assert_eq!(envelope.owner_pubkey, owner.public_key());
        assert_eq!(envelope.generation, 1);
        assert_eq!(envelope.state, State::Active);
    }

    #[test]
    fn debug_output_redacts_private_material() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let mut candidate = payload(&owner, &agent);
        let private_key_nsec = candidate
            .active
            .as_ref()
            .unwrap()
            .identity
            .private_key_nsec
            .clone();
        let active = candidate.active.as_mut().unwrap();
        active.identity.auth_tag = Some("secret-auth-tag".into());
        active.config.backend = serde_json::json!({"token": "secret-backend-token"});

        let debug = format!("{candidate:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains(&private_key_nsec));
        assert!(!debug.contains("secret-auth-tag"));
        assert!(!debug.contains("not-public"));
        assert!(!debug.contains("secret-backend-token"));
    }

    #[test]
    fn wrong_owner_and_tampering_fail_closed() {
        let owner = Keys::generate();
        let event =
            build_event(&owner, &payload(&owner, &Keys::generate()), 1_785_780_000).unwrap();
        let stranger = Keys::generate();
        assert!(matches!(
            validate_and_decrypt(&event, &stranger),
            Err(Error::InvalidEnvelope(_))
        ));

        let mut tampered = event;
        tampered.content.push('A');
        assert!(matches!(
            validate_and_decrypt(&tampered, &owner),
            Err(Error::InvalidEnvelope(_))
        ));
    }

    #[test]
    fn duplicate_and_unknown_json_fields_are_rejected() {
        let duplicate = br#"{"format":"a","format":"b"}"#;
        assert!(matches!(
            parse_strict_json(duplicate),
            Err(Error::InvalidPayload(message)) if message.contains("duplicate key")
        ));

        let owner = Keys::generate();
        let agent = Keys::generate();
        let mut value = serde_json::to_value(payload(&owner, &agent)).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("surprise".into(), Value::Bool(true));
        let err = serde_json::from_value::<Payload>(value).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn auth_tag_must_be_unconditional_and_bound_to_owner_and_agent() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let mut candidate = payload(&owner, &agent);
        candidate.active.as_mut().unwrap().identity.auth_tag = Some(auth_tag(&owner, &agent));
        validate_payload(&candidate).unwrap();

        candidate.active.as_mut().unwrap().identity.auth_tag =
            Some(auth_tag(&Keys::generate(), &agent));
        assert!(validate_payload(&candidate).is_err());

        candidate.active.as_mut().unwrap().identity.auth_tag =
            Some(auth_tag(&owner, &Keys::generate()));
        assert!(validate_payload(&candidate).is_err());

        let mut self_attested = payload(&owner, &owner);
        self_attested.active.as_mut().unwrap().identity.auth_tag = Some(auth_tag(&owner, &owner));
        assert!(matches!(
            validate_payload(&self_attested),
            Err(Error::InvalidPayload(message)) if message.contains("distinct agent key")
        ));

        let valid = auth_tag(&owner, &agent);
        let mut parts: Vec<String> = serde_json::from_str(&valid).unwrap();
        parts[2] = "kind=9".into();
        candidate.active.as_mut().unwrap().identity.auth_tag =
            Some(serde_json::to_string(&parts).unwrap());
        assert!(validate_payload(&candidate).is_err());
    }

    #[test]
    fn active_identity_must_derive_coordinate() {
        let owner = Keys::generate();
        let mut candidate = payload(&owner, &Keys::generate());
        candidate.active.as_mut().unwrap().identity.private_key_nsec =
            Keys::generate().secret_key().to_bech32().unwrap();
        assert!(matches!(
            validate_payload(&candidate),
            Err(Error::InvalidPayload(message)) if message.contains("does not derive")
        ));
    }

    #[test]
    fn tombstone_requires_successor_shape() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let mut deleted = payload(&owner, &agent);
        deleted.generation = 2;
        deleted.previous_event_id = Some("33".repeat(32));
        deleted.state = State::Deleted;
        deleted.active = None;
        deleted.deleted_at = Some("2026-08-03T18:01:00Z".into());
        validate_payload(&deleted).unwrap();

        deleted.previous_event_id = None;
        assert!(validate_payload(&deleted).is_err());
    }

    #[test]
    fn outer_tag_grammar_rejects_duplicates_and_noncanonical_generation() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let body = payload(&owner, &agent);
        let ciphertext = nip44::encrypt(
            owner.secret_key(),
            &owner.public_key(),
            serde_json::to_string(&body).unwrap(),
            Version::V2,
        )
        .unwrap();
        let event = EventBuilder::new(Kind::Custom(KIND_PRIVATE_MANAGED_AGENT as u16), ciphertext)
            .tags(vec![
                Tag::parse(["d", agent.public_key().to_hex().as_str()]).unwrap(),
                Tag::parse(["g", "01"]).unwrap(),
                Tag::parse(["state", "active"]).unwrap(),
            ])
            .sign_with_keys(&owner)
            .unwrap();
        assert!(matches!(
            validate_envelope(&event, &owner.public_key()),
            Err(Error::InvalidEnvelope(message)) if message.contains("canonical decimal")
        ));
    }

    #[test]
    fn projection_recovery_must_match_binding_and_coordinate() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let mut candidate = payload(&owner, &agent);
        let active = candidate.active.as_mut().unwrap();
        active.instance_projection.content_sha256 = content_sha256(b"wrong");
        assert!(matches!(
            validate_payload(&candidate),
            Err(Error::InvalidPayload(message)) if message.contains("does not match binding")
        ));

        let mut candidate = payload(&owner, &agent);
        candidate
            .active
            .as_mut()
            .unwrap()
            .config
            .definition_coordinate =
            Some(format!("30175:{}:wrong-slug", owner.public_key().to_hex()));
        assert!(matches!(
            validate_payload(&candidate),
            Err(Error::InvalidPayload(message)) if message.contains("wrong coordinate")
        ));
        let mut candidate = payload(&owner, &agent);
        candidate
            .active
            .as_mut()
            .unwrap()
            .definition
            .recovery
            .version = 2;
        assert!(matches!(
            validate_payload(&candidate),
            Err(Error::InvalidPayload(message)) if message.contains("unsupported definition recovery version")
        ));

        let mut candidate = payload(&owner, &agent);
        candidate
            .active
            .as_mut()
            .unwrap()
            .definition
            .recovery
            .signed_event
            .content
            .push('!');
        assert!(matches!(
            validate_payload(&candidate),
            Err(Error::InvalidPayload(message)) if message.contains("invalid definition recovery event")
        ));

        let mut candidate = payload(&owner, &agent);
        let wrong_kind = EventBuilder::new(Kind::Custom(KIND_MANAGED_AGENT as u16), "definition")
            .tags(vec![Tag::parse(["d", "test-agent"]).unwrap()])
            .sign_with_keys(&owner)
            .unwrap();
        let definition = &mut candidate.active.as_mut().unwrap().definition;
        definition.event_id = wrong_kind.id.to_hex();
        definition.content_sha256 = content_sha256(wrong_kind.content.as_bytes());
        definition.recovery.signed_event = wrong_kind;
        assert!(matches!(
            validate_payload(&candidate),
            Err(Error::InvalidPayload(message)) if message.contains("does not match binding")
        ));

        let mut candidate = payload(&owner, &agent);
        let missing_d = EventBuilder::new(Kind::Custom(KIND_PERSONA as u16), "definition")
            .sign_with_keys(&owner)
            .unwrap();
        let definition = &mut candidate.active.as_mut().unwrap().definition;
        definition.event_id = missing_d.id.to_hex();
        definition.content_sha256 = content_sha256(missing_d.content.as_bytes());
        definition.recovery.signed_event = missing_d;
        assert!(matches!(
            validate_payload(&candidate),
            Err(Error::InvalidPayload(message)) if message.contains("exactly one non-empty d tag")
        ));
    }

    #[test]
    fn projection_hash_fixture_is_stable() {
        assert_eq!(
            content_sha256(b"buzz-private-managed-agent-v1"),
            "c3ca1603249c95343fc1766ba58d075d6bdf0e57b375bef38738729b2022cc80"
        );
    }
}
