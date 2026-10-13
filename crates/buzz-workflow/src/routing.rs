//! Project-channel routing types and control-plane hashing helpers.
//!
//! Webhook workflows may opt into `project_channel_by_repository` routing.
//! This module owns the definition schema, admission failure codes, and the
//! idempotency / payload hashing used at the webhook admission boundary.

use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use crate::error::WorkflowError;

/// Inbound routing contract for a webhook workflow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RoutingDef {
    /// Routing mode. The only supported value is `project_channel_by_repository`.
    pub mode: RoutingMode,
    /// Exact alias key to full `30617:<owner>:<d>` coordinate.
    #[serde(default, deserialize_with = "deserialize_aliases_no_duplicates")]
    pub aliases: BTreeMap<String, String>,
}

/// How inbound webhook traffic selects a destination channel.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutingMode {
    /// Resolve `repository_name` through NIP-MP to one project `buzz-channel`.
    ProjectChannelByRepository,
}

/// Deterministic webhook routing admission failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteFailure {
    /// Control fields missing, wrong type, or empty where non-empty is required.
    InvalidControlFields,
    /// No repository matched the requested name.
    RepositoryMissing,
    /// More than one repository matched the requested name.
    RepositoryAmbiguous,
    /// An alias target coordinate could not be loaded.
    AliasTargetUnavailable,
    /// No project matched the resolved repository.
    ProjectMissing,
    /// More than one project matched the resolved repository.
    ProjectAmbiguous,
    /// The project channel reference is missing or malformed.
    ProjectChannelInvalid,
    /// No live stream channel matched the requested channel name.
    ChannelMissing,
    /// More than one live stream channel matched the requested channel name.
    ChannelAmbiguous,
    /// Caller is not authorized for the resolved route.
    RouteUnauthorized,
    /// A previously valid route is no longer usable.
    RouteStale,
    /// Idempotency key reused with a different payload hash.
    IdempotencyConflict,
}

impl RouteFailure {
    /// Stable machine-readable code. Never include candidate lists or raw keys.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidControlFields => "invalid_control_fields",
            Self::RepositoryMissing => "repository_missing",
            Self::RepositoryAmbiguous => "repository_ambiguous",
            Self::AliasTargetUnavailable => "alias_target_unavailable",
            Self::ProjectMissing => "project_missing",
            Self::ProjectAmbiguous => "project_ambiguous",
            Self::ProjectChannelInvalid => "project_channel_invalid",
            Self::ChannelMissing => "channel_missing",
            Self::ChannelAmbiguous => "channel_ambiguous",
            Self::RouteUnauthorized => "route_unauthorized",
            Self::RouteStale => "route_stale",
            Self::IdempotencyConflict => "idempotency_conflict",
        }
    }

    /// HTTP status for the webhook admission boundary.
    pub const fn http_status(self) -> u16 {
        match self {
            Self::IdempotencyConflict => 409,
            Self::RouteStale => 422,
            _ => 422,
        }
    }

    /// Provider-visible redacted text. No coordinates, keys, or bodies.
    pub const fn redacted_message(self) -> &'static str {
        match self {
            Self::InvalidControlFields => "invalid routing control fields",
            Self::RepositoryMissing => "repository could not be resolved",
            Self::RepositoryAmbiguous => "repository is ambiguous",
            Self::AliasTargetUnavailable => "alias target is unavailable",
            Self::ProjectMissing => "project could not be resolved",
            Self::ProjectAmbiguous => "project is ambiguous",
            Self::ProjectChannelInvalid => "project channel is invalid",
            Self::ChannelMissing => "channel could not be resolved",
            Self::ChannelAmbiguous => "channel is ambiguous",
            Self::RouteUnauthorized => "route is unauthorized",
            Self::RouteStale => "route is no longer valid",
            Self::IdempotencyConflict => "idempotency key already used with a different payload",
        }
    }

    /// Parse a stored admission failure code for deterministic replay.
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "invalid_control_fields" => Some(Self::InvalidControlFields),
            "repository_missing" => Some(Self::RepositoryMissing),
            "repository_ambiguous" => Some(Self::RepositoryAmbiguous),
            "alias_target_unavailable" => Some(Self::AliasTargetUnavailable),
            "project_missing" => Some(Self::ProjectMissing),
            "project_ambiguous" => Some(Self::ProjectAmbiguous),
            "project_channel_invalid" => Some(Self::ProjectChannelInvalid),
            "channel_missing" => Some(Self::ChannelMissing),
            "channel_ambiguous" => Some(Self::ChannelAmbiguous),
            "route_unauthorized" => Some(Self::RouteUnauthorized),
            "route_stale" => Some(Self::RouteStale),
            "idempotency_conflict" => Some(Self::IdempotencyConflict),
            _ => None,
        }
    }

    /// True only for deterministic admission failures that replay as `422`.
    pub const fn is_admission_rejection(self) -> bool {
        !matches!(self, Self::RouteStale | Self::IdempotencyConflict)
    }
}

