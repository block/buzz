//! Owner-encrypted, versioned autonomous skill contracts.

use chrono::DateTime;
use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use nostr::nips::nip44::{self, v2::ConversationKey, Version};
use nostr::{Event, EventBuilder, Keys, Kind, PublicKey, SecretKey, Tag};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::kind::{KIND_AGENT_SKILL_POINTER, KIND_AGENT_SKILL_VERSION};

/// Maximum UTF-8 size of one managed `SKILL.md` body.
pub const MAX_SKILL_BODY_BYTES: usize = 32 * 1024;
/// Maximum number of entries in any skill payload list.
pub const MAX_SKILL_ITEMS: usize = 64;
/// Maximum plaintext accepted by NIP-44 v2.
pub const MAX_SKILL_PLAINTEXT_BYTES: usize = 65_535;

const MAX_ID_BYTES: usize = 512;
const MAX_TEST_TEXT_BYTES: usize = 4_096;
const VERSION_D_TAG_DOMAIN: &[u8] = b"agent-skill-version/v1/d-tag";
const POINTER_D_TAG_DOMAIN: &[u8] = b"agent-skill-pointer/v1/d-tag";
const SAFE_REQUIRED_TOOLS: &[&str] = &[
    "rag.search",
    "memory.recall",
    "memory.record",
    "buzz.messages",
];

/// Visibility of a learned skill.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkillScope {
    /// Visible only to the originating specialist and owner.
    SpecialistPrivate,
    /// Shared with the owner's Command Team.
    CommandTeamShared,
}

/// One deterministic skill check.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillTestV1 {
    /// Stable check identity.
    pub check_id: String,
    /// Supported deterministic check kind.
    pub kind: String,
    /// Bounded literal expected by the check.
    pub expected: String,
}

/// Immutable version of a learned skill.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillVersionV1 {
    /// Stable managed skill identity.
    pub skill_id: String,
    /// Globally unique immutable version identity.
    pub version_id: String,
    /// Prior immutable version, if this version evolves an earlier skill.
    pub parent_version_id: Option<String>,
    /// Visibility of the skill.
    pub scope: SkillScope,
    /// Originating specialist for private skills.
    pub specialist_id: Option<String>,
    /// Command Team identity for shared skills.
    pub team_id: Option<String>,
    /// RFC3339 creation time.
    pub created_at: String,
    /// Distinct successful experiences supporting the version.
    pub source_experience_ids: Vec<String>,
    /// Existing allowlisted tools required by this skill.
    pub required_tools: Vec<String>,
    /// Checks inherited from the active parent.
    pub inherited_tests: Vec<SkillTestV1>,
    /// Checks added for this version.
    pub regression_tests: Vec<SkillTestV1>,
    /// Complete text-only skill body.
    pub skill_md: String,
    /// Lowercase SHA-256 of the exact `skill_md` UTF-8 bytes.
    pub content_hash: String,
}

/// Why an active skill pointer changed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillPointerReason {
    /// A candidate passed deterministic evaluation.
    Promotion,
    /// Repeated verified regressions restored the parent.
    Rollback,
}

/// Replaceable pointer to the active immutable version of one skill.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillPointerV1 {
    /// Stable managed skill identity.
    pub skill_id: String,
    /// Immutable version that should be active.
    pub active_version_id: String,
    /// Version replaced by this pointer update, if any.
    pub previous_version_id: Option<String>,
    /// Visibility of the skill.
    pub scope: SkillScope,
    /// Originating specialist for private skills.
    pub specialist_id: Option<String>,
    /// Command Team identity for shared skills.
    pub team_id: Option<String>,
    /// RFC3339 pointer update time.
    pub changed_at: String,
    /// Promotion or rollback.
    pub reason: SkillPointerReason,
    /// Deterministic evaluations supporting the change.
    pub evaluation_ids: Vec<String>,
}

