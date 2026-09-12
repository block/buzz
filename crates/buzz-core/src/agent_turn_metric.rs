//! NIP-AM: Agent Turn Metric — payload type and encrypt/decrypt helpers.
//!
//! One `kind:44200` event is published per completed agent turn. Its content
//! is a NIP-44 v2 ciphertext (agent key → owner pubkey) that decodes to an
//! [`AgentTurnMetricPayload`] JSON object.
//!
//! See `docs/nips/NIP-AM.md` for the full specification.

use chrono::DateTime;
use nostr::{Event, Keys, PublicKey};
use serde::{Deserialize, Serialize};

use crate::observer::{decrypt_observer_payload, encrypt_observer_payload, ObserverPayloadError};

// Re-export for callers that only need the error type.
pub use crate::observer::ObserverPayloadError as AgentTurnMetricError;

/// Token-usage counters for a single measurement window (one turn or cumulative).
///
/// All token fields are nullable — `None` means the harness did not report them,
/// NOT that the count was zero. See NIP-AM §Numeric validity and token semantics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenCounts {
    /// Input tokens (inclusive of cache reads/writes where applicable).
    pub input_tokens: Option<u64>,

    /// Output tokens.
    pub output_tokens: Option<u64>,

    /// Provider-reported total — NOT derived by summing input + output.
    /// `None` when the provider did not report a total.
    pub total_tokens: Option<u64>,

    /// Estimated cost in USD. Must be finite and non-negative when present.
    pub cost_usd: Option<f64>,

    /// Informational: cache-read tokens included in `input_tokens`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,

    /// Informational: cache-write tokens included in `input_tokens`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
}

/// Why a turn ended.
///
/// NIP-AM: consumers MUST treat unrecognized `stopReason` values as `Unknown`
/// and keep the token counts valid. Custom deserialization maps any unrecognized
/// string to `Unknown` instead of failing the whole payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Model reached a natural end-of-turn.
    EndTurn,
    /// Model hit the max-tokens limit.
    MaxTokens,
    /// Turn was cancelled by the owner or harness.
    Cancelled,
    /// Turn ended with an error.
    Error,
    /// Stop reason is unknown or unrecognized.
    Unknown,
}

impl<'de> Deserialize<'de> for StopReason {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(match s.as_str() {
            "end_turn" => StopReason::EndTurn,
            "max_tokens" => StopReason::MaxTokens,
            "cancelled" => StopReason::Cancelled,
            "error" => StopReason::Error,
            "unknown" => StopReason::Unknown,
            _ => StopReason::Unknown,
        })
    }
}

/// Billing identity for a turn — present only when the publisher can prove
/// applicability from the actual endpoint and actually-requested model.
///
/// NIP-AM: this is OPTIONAL but NOT nullable. When present, `authority` and
/// `model` MUST be non-null strings; `cache_class` is omitted (not null) when
/// not applicable.
///
/// Consumers MUST treat omission as "price unknown" and MUST NOT infer a price
/// from the session `model` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingIdentity {
    /// Registered billing-namespace identifier: exact lowercase hostname, no
    /// scheme, no path, no trailing slash. Registered values: `api.anthropic.com`,
    /// `api.openai.com`, `openrouter.ai`. Set extends only by NIP amendment.
    pub authority: String,

    /// The billable model identifier as resolved at request time — the
    /// actually-requested model, not the configured/session model alias.
    pub model: String,

    /// Cache-write class (e.g. `"ephemeral"`). Omitted when not applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_class: Option<String>,
}

