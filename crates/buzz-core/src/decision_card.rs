//! Typed payloads for channel-native decision cards and their durable responses.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Current wire schema for decision-card payloads.
pub const DECISION_CARD_SCHEMA_VERSION: u8 = 1;

/// A human choice exposed by a decision card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionCardChoice {
    /// Accept the proposed action.
    Approve,
    /// Ask for a revised proposal.
    Redraft,
    /// Route the case to a higher-authority reviewer.
    Escalate,
    /// Reject the proposed action.
    Reject,
}

/// Structured data carried by a `kind:40009` decision-card event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionCardPayload {
    /// Payload schema version.
    pub schema_version: u8,
    /// Stable business-level card identifier.
    pub card_id: Uuid,
    /// Short decision title.
    pub title: String,
    /// Concise current situation.
    pub situation: String,
    /// Recommended choice and why.
    pub recommendation: String,
    /// Exact action that the decision would authorize as intent.
    pub proposed_action: String,
    /// Material risk or consequence.
    pub risk: String,
    /// Optional authoritative-record URL.
    pub record_url: Option<String>,
    /// Ordered choices shown to the human.
    pub choices: Vec<DecisionCardChoice>,
    /// Optional Unix-seconds expiry.
    pub expires_at: Option<i64>,
    /// Whether this card is explicitly non-production.
    pub shadow: bool,
}

impl DecisionCardPayload {
    /// Validate the bounded wire contract.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != DECISION_CARD_SCHEMA_VERSION {
            return Err("unsupported decision card schema version");
        }
        if self.title.trim().is_empty()
            || self.situation.trim().is_empty()
            || self.recommendation.trim().is_empty()
            || self.proposed_action.trim().is_empty()
            || self.risk.trim().is_empty()
        {
            return Err("decision card text fields must not be empty");
        }
        if self.title.len() > 160
            || self.situation.len() > 2_000
            || self.recommendation.len() > 2_000
            || self.proposed_action.len() > 2_000
            || self.risk.len() > 2_000
        {
            return Err("decision card text field exceeds its size limit");
        }
        if self
            .record_url
            .as_ref()
            .is_some_and(|record_url| record_url.len() > 2_048)
        {
            return Err("decision card record URL exceeds its size limit");
        }
        if let Some(record_url) = &self.record_url {
            let parsed = url::Url::parse(record_url)
                .map_err(|_| "decision card record URL must be an absolute HTTP(S) URL")?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err("decision card record URL must be an absolute HTTP(S) URL");
            }
        }
        if self.choices.is_empty() || self.choices.len() > 4 {
            return Err("decision card must expose between one and four choices");
        }
        let unique: std::collections::HashSet<_> = self.choices.iter().collect();
        if unique.len() != self.choices.len() {
            return Err("decision card choices must be unique");
        }
        Ok(())
    }

    /// Serialize the payload in its canonical field order.
    pub fn canonical_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// SHA-256 digest of the canonical structured payload.
    pub fn payload_hash(&self) -> Result<String, serde_json::Error> {
        let encoded = self.canonical_json()?;
        Ok(hex::encode(Sha256::digest(encoded.as_bytes())))
    }
}

/// Structured data carried by a `kind:40010` decision-response event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionResponsePayload {
    /// Payload schema version.
    pub schema_version: u8,
    /// Idempotency identifier for this response intent.
    pub action_id: Uuid,
    /// Stable business-level card identifier.
    pub card_id: Uuid,
    /// Human choice.
    pub decision: DecisionCardChoice,
    /// Digest of the exact card payload the human saw.
    pub payload_hash: String,
    /// Optional human note.
    pub note: Option<String>,
    /// Whether this response is explicitly non-production.
    pub shadow: bool,
}

impl DecisionResponsePayload {
    /// Validate the bounded response contract.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != DECISION_CARD_SCHEMA_VERSION {
            return Err("unsupported decision response schema version");
        }
        if self.payload_hash.len() != 64
            || !self
                .payload_hash
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Err("decision response payload hash must be 64 hexadecimal characters");
        }
        if self.note.as_ref().is_some_and(|note| note.len() > 2_000) {
            return Err("decision response note exceeds its size limit");
        }
        Ok(())
    }
}