/// Parsed `30617:<owner_hex>:<d_tag>` repository coordinate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryCoordinate {
    owner_pubkey: [u8; 32],
    d_tag: String,
}

impl RepositoryCoordinate {
    /// Owner pubkey bytes.
    pub fn owner_pubkey(&self) -> &[u8; 32] {
        &self.owner_pubkey
    }

    /// Lowercase hex encoding of the owner pubkey.
    pub fn owner_hex(&self) -> String {
        hex::encode(self.owner_pubkey)
    }

    /// Repository `d` tag (may itself contain `:`).
    pub fn d_tag(&self) -> &str {
        &self.d_tag
    }

    /// Canonical `30617:<owner_hex>:<d_tag>` form.
    pub fn as_coordinate(&self) -> String {
        format!("30617:{}:{}", self.owner_hex(), self.d_tag)
    }
}

/// Parse a full `30617:<owner>:<d>` coordinate.
///
/// Splits on the first two colons only so colon-bearing `d` tags are preserved.
/// Owner hex must be exactly 64 lowercase characters; uppercase is rejected.
pub fn parse_repository_coordinate(coord: &str) -> Result<RepositoryCoordinate, WorkflowError> {
    let mut parts = coord.splitn(3, ':');
    let kind = parts.next().unwrap_or_default();
    let owner = parts.next().unwrap_or_default();
    let d_tag = parts.next().unwrap_or_default();

    if kind != "30617" {
        return Err(WorkflowError::InvalidDefinition(format!(
            "repository coordinate must start with 30617:, got {coord:?}"
        )));
    }
    if owner.len() != 64 || !owner.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
        return Err(WorkflowError::InvalidDefinition(format!(
            "repository coordinate owner must be 64 lowercase hex chars, got {owner:?}"
        )));
    }
    if d_tag.is_empty() {
        return Err(WorkflowError::InvalidDefinition(
            "repository coordinate d tag must be non-empty".into(),
        ));
    }

    let mut owner_pubkey = [0u8; 32];
    hex::decode_to_slice(owner, &mut owner_pubkey).map_err(|e| {
        WorkflowError::InvalidDefinition(format!(
            "repository coordinate owner hex decode failed: {e}"
        ))
    })?;

    Ok(RepositoryCoordinate {
        owner_pubkey,
        d_tag: d_tag.to_owned(),
    })
}

/// Extract a non-empty `idempotency_key` string from a JSON object body.
///
/// Does not trim or normalize; whitespace-only keys are accepted.
pub fn parse_idempotency_key(body: &serde_json::Value) -> Result<String, RouteFailure> {
    parse_non_empty_string_field(body, "idempotency_key")
}

/// Extract a non-empty `repository_name` string from a JSON object body.
///
/// Does not trim or normalize; whitespace-only names are accepted.
pub fn parse_repository_name(body: &serde_json::Value) -> Result<String, RouteFailure> {
    parse_non_empty_string_field(body, "repository_name")
}

fn parse_non_empty_string_field(
    body: &serde_json::Value,
    field: &str,
) -> Result<String, RouteFailure> {
    let Some(obj) = body.as_object() else {
        return Err(RouteFailure::InvalidControlFields);
    };
    let Some(value) = obj.get(field) else {
        return Err(RouteFailure::InvalidControlFields);
    };
    let Some(s) = value.as_str() else {
        return Err(RouteFailure::InvalidControlFields);
    };
    if s.is_empty() {
        return Err(RouteFailure::InvalidControlFields);
    }
    Ok(s.to_owned())
}

/// Remove only the `idempotency_key` object field; leave every other field intact.
pub fn strip_idempotency_key(mut body: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = body.as_object_mut() {
        obj.remove("idempotency_key");
    }
    body
}

/// SHA-256 of the raw UTF-8 idempotency key bytes.
pub fn hash_idempotency_key(raw: &str) -> [u8; 32] {
    Sha256::digest(raw.as_bytes()).into()
}

/// SHA-256 of the canonical JSON encoding of `body` (`serde_json::to_vec`).
pub fn canonical_payload_hash(body: &serde_json::Value) -> Result<[u8; 32], WorkflowError> {
    let bytes = serde_json::to_vec(body)
        .map_err(|e| WorkflowError::InvalidDefinition(format!("payload serialize failed: {e}")))?;
    Ok(Sha256::digest(&bytes).into())
}