/// Decrypted payload of a `kind:44200` Agent Turn Metric event.
/// nullable unless constrained by the NIP (e.g. `session_id` + `turn_seq`
/// are required whenever `cumulative` is present).
///
/// Consumers MUST ignore unknown fields (forward compatibility).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnMetricPayload {
    /// Harness identifier (e.g. `"goose"`, `"buzz-agent"`). REQUIRED.
    pub harness: String,

    /// Model identifier as reported by the harness, or `None` if unknown.
    pub model: Option<String>,

    /// Channel UUID the turn served, encrypted inside the payload.
    pub channel_id: Option<String>,

    /// Canonical thread-root event id when the turn used a thread-scoped
    /// session. Omitted for channel-wide conversations, DMs, and heartbeats.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_root_id: Option<String>,

    /// Session identifier. REQUIRED when `cumulative` is present.
    pub session_id: Option<String>,

    /// Turn identifier (harness-internal).
    pub turn_id: Option<String>,

    /// Monotonically increasing per-session sequence number.
    /// REQUIRED when `cumulative` is present; strictly increasing within one
    /// `session_id`. A publisher restart that loses the counter MUST start a
    /// new `session_id`.
    pub turn_seq: Option<u64>,

    /// RFC 3339 timestamp (end-of-turn). REQUIRED.
    pub timestamp: String,

    /// Usage for this turn (computed delta). Null fields mean not reported.
    pub turn: Option<TokenCounts>,

    /// Session-cumulative usage as reported at end of this turn.
    pub cumulative: Option<TokenCounts>,

    /// `false` when the publisher could not observe the previous cumulative
    /// baseline (e.g. harness restart mid-session), making `turn` unreliable.
    /// Defaults to `true` on the wire when not explicitly set.
    #[serde(default = "default_delta_reliable")]
    pub delta_reliable: bool,

    /// Why the turn ended. Unrecognized values MUST be treated as `Unknown`.
    pub stop_reason: Option<StopReason>,

    /// Input-side context tokens in the last successful model request of this
    /// turn. This is a point-in-time context-window snapshot, not cumulative
    /// session usage. Omitted when unknown or reported as zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_used_tokens: Option<u64>,

    /// Configured context-window capacity corresponding to
    /// `context_used_tokens`. Omitted when unknown or reported as zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_limit_tokens: Option<u64>,

    /// Provider-account rate-limit windows reported by the agent runtime.
    /// The payload is encrypted; credentials and bearer tokens never belong
    /// here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub account_usage_windows: Vec<AccountUsageWindow>,

    /// Billing identity, present only when the publisher can prove it from the
    /// actual endpoint (official provider API) and the actually-requested model.
    ///
    /// Omit (never null) when applicability cannot be proven. Consumers MUST
    /// treat omission as "price unknown".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing_identity: Option<PricingIdentity>,
}

/// A provider-account quota window for a native usage gauge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUsageWindow {
    /// Provider-facing label such as `Session`, `Weekly`, or `Sonnet week`.
    pub label: String,
    /// Percentage consumed. Consumers clamp this only for visual rendering.
    pub used_percent: f64,
    /// Provider-reported RFC 3339 reset instant, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<String>,
}

fn default_delta_reliable() -> bool {
    true
}

impl AgentTurnMetricPayload {
    /// Validate the NIP-AM payload contract before encryption or after decryption.
    ///
    /// Returns `Err` when required fields, structural relationships, bounded
    /// strings/windows, RFC 3339 times, pricing identity, context pairs, or
    /// numeric values violate NIP-AM. Token counts are `Option<u64>` and cannot
    /// be negative by construction.
    pub fn validate(&self) -> Result<(), ObserverPayloadError> {
        const MAX_HARNESS_LEN: usize = 128;
        const MAX_IDENTIFIER_LEN: usize = 256;
        const MAX_USAGE_WINDOWS: usize = 64;
        const MAX_WINDOW_LABEL_LEN: usize = 128;

        fn invalid(message: impl Into<String>) -> ObserverPayloadError {
            ObserverPayloadError::InvalidPayload(message.into())
        }

        fn check_optional_text(
            value: Option<&str>,
            field: &str,
            max_len: usize,
        ) -> Result<(), ObserverPayloadError> {
            if let Some(value) = value {
                if value.trim().is_empty() || value.len() > max_len {
                    return Err(invalid(format!(
                        "{field} must be non-empty and at most {max_len} bytes"
                    )));
                }
            }
            Ok(())
        }

        fn check_rfc3339(value: &str, field: &str) -> Result<(), ObserverPayloadError> {
            DateTime::parse_from_rfc3339(value)
                .map(|_| ())
                .map_err(|_| invalid(format!("{field} must be RFC 3339")))
        }

        fn check_cost(cost: Option<f64>, field: &str) -> Result<(), ObserverPayloadError> {
            if let Some(c) = cost {
                if !c.is_finite() || c < 0.0 {
                    return Err(ObserverPayloadError::InvalidPayload(format!(
                        "{field} must be finite and non-negative (got {c})"
                    )));
                }
            }
            Ok(())
        }

        fn counts_have_observation(counts: &TokenCounts) -> bool {
            counts.input_tokens.is_some()
                || counts.output_tokens.is_some()
                || counts.total_tokens.is_some()
                || counts.cost_usd.is_some()
                || counts.cache_read_tokens.is_some()
                || counts.cache_write_tokens.is_some()
        }

        if self.harness.trim().is_empty() || self.harness.len() > MAX_HARNESS_LEN {
            return Err(invalid(format!(
                "harness must be non-empty and at most {MAX_HARNESS_LEN} bytes"
            )));
        }
        check_rfc3339(&self.timestamp, "timestamp")?;
        check_optional_text(self.model.as_deref(), "model", MAX_IDENTIFIER_LEN)?;
        check_optional_text(self.channel_id.as_deref(), "channelId", MAX_IDENTIFIER_LEN)?;
        check_optional_text(self.session_id.as_deref(), "sessionId", MAX_IDENTIFIER_LEN)?;
        check_optional_text(self.turn_id.as_deref(), "turnId", MAX_IDENTIFIER_LEN)?;

        if let Some(thread_root_id) = self.thread_root_id.as_deref() {
            if self.channel_id.is_none()
                || thread_root_id.len() != 64
                || !thread_root_id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(invalid(
                    "threadRootId requires channelId and must be 64 lowercase hex characters",
                ));
            }
        }