/// Skill encoding, validation, cryptography, or envelope failure.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    /// Payload failed the bounded contract.
    #[error("invalid agent skill")]
    Invalid,
    /// JSON encoding or decoding failed.
    #[error("agent skill serialization failed")]
    Serialization,
    /// NIP-44 encryption failed.
    #[error("agent skill encryption failed")]
    Encrypt,
    /// NIP-44 decryption failed.
    #[error("agent skill decryption failed")]
    Decrypt,
    /// Event construction or signing failed.
    #[error("agent skill signing failed")]
    Sign,
}

impl SkillVersionV1 {
    /// Validate a bounded immutable skill version.
    pub fn validate(&self) -> Result<(), SkillError> {
        if !valid_managed_skill_id(&self.skill_id)
            || !valid_id(&self.version_id)
            || !valid_optional_id(self.parent_version_id.as_deref())
            || self.parent_version_id.as_deref() == Some(self.version_id.as_str())
            || DateTime::parse_from_rfc3339(&self.created_at).is_err()
            || self.source_experience_ids.len() < 2
            || !valid_distinct_ids(&self.source_experience_ids)
            || self.required_tools.len() > MAX_SKILL_ITEMS
            || self
                .required_tools
                .iter()
                .any(|tool| !SAFE_REQUIRED_TOOLS.contains(&tool.as_str()))
            || !valid_tests(&self.inherited_tests)
            || !valid_tests(&self.regression_tests)
            || duplicate_test_ids(&self.inherited_tests, &self.regression_tests)
            || self.skill_md.trim().is_empty()
            || self.skill_md.len() > MAX_SKILL_BODY_BYTES
            || self.skill_md.contains('\0')
            || !is_lower_hex_64(&self.content_hash)
            || skill_body_hash(&self.skill_md) != self.content_hash
            || !valid_scope(
                self.scope,
                self.specialist_id.as_deref(),
                self.team_id.as_deref(),
            )
            || serde_json::to_vec(self)
                .map_or(true, |encoded| encoded.len() > MAX_SKILL_PLAINTEXT_BYTES)
        {
            return Err(SkillError::Invalid);
        }
        Ok(())
    }
}

impl SkillPointerV1 {
    /// Validate a bounded active-version pointer.
    pub fn validate(&self) -> Result<(), SkillError> {
        if !valid_managed_skill_id(&self.skill_id)
            || !valid_id(&self.active_version_id)
            || !valid_optional_id(self.previous_version_id.as_deref())
            || self.previous_version_id.as_deref() == Some(self.active_version_id.as_str())
            || DateTime::parse_from_rfc3339(&self.changed_at).is_err()
            || !valid_distinct_ids(&self.evaluation_ids)
            || self.evaluation_ids.is_empty()
            || !valid_scope(
                self.scope,
                self.specialist_id.as_deref(),
                self.team_id.as_deref(),
            )
            || matches!(self.reason, SkillPointerReason::Rollback)
                && self.previous_version_id.is_none()
        {
            return Err(SkillError::Invalid);
        }
        Ok(())
    }
}

/// Compute the lowercase SHA-256 of an exact text skill body.
pub fn skill_body_hash(skill_md: &str) -> String {
    hex::encode(Sha256::digest(skill_md.as_bytes()))
}

/// Build and sign one owner-encrypted immutable skill version event.
pub fn build_skill_version_event(
    agent_keys: &Keys,
    owner_pubkey: &PublicKey,
    payload: &SkillVersionV1,
    created_at: u64,
) -> Result<Event, SkillError> {
    payload.validate()?;
    build_event(
        agent_keys,
        owner_pubkey,
        KIND_AGENT_SKILL_VERSION,
        version_d_tag(
            &conversation_key(agent_keys.secret_key(), owner_pubkey),
            &payload.skill_id,
            &payload.version_id,
        ),
        payload,
        created_at,
    )
}

