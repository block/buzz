//! Buzz trusted-operation broker — wire protocol.
//!
//! The broker lets a requester ask an owner's host to perform a small set of
//! named business operations ("capabilities") that require the owner's
//! credentials. It exposes business capabilities only: never signing,
//! publishing, credential access, or arbitrary tool execution.
//!
//! # Mental model
//!
//! ```text
//! requester → BrokerRequest → (signed, encrypted frame) → host broker
//!   host: verify → authorize → validate → claim → dispatch → BrokerResult
//! ```
//!
//! # What is *not* in this envelope
//!
//! Owner, requester, and relay scope are **transport-derived** on the host from
//! the verified frame. They are deliberately absent here: a request body that
//! could name its own owner would let any signer act on another owner's agents.
//!
//! There is no `authorization` field yet. One gets added, as a discriminated
//! object, when a real grant format and verifier exist — not before, so that no
//! field looks security-bearing while enforcing nothing.
//!
//! There is no client-computed digest. Idempotency is decided host-side; see
//! [`BrokerRequest`] for the retry contract.

use serde::{Deserialize, Serialize};

use crate::SdkError;

pub mod capabilities;

pub use capabilities::{
    AgentTarget, AgentsCreateArgs, AgentsCreateOutcome, AgentsDeleteArgs, AgentsDeleteOutcome,
    AgentsUpdateArgs, AgentsUpdateOutcome, Capability, CapabilityArgs, CapabilityOutcome,
};

/// Wire `type` discriminator for a broker request payload.
pub const BROKER_REQUEST_TYPE: &str = "broker_request";

/// Wire `type` discriminator for a broker result payload.
pub const BROKER_RESULT_TYPE: &str = "broker_result";

/// Current broker protocol version.
///
/// There is no "absent means 1" compatibility rule: the protocol is unshipped,
/// so `protocolVersion` is required and an unknown value is rejected outright.
pub const BROKER_PROTOCOL_VERSION: u16 = 1;

/// Maximum accepted length of a `requestId`, in bytes.
pub const MAX_REQUEST_ID_LEN: usize = 128;

/// A request to execute one broker capability.
///
/// # Retry contract
///
/// Retrying an operation means **resending the identical serialized request**
/// with the same `requestId`. The host hashes what it receives and compares
/// that digest against the digest recorded under the same idempotency key:
///
/// - same key, same digest → the recorded outcome is replayed, nothing re-runs
/// - same key, different digest → rejected as a request-ID conflict
///
/// Callers therefore must not regenerate, re-order, or re-render the payload
/// between attempts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrokerRequest {
    /// Payload discriminator — must equal [`BROKER_REQUEST_TYPE`].
    pub r#type: String,
    /// Protocol version — must equal [`BROKER_PROTOCOL_VERSION`].
    pub protocol_version: u16,
    /// Caller-chosen idempotency key, unique per logical operation.
    pub request_id: String,
    /// Capability contract version the caller wrote `args` against.
    pub capability_version: u16,
    /// The capability to invoke, with its strictly typed arguments.
    #[serde(flatten)]
    pub capability: CapabilityArgs,
}

impl BrokerRequest {
    /// Build a request for `capability` with the current protocol version.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] if `request_id` is empty, longer than
    /// [`MAX_REQUEST_ID_LEN`], or contains non-printable-ASCII bytes.
    pub fn new(
        request_id: impl Into<String>,
        capability: CapabilityArgs,
    ) -> Result<Self, SdkError> {
        let request_id = request_id.into();
        let request = Self {
            r#type: BROKER_REQUEST_TYPE.to_string(),
            protocol_version: BROKER_PROTOCOL_VERSION,
            request_id,
            capability_version: capability.capability().current_version(),
            capability,
        };
        request.validate()?;
        Ok(request)
    }

    /// The capability this request invokes.
    #[must_use]
    pub fn capability(&self) -> Capability {
        self.capability.capability()
    }