        if self.cumulative.is_some() && (self.session_id.is_none() || self.turn_seq.is_none()) {
            return Err(invalid(
                "sessionId and turnSeq are required when cumulative is present",
            ));
        }

        match (self.context_used_tokens, self.context_limit_tokens) {
            (None, None) => {}
            (Some(used), Some(limit)) if used > 0 && limit > 0 => {}
            _ => {
                return Err(invalid(
                    "contextUsedTokens and contextLimitTokens must be a complete positive pair",
                ));
            }
        }

        if let Some(t) = &self.turn {
            check_cost(t.cost_usd, "turn.costUsd")?;
        }
        if let Some(c) = &self.cumulative {
            check_cost(c.cost_usd, "cumulative.costUsd")?;
        }
        if self.account_usage_windows.len() > MAX_USAGE_WINDOWS {
            return Err(invalid(format!(
                "accountUsageWindows must contain at most {MAX_USAGE_WINDOWS} entries"
            )));
        }
        for window in &self.account_usage_windows {
            if window.label.trim().is_empty() || window.label.len() > MAX_WINDOW_LABEL_LEN {
                return Err(invalid(format!(
                    "accountUsageWindows.label must be non-empty and at most {MAX_WINDOW_LABEL_LEN} bytes"
                )));
            }
            if !window.used_percent.is_finite() || window.used_percent < 0.0 {
                return Err(ObserverPayloadError::InvalidPayload(format!(
                    "accountUsageWindows.usedPercent must be finite and non-negative (got {})",
                    window.used_percent
                )));
            }
            if let Some(reset_at) = window.reset_at.as_deref() {
                check_rfc3339(reset_at, "accountUsageWindows.resetAt")?;
            }
        }

        if let Some(pricing) = &self.pricing_identity {
            if !matches!(
                pricing.authority.as_str(),
                "api.anthropic.com" | "api.openai.com" | "openrouter.ai"
            ) {
                return Err(invalid("pricingIdentity.authority is not registered"));
            }
            check_optional_text(
                Some(&pricing.model),
                "pricingIdentity.model",
                MAX_IDENTIFIER_LEN,
            )?;
            check_optional_text(
                pricing.cache_class.as_deref(),
                "pricingIdentity.cacheClass",
                MAX_IDENTIFIER_LEN,
            )?;
        }

        let has_observation = self.turn.as_ref().is_some_and(counts_have_observation)
            || self
                .cumulative
                .as_ref()
                .is_some_and(counts_have_observation)
            || self.context_used_tokens.is_some()
            || !self.account_usage_windows.is_empty();
        if !has_observation {
            return Err(invalid(
                "agent turn metric must contain observed usage, context, or account windows",
            ));
        }
        Ok(())
    }
}