/// Build and sign one owner-encrypted replaceable active pointer event.
pub fn build_skill_pointer_event(
    agent_keys: &Keys,
    owner_pubkey: &PublicKey,
    payload: &SkillPointerV1,
    created_at: u64,
) -> Result<Event, SkillError> {
    payload.validate()?;
    build_event(
        agent_keys,
        owner_pubkey,
        KIND_AGENT_SKILL_POINTER,
        pointer_d_tag(
            &conversation_key(agent_keys.secret_key(), owner_pubkey),
            &payload.skill_id,
        ),
        payload,
        created_at,
    )
}

/// Verify, decrypt, and validate an immutable skill version event.
pub fn validate_and_decrypt_skill_version(
    event: &Event,
    expected_agent: &PublicKey,
    expected_owner: &PublicKey,
    my_seckey: &SecretKey,
    their_pubkey: &PublicKey,
) -> Result<SkillVersionV1, SkillError> {
    let payload: SkillVersionV1 = decrypt_event(
        event,
        KIND_AGENT_SKILL_VERSION,
        expected_agent,
        expected_owner,
        my_seckey,
        their_pubkey,
    )?;
    payload.validate()?;
    let expected_d = version_d_tag(
        &conversation_key(my_seckey, their_pubkey),
        &payload.skill_id,
        &payload.version_id,
    );
    require_address(event, &expected_d)?;
    Ok(payload)
}

/// Verify, decrypt, and validate a replaceable active skill pointer event.
pub fn validate_and_decrypt_skill_pointer(
    event: &Event,
    expected_agent: &PublicKey,
    expected_owner: &PublicKey,
    my_seckey: &SecretKey,
    their_pubkey: &PublicKey,
) -> Result<SkillPointerV1, SkillError> {
    let payload: SkillPointerV1 = decrypt_event(
        event,
        KIND_AGENT_SKILL_POINTER,
        expected_agent,
        expected_owner,
        my_seckey,
        their_pubkey,
    )?;
    payload.validate()?;
    let expected_d = pointer_d_tag(
        &conversation_key(my_seckey, their_pubkey),
        &payload.skill_id,
    );
    require_address(event, &expected_d)?;
    Ok(payload)
}

fn build_event<T: Serialize>(
    agent_keys: &Keys,
    owner_pubkey: &PublicKey,
    kind: u32,
    d_tag: String,
    payload: &T,
    created_at: u64,
) -> Result<Event, SkillError> {
    let plaintext = serde_json::to_string(payload).map_err(|_| SkillError::Serialization)?;
    let ciphertext = nip44::encrypt(
        agent_keys.secret_key(),
        owner_pubkey,
        plaintext,
        Version::V2,
    )
    .map_err(|_| SkillError::Encrypt)?;
    let tags = vec![
        Tag::parse(["d", d_tag.as_str()]).map_err(|_| SkillError::Encrypt)?,
        Tag::parse(["p", owner_pubkey.to_hex().as_str()]).map_err(|_| SkillError::Encrypt)?,
    ];
    EventBuilder::new(Kind::Custom(kind as u16), ciphertext)
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from(created_at))
        .sign_with_keys(agent_keys)
        .map_err(|_| SkillError::Sign)
}

fn decrypt_event<T: DeserializeOwned>(
    event: &Event,
    expected_kind: u32,
    expected_agent: &PublicKey,
    expected_owner: &PublicKey,
    my_seckey: &SecretKey,
    their_pubkey: &PublicKey,
) -> Result<T, SkillError> {
    if event.kind.as_u16() as u32 != expected_kind
        || event.pubkey != *expected_agent
        || !event.verify_id()
        || !event.verify_signature()
    {
        return Err(SkillError::Invalid);
    }
    require_owner(event, expected_owner)?;
    let plaintext =
        nip44::decrypt(my_seckey, their_pubkey, &event.content).map_err(|_| SkillError::Decrypt)?;
    serde_json::from_str(&plaintext).map_err(|_| SkillError::Serialization)
}

