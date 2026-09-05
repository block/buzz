//! NIP-PMA private managed-agent wire codec.
//!
//! This module defines and validates the owner-authored encrypted wire format.
//! Relays treat it as global owner data; Desktop performs all decryption and
//! device-specific runtime validation.

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

use crate::kind::KIND_PRIVATE_MANAGED_AGENT;

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
/// Maximum UTF-8 bytes in a portable effort level (a short runtime keyword).
pub const MAX_EFFORT_LEVEL_BYTES: usize = 256;
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
///
/// Forward-compatible: unknown JSON members authored by a newer Desktop are
/// preserved verbatim in [`PrivateConfig::extra`] rather than rejected, so an
/// older writer round-tripping this config cannot silently drop them. Known
/// members are still strictly typed; unknown members can never override a
/// known field (serde routes a matching key to the typed field first).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct PrivateConfig {
    /// Intended relay endpoint; validated again on each device before use.
    pub relay_url: String,
    /// Unique agent handle (`ManagedAgentRecord.name`). Required for fresh-device
    /// reconstruction. Non-empty.
    pub name: String,
    /// Stable definition/persona slug (`ManagedAgentRecord.persona_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona_id: Option<String>,
    /// Preferred ACP runtime id, e.g. `"goose"`/`"claude"`. `None` = inherit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// Desired LLM model id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// LLM inference provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// System prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Turn parallelism. `None` = the Desktop default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism: Option<u32>,
    /// Inbound author gate mode as the NIP-AP wire string
    /// (`"owner-only"`/`"allowlist"`/`"anyone"`). Wire string, not the Desktop
    /// `RespondTo` enum, so unknown future modes round-trip verbatim. `None` =
    /// the Desktop default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respond_to: Option<String>,
    /// Allowlist used when `respond_to == "allowlist"`; normalized lowercase hex.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub respond_to_allowlist: Vec<String>,
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
    /// Canonical harness-agnostic effort level (`ManagedAgentRecord.effort_level`).
    /// Carried verbatim; each device normalizes it against the destination
    /// runtime at spawn. `None` = inherit. Absent in payloads authored before
    /// this field existed, which deserialize as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort_level: Option<String>,
    /// Unknown JSON members preserved verbatim for forward compatibility.
    ///
    /// A newer Desktop may author config keys this version does not model; they
    /// round-trip here untouched so an older writer never drops them. Never
    /// contains a key that collides with a known field above (serde binds known
    /// keys first). Core semantics must never depend on this map.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl fmt::Debug for PrivateConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivateConfig")
            .field("contents", &"<redacted>")
            .finish()
    }
}

/// Decrypted private managed-agent payload.
///
/// Forward-compatible at the top level: unknown JSON members authored by a
/// newer Desktop round-trip verbatim in [`Payload::extra`] (see [`PrivateConfig`]
/// for the same guarantee on config). Known members remain strictly typed and
/// validated; an unknown member can never override a known field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Payload {
    /// Always [`FORMAT`].
    pub format: String,
    /// Always [`VERSION`].
    pub version: u32,
    /// Agent pubkey and event `d` coordinate.
    pub agent_pubkey: String,
    /// Owner pubkey and signed event author.
    pub owner_pubkey: String,
    /// Advisory monotonic generation (validated shape, never CAS-enforced).
    pub generation: u64,
    /// Advisory predecessor event ID; absent exactly at generation one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_event_id: Option<String>,
    /// RFC3339 bookkeeping timestamp; never used for conflict resolution.
    pub updated_at: String,
    /// Secret identity material.
    pub identity: PrivateIdentity,
    /// Private portable/device-validated configuration.
    pub config: PrivateConfig,
    /// Forward-compatible namespaced data. Core semantics must never depend on it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
    /// Unknown top-level JSON members preserved verbatim for forward
    /// compatibility. A newer Desktop may author payload keys this version does
    /// not model; they round-trip here untouched so an older writer never drops
    /// them. Never contains a key that collides with a known field above (serde
    /// binds known keys first). Core semantics must never depend on this map.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