    /// Validate every field the host must agree on before executing anything.
    ///
    /// Callers validate before sealing a frame; the host revalidates after
    /// decrypting, because only the host's verdict is authoritative.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] for a wrong `type`, an unsupported
    /// `protocolVersion` or `capabilityVersion`, a malformed `requestId`, or
    /// capability arguments that fail their own validation.
    pub fn validate(&self) -> Result<(), SdkError> {
        if self.r#type != BROKER_REQUEST_TYPE {
            return Err(SdkError::InvalidInput(format!(
                "broker request type must be \"{BROKER_REQUEST_TYPE}\", got \"{}\"",
                self.r#type
            )));
        }
        if self.protocol_version != BROKER_PROTOCOL_VERSION {
            return Err(SdkError::InvalidInput(format!(
                "unsupported broker protocolVersion {} (expected {BROKER_PROTOCOL_VERSION})",
                self.protocol_version
            )));
        }
        validate_request_id(&self.request_id)?;
        let capability = self.capability();
        if self.capability_version != capability.current_version() {
            return Err(SdkError::InvalidInput(format!(
                "unsupported capabilityVersion {} for {} (expected {})",
                self.capability_version,
                capability.as_str(),
                capability.current_version()
            )));
        }
        self.capability.validate()
    }
}

/// Validate a `requestId`: non-empty, bounded, printable ASCII.
///
/// The bound and character set exist because this value becomes part of a
/// durable primary key and appears in audit records.
///
/// # Errors
///
/// Returns [`SdkError::InvalidInput`] when the id is empty, exceeds
/// [`MAX_REQUEST_ID_LEN`] bytes, or contains a byte outside `0x21..=0x7e`.
pub fn validate_request_id(request_id: &str) -> Result<(), SdkError> {
    if request_id.is_empty() {
        return Err(SdkError::InvalidInput("requestId must not be empty".into()));
    }
    if request_id.len() > MAX_REQUEST_ID_LEN {
        return Err(SdkError::InvalidInput(format!(
            "requestId exceeds {MAX_REQUEST_ID_LEN} bytes (got {})",
            request_id.len()
        )));
    }
    if let Some(bad) = request_id
        .bytes()
        .find(|b| !(0x21..=0x7e).contains(b))
        .map(|b| format!("0x{b:02x}"))
    {
        return Err(SdkError::InvalidInput(format!(
            "requestId must be printable ASCII without spaces (found byte {bad})"
        )));
    }
    Ok(())
}

/// Machine-readable broker error code.
///
/// These name failures the *broker* is responsible for. Capability-specific
/// failures arrive as [`BrokerErrorCode::CapabilityFailed`] with detail in the
/// accompanying message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerErrorCode {
    /// The envelope or capability arguments failed validation.
    InvalidRequest,
    /// The `protocolVersion` is not supported by this host.
    UnsupportedProtocolVersion,
    /// The capability name is unknown to this host.
    UnknownCapability,
    /// The `capabilityVersion` is not supported for this capability.
    UnsupportedCapabilityVersion,
    /// The requester's signature or frame could not be verified.
    Unauthenticated,
    /// The requester is authenticated but not permitted this operation.
    Unauthorized,
    /// Reuse of a `requestId` with different request content.
    RequestIdConflict,
    /// The capability handler ran and reported a domain failure.
    CapabilityFailed,
    /// The host could not determine whether side effects occurred.
    ///
    /// Only ever paired with [`BrokerResult::Indeterminate`].
    OutcomeUnknown,
    /// An unexpected host-side fault.
    Internal,
}

impl BrokerErrorCode {
    /// Stable wire string for this code.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::UnsupportedProtocolVersion => "unsupported_protocol_version",
            Self::UnknownCapability => "unknown_capability",
            Self::UnsupportedCapabilityVersion => "unsupported_capability_version",
            Self::Unauthenticated => "unauthenticated",
            Self::Unauthorized => "unauthorized",
            Self::RequestIdConflict => "request_id_conflict",
            Self::CapabilityFailed => "capability_failed",
            Self::OutcomeUnknown => "outcome_unknown",
            Self::Internal => "internal",
        }
    }
}

/// A broker error: a machine-readable code plus a human-readable message.
///
/// Messages are for operators and must never carry secrets — no nsec, no
/// credentials, no decrypted payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerError {
    /// Machine-readable failure code.
    pub code: BrokerErrorCode,
    /// Operator-facing description. Secret-free.
    pub message: String,
}

impl BrokerError {
    /// Construct an error from a code and message.
    pub fn new(code: BrokerErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// An [`BrokerErrorCode::InvalidRequest`] error.
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(BrokerErrorCode::InvalidRequest, message)
    }