fn require_owner(event: &Event, expected_owner: &PublicKey) -> Result<(), SkillError> {
    let owners: Vec<&str> = event
        .tags
        .iter()
        .filter(|tag| tag.kind().to_string() == "p")
        .filter_map(|tag| tag.content())
        .collect();
    if owners.len() == 1 && owners[0] == expected_owner.to_hex() {
        Ok(())
    } else {
        Err(SkillError::Invalid)
    }
}

fn require_address(event: &Event, expected_d: &str) -> Result<(), SkillError> {
    let addresses: Vec<&str> = event
        .tags
        .iter()
        .filter(|tag| tag.kind().to_string() == "d")
        .filter_map(|tag| tag.content())
        .collect();
    if addresses.len() == 1 && addresses[0] == expected_d && is_lower_hex_64(expected_d) {
        Ok(())
    } else {
        Err(SkillError::Invalid)
    }
}

fn conversation_key(my_seckey: &SecretKey, their_pubkey: &PublicKey) -> ConversationKey {
    ConversationKey::derive(my_seckey, their_pubkey)
        .unwrap_or_else(|_| unreachable!("validated secp256k1 keys derive a conversation key"))
}

fn version_d_tag(key: &ConversationKey, skill_id: &str, version_id: &str) -> String {
    hmac_d_tag(key, VERSION_D_TAG_DOMAIN, &[skill_id, version_id])
}

fn pointer_d_tag(key: &ConversationKey, skill_id: &str) -> String {
    hmac_d_tag(key, POINTER_D_TAG_DOMAIN, &[skill_id])
}

fn hmac_d_tag(key: &ConversationKey, domain: &[u8], components: &[&str]) -> String {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key.as_bytes())
        .unwrap_or_else(|_| unreachable!("HMAC-SHA256 accepts the conversation key"));
    mac.update(domain);
    for component in components {
        mac.update(&[0]);
        mac.update(component.as_bytes());
    }
    hex::encode(mac.finalize().into_bytes())
}

fn valid_scope(scope: SkillScope, specialist_id: Option<&str>, team_id: Option<&str>) -> bool {
    match scope {
        SkillScope::SpecialistPrivate => specialist_id.is_some_and(valid_id) && team_id.is_none(),
        SkillScope::CommandTeamShared => team_id.is_some_and(valid_id) && specialist_id.is_none(),
    }
}

fn valid_managed_skill_id(value: &str) -> bool {
    value
        .strip_prefix("learned-")
        .is_some_and(|suffix| suffix.len() == 12 && suffix.bytes().all(is_lower_hex))
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}

fn valid_optional_id(value: Option<&str>) -> bool {
    value.is_none_or(valid_id)
}

fn valid_distinct_ids(values: &[String]) -> bool {
    values.len() <= MAX_SKILL_ITEMS
        && values.iter().all(|value| valid_id(value))
        && values
            .iter()
            .enumerate()
            .all(|(index, value)| !values[index + 1..].contains(value))
}

fn valid_tests(values: &[SkillTestV1]) -> bool {
    values.len() <= MAX_SKILL_ITEMS
        && values.iter().all(|value| {
            valid_id(&value.check_id)
                && matches!(value.kind.as_str(), "contains" | "not_contains" | "exact")
                && !value.expected.trim().is_empty()
                && value.expected.len() <= MAX_TEST_TEXT_BYTES
                && !value.expected.contains('\0')
        })
}

fn duplicate_test_ids(first: &[SkillTestV1], second: &[SkillTestV1]) -> bool {
    first
        .iter()
        .chain(second)
        .enumerate()
        .any(|(index, value)| {
            first
                .iter()
                .chain(second)
                .skip(index + 1)
                .any(|other| other.check_id == value.check_id)
        })
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(is_lower_hex)
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
}