/// Validated public metadata from a private managed-agent event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// Agent pubkey from `d`.
    pub agent_pubkey: PublicKey,
    /// Owner pubkey from the signed event author.
    pub owner_pubkey: PublicKey,
    /// Advisory generation from `g` (validated shape, never CAS-enforced).
    pub generation: u64,
    /// Advisory predecessor from `prev`.
    pub previous_event_id: Option<EventId>,
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
    Ok(Envelope {
        agent_pubkey,
        owner_pubkey,
        generation,
        previous_event_id,
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
    let owner = parse_canonical_pubkey("owner_pubkey", &payload.owner_pubkey)
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
    validate_identity_and_config(&payload.identity, &payload.config, &agent, &owner)?;
    Ok(())
}

/// Validate the secret identity and portable config of a payload.
///
/// The nsec must derive the payload's `agent_pubkey` (the `d` coordinate), and
/// the config's bounds must hold. This is the nsec→coordinate binding gate.
fn validate_identity_and_config(
    identity: &PrivateIdentity,
    config: &PrivateConfig,
    agent: &PublicKey,
    owner: &PublicKey,
) -> Result<(), Error> {
    let agent_keys = Keys::parse(identity.private_key_nsec.trim())
        .map_err(|_| Error::InvalidPayload("invalid agent nsec".into()))?;
    if agent_keys.public_key() != *agent {
        return Err(Error::InvalidPayload(
            "agent nsec does not derive agent_pubkey".into(),
        ));
    }
    if let Some(auth_tag) = &identity.auth_tag {
        validate_auth_tag(auth_tag, &owner.to_hex(), agent)?;
    }
    // Empty is legal: a pin-less agent resolves to the active workspace relay
    // at read time, so only the upper bound is enforced here.
    if config.relay_url.len() > 4096 {
        return Err(Error::InvalidPayload("invalid relay_url length".into()));
    }
    if config.name.is_empty() || config.name.len() > 4096 {
        return Err(Error::InvalidPayload("invalid name length".into()));
    }
    if config
        .effort_level
        .as_deref()
        .is_some_and(|effort| effort.len() > MAX_EFFORT_LEVEL_BYTES)
    {
        return Err(Error::InvalidPayload("invalid effort_level length".into()));
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

    /// Minimal valid payload: the nsec derives `agent_pubkey`, generation 1,
    /// required config fields present, no unknown members.
    fn payload(owner: &Keys, agent: &Keys) -> Payload {
        Payload {
            format: FORMAT.into(),
            version: VERSION,
            agent_pubkey: agent.public_key().to_hex(),
            owner_pubkey: owner.public_key().to_hex(),
            generation: 1,
            previous_event_id: None,
            updated_at: "2026-08-03T18:00:00Z".into(),
            identity: PrivateIdentity {
                private_key_nsec: agent.secret_key().to_bech32().unwrap(),
                auth_tag: None,
            },
            config: PrivateConfig {
                relay_url: "wss://relay.example".into(),
                name: "aphid".into(),
                persona_id: Some("aphid-def".into()),
                runtime: Some("goose".into()),
                model: None,
                provider: None,
                system_prompt: Some("be terse".into()),
                parallelism: Some(2),
                respond_to: Some("owner-only".into()),
                respond_to_allowlist: vec![],
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
                effort_level: None,
                extra: serde_json::Map::new(),
            },
            extensions: BTreeMap::new(),
            extra: serde_json::Map::new(),
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
        assert_eq!(envelope.previous_event_id, None);
    }

    #[test]
    fn debug_output_redacts_private_material() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let mut candidate = payload(&owner, &agent);
        let private_key_nsec = candidate.identity.private_key_nsec.clone();
        candidate.identity.auth_tag = Some("secret-auth-tag".into());
        candidate.config.backend = serde_json::json!({"token": "secret-backend-token"});

        let debug = format!("{candidate:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains(&private_key_nsec));
        assert!(!debug.contains("secret-auth-tag"));
        assert!(!debug.contains("not-public"));
        assert!(!debug.contains("secret-backend-token"));
    }

    // (1) privacy / wrong-owner + tamper fail closed.
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

    // (2) nsec -> pubkey binding: the identity nsec must derive agent_pubkey (d).
    #[test]
    fn active_identity_must_derive_coordinate() {
        let owner = Keys::generate();
        let mut candidate = payload(&owner, &Keys::generate());
        candidate.identity.private_key_nsec = Keys::generate().secret_key().to_bech32().unwrap();
        assert!(matches!(
            validate_payload(&candidate),
            Err(Error::InvalidPayload(message)) if message.contains("does not derive")
        ));
    }

    /// A pin-less agent (empty `relay_url`, legal since the per-record pin
    /// became read-time-ignored) must still produce a valid payload.
    #[test]
    fn empty_relay_url_is_accepted() {
        let owner = Keys::generate();
        let mut candidate = payload(&owner, &Keys::generate());
        candidate.config.relay_url.clear();
        validate_payload(&candidate).unwrap();
    }

    // `effort_level` is a typed portable field: it round-trips through the
    // owner-self codec, is bounded, and a pre-field payload (key absent)
    // decodes to `None` rather than being rejected.
    #[test]
    fn effort_level_is_typed_bounded_and_backward_compatible() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let mut expected = payload(&owner, &agent);
        expected.config.effort_level = Some("high".into());
        let event = build_event(&owner, &expected, 1_785_780_000).unwrap();
        let (_, actual) = validate_and_decrypt(&event, &owner).unwrap();
        assert_eq!(actual.config.effort_level.as_deref(), Some("high"));
        assert!(!actual.config.extra.contains_key("effort_level"));

        let mut oversized = payload(&owner, &agent);
        oversized.config.effort_level = Some("x".repeat(MAX_EFFORT_LEVEL_BYTES + 1));
        assert!(matches!(
            validate_payload(&oversized),
            Err(Error::InvalidPayload(message)) if message.contains("effort_level")
        ));

        let mut legacy = serde_json::to_value(payload(&owner, &agent)).unwrap();
        let config = legacy["config"].as_object_mut().unwrap();
        assert!(
            config.remove("effort_level").is_none(),
            "None must serialize as absent"
        );
        let decoded: Payload = serde_json::from_value(legacy).unwrap();
        assert_eq!(decoded.config.effort_level, None);
    }

    #[test]
    fn valid_owner_attestation_passes_and_binds_agent() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let mut candidate = payload(&owner, &agent);
        // The owner signs an unconditional attestation over the AGENT key.
        candidate.identity.auth_tag = Some(auth_tag(&owner, &agent));
        assert!(validate_payload(&candidate).is_ok());

        // Round-trips end-to-end with the attestation intact.
        let event = build_event(&owner, &candidate, 1_785_780_000).unwrap();
        let (_envelope, decoded) = validate_and_decrypt(&event, &owner).unwrap();
        assert_eq!(decoded, candidate);
    }

    #[test]
    fn auth_tag_from_wrong_attestor_is_rejected() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let mut candidate = payload(&owner, &agent);
        // A stranger (not the owner) signs the attestation: parts[1] is not the
        // owner pubkey, so the attestation is rejected.
        let stranger = Keys::generate();
        candidate.identity.auth_tag = Some(auth_tag(&stranger, &agent));
        assert!(matches!(
            validate_payload(&candidate),
            Err(Error::InvalidPayload(message)) if message.contains("auth_tag")
        ));
    }

    #[test]
    fn auth_tag_signature_must_verify() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let mut candidate = payload(&owner, &agent);
        // Correct owner in parts[1], but the signature is over a DIFFERENT agent
        // key, so schnorr verification against this agent's preimage fails.
        let other_agent = Keys::generate();
        let preimage = format!("nostr:agent-auth:{}:", other_agent.public_key().to_hex());
        let digest = Sha256::digest(preimage.as_bytes());
        let signature = owner.sign_schnorr(&Message::from_digest(digest.into()));
        candidate.identity.auth_tag = Some(
            serde_json::json!([
                "auth",
                owner.public_key().to_hex(),
                "",
                signature.to_string()
            ])
            .to_string(),
        );
        assert!(matches!(
            validate_payload(&candidate),
            Err(Error::InvalidPayload(message)) if message.contains("signature")
        ));
    }

    // (3) unknown-field round-trip: unknown top-level + config members survive
    // verbatim through serialize/deserialize and land in the `extra` maps.
    #[test]
    fn unknown_members_round_trip_verbatim() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let mut candidate = payload(&owner, &agent);
        candidate
            .extra
            .insert("future_top".into(), serde_json::json!({"nested": [1, 2]}));
        candidate
            .config
            .extra
            .insert("future_cfg".into(), Value::String("keep-me".into()));

        let event = build_event(&owner, &candidate, 1_785_780_000).unwrap();
        let (_envelope, decoded) = validate_and_decrypt(&event, &owner).unwrap();
        assert_eq!(decoded, candidate);
        assert_eq!(
            decoded.extra.get("future_top"),
            Some(&serde_json::json!({"nested": [1, 2]}))
        );
        assert_eq!(
            decoded.config.extra.get("future_cfg"),
            Some(&Value::String("keep-me".into()))
        );
    }

    // (3b) an unknown member can never override a known field: serde binds the
    // typed field first, so a colliding key is impossible to smuggle into `extra`.
    #[test]
    fn unknown_member_cannot_override_known_field() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let mut value = serde_json::to_value(payload(&owner, &agent)).unwrap();
        // Inject a duplicate-looking known key into the config object; serde
        // routes it to the typed `name`, NOT to `extra`.
        value["config"]["name"] = Value::String("renamed".into());
        let decoded: Payload = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.config.name, "renamed");
        assert!(!decoded.config.extra.contains_key("name"));
    }

    // (4) required / null semantics: a missing required known field is rejected;
    // duplicate JSON keys are rejected by the strict parser.
    #[test]
    fn required_fields_and_duplicate_keys() {
        let owner = Keys::generate();
        let agent = Keys::generate();

        // Missing required `config.name`.
        let mut value = serde_json::to_value(payload(&owner, &agent)).unwrap();
        value["config"].as_object_mut().unwrap().remove("name");
        assert!(serde_json::from_value::<Payload>(value).is_err());

        // Duplicate top-level key rejected pre-deserialization.
        let duplicate = br#"{"format":"a","format":"b"}"#;
        assert!(matches!(
            parse_strict_json(duplicate),
            Err(Error::InvalidPayload(message)) if message.contains("duplicate key")
        ));

        // Empty required `name` fails semantic validation.
        let mut empty_name = payload(&owner, &agent);
        empty_name.config.name = String::new();
        assert!(matches!(
            validate_payload(&empty_name),
            Err(Error::InvalidPayload(message)) if message.contains("invalid name length")
        ));
    }

    // gen/prev are a3 advisory metadata: shape is validated (gen1 XOR prev,
    // canonical decimal, outer/inner equality) but ordering is never enforced.
    #[test]
    fn generation_prev_shape_is_validated_metadata() {
        let owner = Keys::generate();
        let agent = Keys::generate();

        // Higher generation with a well-formed prev round-trips fine — no head
        // consult, no staleness rejection.
        let mut successor = payload(&owner, &agent);
        successor.generation = 7;
        successor.previous_event_id = Some("33".repeat(32));
        let event = build_event(&owner, &successor, 1_785_780_001).unwrap();
        let (envelope, decoded) = validate_and_decrypt(&event, &owner).unwrap();
        assert_eq!(envelope.generation, 7);
        assert_eq!(decoded.generation, 7);

        // prev present at generation 1 violates the shape rule.
        let mut bad = payload(&owner, &agent);
        bad.previous_event_id = Some("33".repeat(32));
        assert!(matches!(
            validate_payload(&bad),
            Err(Error::InvalidPayload(message)) if message.contains("absent exactly at generation 1")
        ));
    }

    #[test]
    fn outer_tag_grammar_rejects_noncanonical_generation_and_stray_tags() {
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
        // Non-canonical generation "01".
        let event = EventBuilder::new(
            Kind::Custom(KIND_PRIVATE_MANAGED_AGENT as u16),
            ciphertext.clone(),
        )
        .tags(vec![
            Tag::parse(["d", agent.public_key().to_hex().as_str()]).unwrap(),
            Tag::parse(["g", "01"]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
        assert!(matches!(
            validate_envelope(&event, &owner.public_key()),
            Err(Error::InvalidEnvelope(message)) if message.contains("canonical decimal")
        ));

        // A stray lifecycle `state` tag is now unexpected (lifecycle removed).
        let stray = EventBuilder::new(Kind::Custom(KIND_PRIVATE_MANAGED_AGENT as u16), ciphertext)
            .tags(vec![
                Tag::parse(["d", agent.public_key().to_hex().as_str()]).unwrap(),
                Tag::parse(["g", "1"]).unwrap(),
                Tag::parse(["state", "active"]).unwrap(),
            ])
            .sign_with_keys(&owner)
            .unwrap();
        assert!(matches!(
            validate_envelope(&stray, &owner.public_key()),
            Err(Error::InvalidEnvelope(message)) if message.contains("unexpected tag")
        ));
    }

    #[test]
    fn hash_fixture_is_stable() {
        assert_eq!(
            content_sha256(b"buzz-private-managed-agent-v1"),
            "c3ca1603249c95343fc1766ba58d075d6bdf0e57b375bef38738729b2022cc80"
        );
    }
}