    /// An [`BrokerErrorCode::Unauthorized`] error.
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(BrokerErrorCode::Unauthorized, message)
    }
}

/// The terminal disposition of a broker request.
///
/// A discriminated union, so "succeeded with an error" and "failed with an
/// outcome" are unrepresentable rather than merely discouraged.
///
/// [`Self::Indeterminate`] is distinct from [`Self::Failed`] on purpose:
/// `Failed` promises no side effects took hold, while `Indeterminate` promises
/// nothing at all and demands operator or capability-level reconciliation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BrokerResult {
    /// The capability completed and produced this outcome.
    Succeeded {
        /// Capability-specific success payload.
        #[serde(flatten)]
        outcome: CapabilityOutcome,
    },
    /// The capability did not complete; no side effects are expected to persist.
    Failed {
        /// Why it failed.
        error: BrokerError,
    },
    /// Whether side effects occurred could not be determined.
    Indeterminate {
        /// What is unknown, and why.
        error: BrokerError,
    },
}

impl BrokerResult {
    /// A successful result carrying `outcome`.
    #[must_use]
    pub fn succeeded(outcome: CapabilityOutcome) -> Self {
        Self::Succeeded { outcome }
    }

    /// A failed result carrying `error`.
    #[must_use]
    pub fn failed(error: BrokerError) -> Self {
        Self::Failed { error }
    }

    /// An indeterminate result carrying `error`.
    #[must_use]
    pub fn indeterminate(error: BrokerError) -> Self {
        Self::Indeterminate { error }
    }

    /// Whether this is a terminal success.
    #[must_use]
    pub fn is_succeeded(&self) -> bool {
        matches!(self, Self::Succeeded { .. })
    }

    /// The error, for the two non-success variants.
    #[must_use]
    pub fn error(&self) -> Option<&BrokerError> {
        match self {
            Self::Succeeded { .. } => None,
            Self::Failed { error } | Self::Indeterminate { error } => Some(error),
        }
    }
}

/// A broker result addressed back to the requester.
///
/// `replayed` is **response metadata**: it describes this delivery, not the
/// domain outcome, and is never persisted as part of the stored result. A
/// replayed response is byte-identical in `result` to the original.
///
/// Note: this struct cannot use `deny_unknown_fields`, because serde does not
/// support combining it with `#[serde(flatten)]`. Strictness is enforced where
/// it guards execution — on [`BrokerRequest`] and each capability args type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokerResponse {
    /// Payload discriminator — must equal [`BROKER_RESULT_TYPE`].
    pub r#type: String,
    /// Protocol version — must equal [`BROKER_PROTOCOL_VERSION`].
    pub protocol_version: u16,
    /// Correlates with the originating [`BrokerRequest::request_id`].
    pub request_id: String,
    /// The terminal disposition.
    #[serde(flatten)]
    pub result: BrokerResult,
    /// True when this response replays a previously recorded outcome.
    #[serde(default, skip_serializing_if = "is_false")]
    pub replayed: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl BrokerResponse {
    /// Build a fresh (non-replayed) response for `request_id`.
    pub fn new(request_id: impl Into<String>, result: BrokerResult) -> Self {
        Self {
            r#type: BROKER_RESULT_TYPE.to_string(),
            protocol_version: BROKER_PROTOCOL_VERSION,
            request_id: request_id.into(),
            result,
            replayed: false,
        }
    }

    /// Mark this response as replaying a recorded outcome.
    #[must_use]
    pub fn replayed(mut self) -> Self {
        self.replayed = true;
        self
    }

    /// Validate discriminator, version, and request id.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] on a wrong `type`, an unsupported
    /// `protocolVersion`, or a malformed `requestId`.
    pub fn validate(&self) -> Result<(), SdkError> {
        if self.r#type != BROKER_RESULT_TYPE {
            return Err(SdkError::InvalidInput(format!(
                "broker result type must be \"{BROKER_RESULT_TYPE}\", got \"{}\"",
                self.r#type
            )));
        }
        if self.protocol_version != BROKER_PROTOCOL_VERSION {
            return Err(SdkError::InvalidInput(format!(
                "unsupported broker protocolVersion {} (expected {BROKER_PROTOCOL_VERSION})",
                self.protocol_version
            )));
        }
        validate_request_id(&self.request_id)
    }
}

#[cfg(test)]
mod tests;