/// Encrypt an [`AgentTurnMetricPayload`] into a NIP-44 v2 ciphertext string
/// using the agent's key pair and the owner's public key.
///
/// Returns `Err(ObserverPayloadError::InvalidPayload)` when the payload fails
/// the complete NIP-AM validation contract.
///
/// This is the content field of a `kind:44200` event.
pub fn encrypt_agent_turn_metric(
    agent_keys: &Keys,
    owner_pubkey: &PublicKey,
    payload: &AgentTurnMetricPayload,
) -> Result<String, ObserverPayloadError> {
    payload.validate()?;
    encrypt_observer_payload(agent_keys, owner_pubkey, payload)
}

/// Decrypt and deserialize an [`AgentTurnMetricPayload`] from a `kind:44200` event.
///
/// `recipient_keys` is the owner's key pair.
///
/// Returns `Err(ObserverPayloadError::InvalidPayload)` if the decrypted payload
/// fails the complete validation contract, mirroring the fail-closed behavior
/// of [`encrypt_agent_turn_metric`].
pub fn decrypt_agent_turn_metric(
    recipient_keys: &Keys,
    event: &Event,
) -> Result<AgentTurnMetricPayload, ObserverPayloadError> {
    validate_agent_turn_metric_envelope(recipient_keys, event)?;
    let payload: AgentTurnMetricPayload = decrypt_observer_payload(recipient_keys, event)?;
    payload.validate()?;
    Ok(payload)
}