fn deserialize_aliases_no_duplicates<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct AliasesVisitor {
        marker: PhantomData<fn() -> BTreeMap<String, String>>,
    }

    impl<'de> Visitor<'de> for AliasesVisitor {
        type Value = BTreeMap<String, String>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a map of routing alias keys to coordinates")
        }

        fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut aliases = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, String>()? {
                if aliases.insert(key.clone(), value).is_some() {
                    return Err(de::Error::custom(format!(
                        "duplicate routing alias key: {key}"
                    )));
                }
            }
            Ok(aliases)
        }
    }

    deserializer.deserialize_map(AliasesVisitor {
        marker: PhantomData,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn idempotency_key_requires_a_non_empty_string() {
        let err = parse_idempotency_key(&json!({"idempotency_key": ""})).unwrap_err();
        assert_eq!(err, RouteFailure::InvalidControlFields);
        assert_eq!(err.code(), "invalid_control_fields");
        assert_eq!(err.http_status(), 422);
        for invalid in [json!({}), json!({"idempotency_key": 7})] {
            assert_eq!(
                parse_idempotency_key(&invalid).unwrap_err(),
                RouteFailure::InvalidControlFields
            );
        }
        assert_eq!(
            parse_idempotency_key(&json!({"idempotency_key": " "})).unwrap(),
            " "
        );
    }

    #[test]
    fn repository_name_is_non_empty_but_is_not_trimmed() {
        assert_eq!(
            parse_repository_name(&json!({"repository_name": ""})).unwrap_err(),
            RouteFailure::InvalidControlFields
        );
        assert_eq!(
            parse_repository_name(&json!({"repository_name": " "})).unwrap(),
            " "
        );
    }

    #[test]
    fn strip_removes_only_idempotency_key() {
        let body = json!({
            "repository_name": "agentic-os-plan",
            "idempotency_key": "secret-key",
            "title": "hi"
        });
        let stripped = strip_idempotency_key(body);
        assert_eq!(stripped["repository_name"], "agentic-os-plan");
        assert_eq!(stripped["title"], "hi");
        assert!(stripped.get("idempotency_key").is_none());
    }

    #[test]
    fn canonical_payload_hash_ignores_object_key_order() {
        let a = strip_idempotency_key(json!({
            "b": 1,
            "a": "x",
            "idempotency_key": "k1"
        }));
        let b = strip_idempotency_key(json!({
            "idempotency_key": "k2",
            "a": "x",
            "b": 1
        }));
        assert_eq!(
            canonical_payload_hash(&a).expect("hash a"),
            canonical_payload_hash(&b).expect("hash b")
        );
    }

    #[test]
    fn canonical_payload_hash_keeps_array_order_significant() {
        let a = json!({"items": [1, 2]});
        let b = json!({"items": [2, 1]});
        assert_ne!(
            canonical_payload_hash(&a).expect("hash a"),
            canonical_payload_hash(&b).expect("hash b")
        );
    }

    #[test]
    fn idempotency_key_hash_is_sha256_of_raw_bytes() {
        let digest = hash_idempotency_key("abc");
        assert_eq!(digest.len(), 32);
        assert_ne!(digest, hash_idempotency_key("ABC"));
    }

    #[test]
    fn repository_coordinate_rejects_uppercase_owner_and_bare_d() {
        let upper = "30617:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA:repo";
        assert!(parse_repository_coordinate(upper).is_err());
        assert!(parse_repository_coordinate("repo").is_err());
        let ok = format!("30617:{}:group:agentic-os-plan", "ab".repeat(32));
        let parsed = parse_repository_coordinate(&ok).expect("valid coordinate");
        assert_eq!(parsed.d_tag(), "group:agentic-os-plan");
        assert_eq!(parsed.as_coordinate(), ok);
    }

    #[test]
    fn alias_target_must_be_full_coordinate_not_clone_url() {
        let yaml = concat!(
            "name: Bad Alias\n",
            "trigger:\n  on: webhook\n",
            "routing:\n  mode: project_channel_by_repository\n",
            "  aliases:\n    plan: https://github.com/acme/agentic-os-plan.git\n",
            "steps:\n  - id: notify\n    action: send_message\n    text: hi\n",
        );
        let err = crate::schema::parse_yaml(yaml).unwrap_err();
        assert!(matches!(err, crate::WorkflowError::InvalidDefinition(_)));
    }
}