/// Validate the signed NIP-AM event envelope before attempting decryption.
pub fn validate_agent_turn_metric_envelope(
    recipient_keys: &Keys,
    event: &Event,
) -> Result<(), ObserverPayloadError> {
    let invalid = |message: &str| ObserverPayloadError::InvalidPayload(message.to_string());
    if event.kind.as_u16() != 44_200 {
        return Err(invalid("event kind must be 44200"));
    }
    if !event.verify_id() || !event.verify_signature() {
        return Err(invalid("event id and signature must be valid"));
    }

    let mut owner_tag: Option<&str> = None;
    let mut agent_tag: Option<&str> = None;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        let Some(name) = parts.first().map(String::as_str) else {
            continue;
        };
        match name {
            "h" => return Err(invalid("h tags are forbidden")),
            "p" if parts.len() != 2 || owner_tag.replace(parts[1].as_str()).is_some() => {
                return Err(invalid("exactly one two-element p tag is required"));
            }
            "p" => {}
            "agent" if parts.len() != 2 || agent_tag.replace(parts[1].as_str()).is_some() => {
                return Err(invalid("exactly one two-element agent tag is required"));
            }
            "agent" => {}
            _ => {}
        }
    }

    let owner_tag = owner_tag.ok_or_else(|| invalid("exactly one p tag is required"))?;
    let agent_tag = agent_tag.ok_or_else(|| invalid("exactly one agent tag is required"))?;
    if owner_tag != recipient_keys.public_key().to_hex() {
        return Err(invalid("p tag must identify the decrypting owner"));
    }
    if agent_tag != event.pubkey.to_hex() {
        return Err(invalid("agent tag must equal the event pubkey"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Kind, Tag};

    fn sample_payload() -> AgentTurnMetricPayload {
        AgentTurnMetricPayload {
            harness: "goose".to_string(),
            model: Some("claude-sonnet-4-5".to_string()),
            channel_id: Some("12345678-1234-1234-1234-123456789abc".to_string()),
            thread_root_id: None,
            session_id: Some("sess-abc".to_string()),
            turn_id: Some("turn-1".to_string()),
            turn_seq: Some(1),
            timestamp: "2026-07-01T20:11:03.213Z".to_string(),
            turn: Some(TokenCounts {
                input_tokens: Some(1234),
                output_tokens: Some(567),
                total_tokens: Some(1801),
                cost_usd: Some(0.0123),
                cache_read_tokens: None,
                cache_write_tokens: None,
            }),
            cumulative: Some(TokenCounts {
                input_tokens: Some(45210),
                output_tokens: Some(9876),
                total_tokens: Some(55086),
                cost_usd: Some(0.41),
                cache_read_tokens: None,
                cache_write_tokens: None,
            }),
            delta_reliable: true,
            stop_reason: Some(StopReason::EndTurn),
            context_used_tokens: None,
            context_limit_tokens: None,
            account_usage_windows: Vec::new(),
            pricing_identity: None,
        }
    }

    #[test]
    fn round_trip_encrypt_decrypt() {
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();

        let payload = sample_payload();
        let ciphertext = encrypt_agent_turn_metric(&agent_keys, &owner_keys.public_key(), &payload)
            .expect("encrypt");

        // Build a minimal event envelope so decrypt_observer_payload can use event.pubkey.
        let event = EventBuilder::new(Kind::Custom(44200), ciphertext)
            .tags([
                Tag::parse(["p", &owner_keys.public_key().to_hex()]).unwrap(),
                Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
            ])
            .sign_with_keys(&agent_keys)
            .expect("sign");

        let decoded = decrypt_agent_turn_metric(&owner_keys, &event).expect("decrypt");

        assert_eq!(decoded, payload);
    }

    #[test]
    fn decrypt_rejects_noncanonical_nip_am_envelopes() {
        let agent_keys = Keys::generate();
        let other_agent = Keys::generate();
        let owner_keys = Keys::generate();
        let wrong_owner = Keys::generate();
        let payload = sample_payload();
        let ciphertext = encrypt_agent_turn_metric(&agent_keys, &owner_keys.public_key(), &payload)
            .expect("encrypt");
        let owner = owner_keys.public_key().to_hex();
        let agent = agent_keys.public_key().to_hex();
        let other_agent = other_agent.public_key().to_hex();
        let wrong_owner = wrong_owner.public_key().to_hex();
        let tag = |parts: &[&str]| Tag::parse(parts.iter().copied()).expect("tag");
        let cases = vec![
            vec![tag(&["p", &owner])],
            vec![tag(&["agent", &agent])],
            vec![
                tag(&["p", &owner]),
                tag(&["p", &owner]),
                tag(&["agent", &agent]),
            ],
            vec![
                tag(&["p", &owner]),
                tag(&["agent", &agent]),
                tag(&["agent", &agent]),
            ],
            vec![tag(&["p", &owner, "extra"]), tag(&["agent", &agent])],
            vec![tag(&["p", &owner]), tag(&["agent", &agent, "extra"])],
            vec![tag(&["p", &wrong_owner]), tag(&["agent", &agent])],
            vec![tag(&["p", &owner]), tag(&["agent", &other_agent])],
            vec![
                tag(&["p", &owner]),
                tag(&["agent", &agent]),
                tag(&["h", "channel"]),
            ],
        ];

        for tags in cases {
            let event = EventBuilder::new(Kind::Custom(44200), ciphertext.clone())
                .tags(tags)
                .sign_with_keys(&agent_keys)
                .expect("sign");
            assert!(decrypt_agent_turn_metric(&owner_keys, &event).is_err());
        }
    }

    #[test]
    fn wrong_key_decrypt_fails() {
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let wrong_keys = Keys::generate();

        let payload = sample_payload();
        let ciphertext = encrypt_agent_turn_metric(&agent_keys, &owner_keys.public_key(), &payload)
            .expect("encrypt");

        let event = EventBuilder::new(Kind::Custom(44200), ciphertext)
            .tags([
                Tag::parse(["p", &owner_keys.public_key().to_hex()]).unwrap(),
                Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
            ])
            .sign_with_keys(&agent_keys)
            .expect("sign");

        let result = decrypt_agent_turn_metric(&wrong_keys, &event);
        assert!(result.is_err(), "expected decrypt error with wrong key");
    }

    #[test]
    fn delta_reliable_defaults_to_true_when_absent() {
        let json = r#"{"harness":"goose","timestamp":"2026-07-01T20:11:03Z"}"#;
        let payload: AgentTurnMetricPayload = serde_json::from_str(json).expect("parse");
        assert!(
            payload.delta_reliable,
            "deltaReliable should default to true"
        );
    }

    #[test]
    fn context_snapshot_and_thread_scope_use_optional_camel_case_fields() {
        let json = r#"{
            "harness":"goose",
            "timestamp":"2026-07-01T20:11:03Z",
            "contextUsedTokens":210000,
            "contextLimitTokens":200000,
            "accountUsageWindows":[
                {"label":"Session","usedPercent":12.5,"resetAt":"2026-07-02T01:00:00Z"},
                {"label":"Weekly","usedPercent":41.0}
            ],
            "threadRootId":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }"#;
        let payload: AgentTurnMetricPayload = serde_json::from_str(json).expect("parse");

        assert_eq!(payload.context_used_tokens, Some(210_000));
        assert_eq!(payload.context_limit_tokens, Some(200_000));
        assert_eq!(payload.account_usage_windows.len(), 2);
        assert_eq!(payload.account_usage_windows[0].label, "Session");
        assert_eq!(payload.account_usage_windows[0].used_percent, 12.5);
        assert_eq!(
            payload.account_usage_windows[0].reset_at.as_deref(),
            Some("2026-07-02T01:00:00Z")
        );
        assert_eq!(
            payload.thread_root_id.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );

        let encoded = serde_json::to_value(payload).expect("serialize");
        assert_eq!(encoded["contextUsedTokens"], 210_000);
        assert_eq!(encoded["contextLimitTokens"], 200_000);
        assert_eq!(
            encoded["threadRootId"],
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }

    #[test]
    fn legacy_payload_omits_new_optional_context_and_thread_fields() {
        let json = r#"{"harness":"goose","timestamp":"2026-07-01T20:11:03Z"}"#;
        let payload: AgentTurnMetricPayload = serde_json::from_str(json).expect("parse");

        assert_eq!(payload.context_used_tokens, None);
        assert_eq!(payload.context_limit_tokens, None);
        assert_eq!(payload.thread_root_id, None);

        let encoded = serde_json::to_value(payload).expect("serialize");
        assert!(encoded.get("contextUsedTokens").is_none());
        assert!(encoded.get("contextLimitTokens").is_none());
        assert!(encoded.get("threadRootId").is_none());
    }

    #[test]
    fn stop_reason_round_trips() {
        for (variant, json_val) in [
            (StopReason::EndTurn, "\"end_turn\""),
            (StopReason::MaxTokens, "\"max_tokens\""),
            (StopReason::Cancelled, "\"cancelled\""),
            (StopReason::Error, "\"error\""),
            (StopReason::Unknown, "\"unknown\""),
        ] {
            let serialized = serde_json::to_string(&variant).unwrap();
            assert_eq!(serialized, json_val);
            let deserialized: StopReason = serde_json::from_str(json_val).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn null_token_counts_round_trip() {
        // Verify that None fields serialize to `null` (not absent), as required
        // by the NIP — consumers must distinguish "not reported" from "zero".
        let counts = TokenCounts {
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            cost_usd: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        };
        let json = serde_json::to_string(&counts).unwrap();
        // cache_* are skip_serializing_if = None, others serialize as null
        assert!(json.contains("\"inputTokens\":null"));
        assert!(json.contains("\"outputTokens\":null"));
        let back: TokenCounts = serde_json::from_str(&json).unwrap();
        assert_eq!(back, counts);
    }

    #[test]
    fn unknown_stop_reason_maps_to_unknown_not_error() {
        // NIP-AM: consumers MUST treat unrecognized stopReason values as Unknown;
        // the token counts remain valid and the whole payload must not be rejected.
        let json = r#"{
            "harness": "goose",
            "timestamp": "2026-07-01T20:11:03Z",
            "stopReason": "tool_limit",
            "turn": {
                "inputTokens": 1234,
                "outputTokens": 567,
                "totalTokens": 1801,
                "costUsd": null
            }
        }"#;
        let payload: AgentTurnMetricPayload =
            serde_json::from_str(json).expect("payload with future stopReason must parse");
        assert_eq!(
            payload.stop_reason,
            Some(StopReason::Unknown),
            "unrecognized stopReason must map to Unknown"
        );
        // Token counts must be preserved.
        let turn = payload.turn.expect("turn must be present");
        assert_eq!(turn.input_tokens, Some(1234));
        assert_eq!(turn.output_tokens, Some(567));
        assert_eq!(turn.total_tokens, Some(1801));
    }

    // ── validate() — negative / non-finite costUsd ─────────────────────────

    fn make_payload_with_turn_cost(cost: Option<f64>) -> AgentTurnMetricPayload {
        AgentTurnMetricPayload {
            harness: "test".to_string(),
            model: None,
            channel_id: None,
            thread_root_id: None,
            session_id: None,
            turn_id: None,
            turn_seq: None,
            timestamp: "2026-07-01T00:00:00Z".to_string(),
            turn: Some(TokenCounts {
                input_tokens: Some(100),
                output_tokens: Some(50),
                total_tokens: None,
                cost_usd: cost,
                cache_read_tokens: None,
                cache_write_tokens: None,
            }),
            cumulative: None,
            delta_reliable: true,
            stop_reason: None,
            context_used_tokens: None,
            context_limit_tokens: None,
            account_usage_windows: Vec::new(),
            pricing_identity: None,
        }
    }

    fn make_payload_with_cumulative_cost(cost: Option<f64>) -> AgentTurnMetricPayload {
        AgentTurnMetricPayload {
            harness: "test".to_string(),
            model: None,
            channel_id: None,
            thread_root_id: None,
            session_id: None,
            turn_id: None,
            turn_seq: None,
            timestamp: "2026-07-01T00:00:00Z".to_string(),
            turn: None,
            cumulative: Some(TokenCounts {
                input_tokens: Some(500),
                output_tokens: Some(200),
                total_tokens: None,
                cost_usd: cost,
                cache_read_tokens: None,
                cache_write_tokens: None,
            }),
            delta_reliable: true,
            stop_reason: None,
            context_used_tokens: None,
            context_limit_tokens: None,
            account_usage_windows: Vec::new(),
            pricing_identity: None,
        }
    }

    #[test]
    fn validate_rejects_negative_turn_cost() {
        let payload = make_payload_with_turn_cost(Some(-0.001));
        assert!(
            matches!(
                payload.validate(),
                Err(ObserverPayloadError::InvalidPayload(_))
            ),
            "negative turn.costUsd must be rejected"
        );
    }

    #[test]
    fn validate_rejects_nan_turn_cost() {
        let payload = make_payload_with_turn_cost(Some(f64::NAN));
        assert!(
            matches!(
                payload.validate(),
                Err(ObserverPayloadError::InvalidPayload(_))
            ),
            "NaN turn.costUsd must be rejected"
        );
    }

    #[test]
    fn validate_rejects_infinite_turn_cost() {
        let payload = make_payload_with_turn_cost(Some(f64::INFINITY));
        assert!(
            matches!(
                payload.validate(),
                Err(ObserverPayloadError::InvalidPayload(_))
            ),
            "infinite turn.costUsd must be rejected"
        );
    }

    #[test]
    fn validate_rejects_negative_cumulative_cost() {
        let payload = make_payload_with_cumulative_cost(Some(-1.0));
        assert!(
            matches!(
                payload.validate(),
                Err(ObserverPayloadError::InvalidPayload(_))
            ),
            "negative cumulative.costUsd must be rejected"
        );
    }

    #[test]
    fn validate_accepts_finite_non_negative_cost() {
        // Zero, small, and larger values are all valid.
        for cost in [0.0_f64, 0.001, 1.0, 999.99] {
            let payload = make_payload_with_turn_cost(Some(cost));
            assert!(payload.validate().is_ok(), "cost {cost} should be accepted");
        }
    }

    #[test]
    fn validate_accepts_absent_cost() {
        let payload = make_payload_with_turn_cost(None);
        assert!(
            payload.validate().is_ok(),
            "absent costUsd must be accepted"
        );
    }

    #[test]
    fn encrypt_agent_turn_metric_rejects_negative_cost() {
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let payload = make_payload_with_turn_cost(Some(-0.5));
        let result = encrypt_agent_turn_metric(&agent_keys, &owner_keys.public_key(), &payload);
        assert!(
            matches!(result, Err(ObserverPayloadError::InvalidPayload(_))),
            "encrypt must reject payload with negative costUsd"
        );
    }

    #[test]
    fn validate_enforces_structural_nip_am_contract() {
        let mut cases = Vec::new();

        let mut payload = sample_payload();
        payload.harness = " ".to_string();
        cases.push(("empty harness", payload));

        let mut payload = sample_payload();
        payload.timestamp = "not-a-time".to_string();
        cases.push(("invalid timestamp", payload));

        let mut payload = sample_payload();
        payload.session_id = None;
        cases.push(("cumulative without session", payload));

        let mut payload = sample_payload();
        payload.context_used_tokens = Some(10);
        payload.context_limit_tokens = None;
        cases.push(("one-sided context", payload));

        let mut payload = sample_payload();
        payload.context_used_tokens = Some(0);
        payload.context_limit_tokens = Some(100);
        cases.push(("zero context", payload));

        let mut payload = sample_payload();
        payload.channel_id = None;
        payload.thread_root_id = Some("a".repeat(64));
        cases.push(("thread without channel", payload));

        let mut payload = sample_payload();
        payload.account_usage_windows = vec![AccountUsageWindow {
            label: "Session".to_string(),
            used_percent: 10.0,
            reset_at: Some("tomorrow".to_string()),
        }];
        cases.push(("invalid reset time", payload));

        let mut payload = sample_payload();
        payload.account_usage_windows = vec![AccountUsageWindow {
            label: " ".to_string(),
            used_percent: 10.0,
            reset_at: None,
        }];
        cases.push(("empty window label", payload));

        let mut payload = sample_payload();
        payload.account_usage_windows = (0..65)
            .map(|index| AccountUsageWindow {
                label: format!("Window {index}"),
                used_percent: 10.0,
                reset_at: None,
            })
            .collect();
        cases.push(("too many windows", payload));

        let mut payload = sample_payload();
        payload.pricing_identity = Some(PricingIdentity {
            authority: "example.com".to_string(),
            model: "model".to_string(),
            cache_class: None,
        });
        cases.push(("unregistered pricing authority", payload));

        let mut payload = sample_payload();
        payload.turn = None;
        payload.cumulative = None;
        payload.session_id = None;
        payload.turn_seq = None;
        payload.context_used_tokens = None;
        payload.context_limit_tokens = None;
        payload.account_usage_windows.clear();
        cases.push(("informationless snapshot", payload));

        for (label, payload) in cases {
            assert!(payload.validate().is_err(), "{label} must be rejected");
        }
    }

    #[test]
    fn validate_accepts_quota_only_snapshot() {
        let mut payload = sample_payload();
        payload.turn = None;
        payload.cumulative = None;
        payload.session_id = None;
        payload.turn_seq = None;
        payload.account_usage_windows = vec![AccountUsageWindow {
            label: "Weekly".to_string(),
            used_percent: 42.0,
            reset_at: Some("2026-07-02T01:00:00Z".to_string()),
        }];
        assert!(payload.validate().is_ok());
    }

    #[test]
    fn decrypt_agent_turn_metric_rejects_negative_cost_bypassing_encrypt() {
        // Regression: a raw/misbehaving agent can persist a syntactically valid
        // NIP-44 payload with costUsd: -1 by calling encrypt_observer_payload
        // directly (bypassing the validating encrypt_agent_turn_metric helper).
        // decrypt_agent_turn_metric must reject it symmetrically.
        use crate::observer::encrypt_observer_payload;

        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();

        // Build a payload with negative costUsd and encrypt via the lower-level
        // path, bypassing encrypt_agent_turn_metric's validate() call.
        let bad_payload = make_payload_with_turn_cost(Some(-1.0));
        let ciphertext =
            encrypt_observer_payload(&agent_keys, &owner_keys.public_key(), &bad_payload)
                .expect("lower-level encrypt should succeed without validation");

        let event = EventBuilder::new(Kind::Custom(44200), ciphertext)
            .tags([
                Tag::parse(["p", &owner_keys.public_key().to_hex()]).unwrap(),
                Tag::parse(["agent", &agent_keys.public_key().to_hex()]).unwrap(),
            ])
            .sign_with_keys(&agent_keys)
            .expect("sign");

        let result = decrypt_agent_turn_metric(&owner_keys, &event);
        assert!(
            matches!(result, Err(ObserverPayloadError::InvalidPayload(_))),
            "decrypt must reject a payload with negative costUsd even when \
             encrypted via the lower-level path"
        );
    }
}
