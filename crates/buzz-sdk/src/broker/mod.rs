//! Agent ↔ broker contract — the operations an agent asks a host to perform.
//!
//! This module is a **contract only**: the request envelope, the closed set of
//! [`Action`]s, the result shape, the HTTP binding, and a client trait. No
//! host, no transport, no signing. The full design rationale lives in the
//! English spec (`docs/agent-broker.md`); doc comments here explain only what
//! the code cannot say itself.
//!
//! ```text
//! agent → BrokerRequest → (POST /v1/action, bearer credential) → host
//!   host: authenticate → authorize → validate → execute → BrokerResponse
//! ```
//!
//! The agent holds its public key and a session credential — no secret key, no
//! relay connection. Everything it wants to do, reading included, is an action.
//! Actions are named business operations rather than a `sign(bytes)` primitive
//! so a host can hold per-operation policy; that and the rest of the
//! [#6467](https://github.com/block/buzz/issues/6467) mapping are covered in
//! the spec.
//!
//! # Contract-wide rules
//!
//! - **No secret crosses this boundary, in either direction.** Every wire type
//!   is strict — unknown members are rejected at every depth, and each type's
//!   exact key set is pinned by test. Where a derive would have left a lax
//!   reader ([`BrokerResponse`], [`BrokerMessage`], [`BrokerResult`]), the type
//!   documents how its strictness is restored.
//! - **Identities have exactly one spelling.** UUIDs and hex admit several
//!   legal spellings, so every identity is canonicalized at both doors —
//!   `validated()` and deserialization. See [`actions`]'s shared validators.
//! - **Omission is the only spelling of absence.** An explicit `null` is
//!   rejected anywhere, at any depth. Canonical rationale on the
//!   `absent_or_valued` guard in [`actions`]; host implementers must configure
//!   serializers to omit unset members.
//! - **No request names its own subject.** Requester, owner, and scope are
//!   derived from the authenticated credential; see [`BrokerRequest`]. This is
//!   also why `agents.create` has no owner field — the creator owns the agent,
//!   and the ownership chain always terminates at a human. Bounding its depth
//!   is a host concern.
//!
//! Two limits worth stating: a `String` field can physically hold secret text,
//! so keeping secrets out of content and error messages is host policy; and
//! nothing stops a host from *holding* keys — that is the point. It stops one
//! from handing them over.
//!
//! # Deferred operations
//!
//! Not in v1, all purely additive later: memory read/write (intent-level
//! operations over the encrypted store — until then [`Action::StorageAddress`]
//! only addresses a record, and the key holder remains the only reader/writer),
//! `presence.set`, `typing.set`, and streaming reads (waking on a mention is
//! `channel.read` with `mentionsOnly`, polled).
//!
//! # Non-goals
//!
//! Hosts (auth, idempotency storage, execution), transports, relay changes,
//! grant/authorization fields (added when a real verifier exists, not before),
//! and secret-key custody. [`BrokerClient`] exists so in-process and HTTP
//! implementations are interchangeable; neither is here.

use serde::{Deserialize, Serialize};

use crate::SdkError;

pub mod actions;
pub mod client;
mod correlate;
mod wire;

use actions::absent_or_valued;
pub use actions::{
    Action, ActionArgs, ActionOutcome, AgentTarget, AgentsCreateArgs, AgentsCreateOutcome,
    AgentsDeleteArgs, AgentsDeleteOutcome, AgentsUpdateArgs, AgentsUpdateOutcome, BrokerMessage,
    ChannelReadArgs, EventPublished, MessagePage, MessagePostArgs, MessageReplyArgs,
    ProfileSetArgs, PubkeyHex, ReactionAddArgs, StorageAddress, StorageAddressArgs,
};
pub use client::{
    BrokerClient, BrokerClientExt, BrokerFuture, BrokerTransportError, Dispatch, ValidatedFuture,
    ValidatedResponse, BROKER_ACTION_PATH, BROKER_CREDENTIAL_HEADER,
};

/// Wire `type` discriminator for a broker request payload.
pub const BROKER_REQUEST_TYPE: &str = "broker_request";

/// Wire `type` discriminator for a broker response payload.
pub const BROKER_RESULT_TYPE: &str = "broker_result";

/// Current broker protocol version.
///
/// There is no "absent means 1" compatibility rule: the protocol is unshipped,
/// so `protocolVersion` is required and an unknown value is rejected outright.
pub const BROKER_PROTOCOL_VERSION: u16 = 1;

/// Maximum accepted length of a `requestId`, in bytes.
pub const MAX_REQUEST_ID_LEN: usize = 128;

/// A request to execute one broker action.
///
/// There is deliberately no requester, owner, scope, or relay field: those are
/// derived by the host from the authenticated session credential. **A body
/// that could name its own subject would let any caller act as anyone.**
///
/// # Retry contract
///
/// Retrying means resending the identical bytes with the same `requestId` —
/// the host compares a digest of the bytes against what it recorded under that
/// idempotency key (same digest → replay the recorded outcome; different →
/// [`BrokerErrorCode::RequestIdConflict`]). Two serializations of one value
/// can differ in bytes, so a client never sends this type directly: call
/// [`Self::prepare`] to freeze it into a [`PreparedRequest`] and hand *that*
/// to [`BrokerClientExt::execute`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrokerRequest {
    /// Payload discriminator — must equal [`BROKER_REQUEST_TYPE`].
    pub r#type: String,
    /// Protocol version — must equal [`BROKER_PROTOCOL_VERSION`].
    pub protocol_version: u16,
    /// Caller-chosen idempotency key, unique per logical operation.
    pub request_id: String,
    /// Action contract version the caller wrote `args` against.
    pub action_version: u16,
    /// The action to invoke, with its strictly typed arguments.
    #[serde(flatten)]
    pub action: ActionArgs,
}

impl BrokerRequest {
    /// Build a request for `action` at the current protocol version.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] if `request_id` is empty, longer than
    /// [`MAX_REQUEST_ID_LEN`], or not printable ASCII, or if the action's
    /// arguments fail validation.
    pub fn new(request_id: impl Into<String>, action: ActionArgs) -> Result<Self, SdkError> {
        let request_id = request_id.into();
        validate_request_id(&request_id)?;
        // Store the normalized copy, not the caller's, so a padded-but-valid
        // value cannot travel in the frozen body.
        let action = action.validated()?;
        Ok(Self {
            r#type: BROKER_REQUEST_TYPE.to_string(),
            protocol_version: BROKER_PROTOCOL_VERSION,
            request_id,
            action_version: action.action().current_version(),
            action,
        })
    }

    /// The action this request invokes.
    #[must_use]
    pub fn action(&self) -> Action {
        self.action.action()
    }

    /// Validate and normalize into the only form execution-side code accepts.
    ///
    /// This is the **one normalization door**, and it consumes the request.
    /// There is deliberately no non-consuming `validate(&self)`: a verdict
    /// about a value it does not replace can drift from the value the caller
    /// keeps holding (an earlier one validated a normalized copy, discarded
    /// it, and let the caller execute the un-normalized original). The only
    /// way to learn a request is valid is to receive the normalized
    /// [`ValidatedRequest`].
    ///
    /// # Errors
    ///
    /// Returns [`SdkError`] for a wrong `type`, an unsupported
    /// `protocolVersion` or `actionVersion`, a malformed `requestId`, or
    /// arguments that fail their own validation.
    pub fn validated(mut self) -> Result<ValidatedRequest, SdkError> {
        self.validate_envelope()?;
        self.action = self.action.validated()?;
        Ok(ValidatedRequest(self))
    }

    /// Validate and normalize, then serialize once into the bytes every attempt
    /// will send — [`Self::validated`] followed by [`ValidatedRequest::prepare`].
    ///
    /// # Errors
    ///
    /// Returns [`SdkError`] when [`Self::validated`] fails, or
    /// [`SdkError::InvalidInput`] if serialization fails.
    pub fn prepare(self) -> Result<PreparedRequest, SdkError> {
        self.validated()?.prepare()
    }

    /// Validate everything except the action arguments, which
    /// [`Self::validated`] normalizes in the same step.
    fn validate_envelope(&self) -> Result<(), SdkError> {
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
        let action = self.action();
        if self.action_version != action.current_version() {
            return Err(SdkError::InvalidInput(format!(
                "unsupported actionVersion {} for {} (expected {})",
                self.action_version,
                action.as_str(),
                action.current_version()
            )));
        }
        Ok(())
    }
}

/// Validate a `requestId`: non-empty, bounded, printable ASCII without spaces.
///
/// The bound and character set exist because this value becomes part of a
/// durable idempotency key and appears in audit records.
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

/// A [`BrokerRequest`] that has been validated **and normalized**.
///
/// The type execution-side code accepts. The only way to obtain one is
/// [`BrokerRequest::validated`], which normalizes on the way through, so
/// holding one proves the value carries what the validator approved — not the
/// caller's spelling of it.
///
/// The inner request is private with no borrowing accessor: a borrow would let
/// execution-side code clone it, mutate a public field, and execute the result.
/// [`Self::into_request`] consumes the wrapper for a host that needs to move
/// the envelope onward; what it yields is no longer evidence of anything.
///
/// A host that receives bytes builds one the same way a client does — parse,
/// call `validated()`, execute what comes back — since only its own verdict is
/// authoritative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedRequest(BrokerRequest);

impl ValidatedRequest {
    /// The action to execute, with its normalized arguments.
    #[must_use]
    pub fn args(&self) -> &ActionArgs {
        &self.0.action
    }

    /// The action being invoked.
    #[must_use]
    pub fn action(&self) -> Action {
        self.0.action()
    }

    /// The idempotency key the host keys replay on.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.0.request_id
    }

    /// Freeze the normalized request into the bytes every attempt will send.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] if serialization fails.
    pub fn prepare(self) -> Result<PreparedRequest, SdkError> {
        let body = serde_json::to_vec(&self.0).map_err(|e| {
            SdkError::InvalidInput(format!("broker request is not serializable: {e}"))
        })?;
        Ok(PreparedRequest {
            request: self.0,
            body,
        })
    }

    /// Consume this wrapper, yielding the normalized envelope — a plain
    /// [`BrokerRequest`] with public fields, no longer evidence that anything
    /// was validated, which is why this consumes rather than borrows.
    #[must_use]
    pub fn into_request(self) -> BrokerRequest {
        self.0
    }
}

/// A validated request together with the exact bytes to send.
///
/// This is what [`BrokerClient::send`] takes, so the retry contract is
/// structural: every attempt sends `body` verbatim, and no implementation gets
/// the chance to reserialize. The typed request is deliberately not exposed —
/// only the correlation metadata ([`Self::request_id`], [`Self::action`]) an
/// implementation legitimately needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRequest {
    request: BrokerRequest,
    body: Vec<u8>,
}

impl PreparedRequest {
    /// The frozen JSON body. Every attempt sends exactly these bytes.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// The idempotency key the host keys replay on.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request.request_id
    }

    /// The action being invoked.
    #[must_use]
    pub fn action(&self) -> Action {
        self.request.action()
    }
}

/// Machine-readable broker error code.
///
/// These name failures the *broker* is responsible for; failures inside an
/// action arrive as [`BrokerErrorCode::ActionFailed`] with detail in the
/// message.
///
/// # Which status a code may carry
///
/// A code and a [`BrokerResult`] status are two statements about the same
/// thing — whether side effects landed — so they cannot be paired freely.
/// `Failed` promises no side effects took hold; `Indeterminate` promises
/// nothing. This is the whole table, and it lives only here:
///
/// | Code | with `failed` | with `indeterminate` |
/// |---|---|---|
/// | `outcome_unknown` | no | yes |
/// | `internal` | yes | yes |
/// | every other code | yes | no |
///
/// [`Self::Internal`] is the one code legitimately either: a host fault before
/// dispatch is a known no-op, the same fault mid-execution is not.
/// [`Self::may_be_failed`] and [`Self::may_be_indeterminate`] are this table in
/// code, consulted by [`BrokerResponse::validate`], which rejects a mismatched
/// pairing as malformed rather than trusting either half.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerErrorCode {
    /// The envelope or action arguments failed validation.
    InvalidRequest,
    /// The `protocolVersion` is not supported by this host.
    UnsupportedProtocolVersion,
    /// The action name is unknown to this host.
    UnknownAction,
    /// The `actionVersion` is not supported for this action.
    UnsupportedActionVersion,
    /// The host knows this action but does not offer it.
    ///
    /// For an action where [`Action::is_best_effort`] holds, this is a normal
    /// answer and the agent carries on. Otherwise the agent cannot do its job
    /// on this host.
    Unsupported,
    /// The session credential was missing, malformed, or rejected.
    ///
    /// A host verdict, delivered as [`BrokerResult::Failed`], never as a
    /// transport error: the request was refused before execution, so the caller
    /// knows no side effects occurred.
    Unauthenticated,
    /// The requester is authenticated but not permitted this action.
    Unauthorized,
    /// Reuse of a `requestId` with different request content.
    RequestIdConflict,
    /// The action ran and reported a domain failure.
    ActionFailed,
    /// The host could not determine whether side effects occurred.
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
            Self::UnknownAction => "unknown_action",
            Self::UnsupportedActionVersion => "unsupported_action_version",
            Self::Unsupported => "unsupported",
            Self::Unauthenticated => "unauthenticated",
            Self::Unauthorized => "unauthorized",
            Self::RequestIdConflict => "request_id_conflict",
            Self::ActionFailed => "action_failed",
            Self::OutcomeUnknown => "outcome_unknown",
            Self::Internal => "internal",
        }
    }

    /// Whether this code may appear with [`BrokerResult::Failed`].
    ///
    /// One half of the table documented on [`BrokerErrorCode`], written as an
    /// exhaustive match so adding a code forces a decision here.
    #[must_use]
    pub fn may_be_failed(self) -> bool {
        match self {
            Self::InvalidRequest
            | Self::UnsupportedProtocolVersion
            | Self::UnknownAction
            | Self::UnsupportedActionVersion
            | Self::Unsupported
            | Self::Unauthenticated
            | Self::Unauthorized
            | Self::RequestIdConflict
            | Self::ActionFailed
            | Self::Internal => true,
            Self::OutcomeUnknown => false,
        }
    }

    /// Whether this code may appear with [`BrokerResult::Indeterminate`] —
    /// the other half of the table documented on [`BrokerErrorCode`].
    #[must_use]
    pub fn may_be_indeterminate(self) -> bool {
        match self {
            Self::OutcomeUnknown | Self::Internal => true,
            Self::InvalidRequest
            | Self::UnsupportedProtocolVersion
            | Self::UnknownAction
            | Self::UnsupportedActionVersion
            | Self::Unsupported
            | Self::Unauthenticated
            | Self::Unauthorized
            | Self::RequestIdConflict
            | Self::ActionFailed => false,
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

    /// An [`BrokerErrorCode::Unsupported`] error.
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(BrokerErrorCode::Unsupported, message)
    }

    /// An [`BrokerErrorCode::Unauthorized`] error.
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(BrokerErrorCode::Unauthorized, message)
    }
}

/// The terminal disposition of a broker request.
///
/// A discriminated union, so "succeeded with an error" and "failed with an
/// outcome" are unrepresentable. [`Self::Indeterminate`] is distinct from
/// [`Self::Failed`] on purpose: `Failed` promises no side effects took hold,
/// `Indeterminate` promises nothing and demands reconciliation. Which
/// [`BrokerErrorCode`] may carry which status is a closed table on that type.
///
/// # Why this type is not [`Deserialize`]
///
/// Its members reach the wire only flattened into [`BrokerResponse`], whose
/// strict reader enforces the exact key set per status. A derived reader here
/// was a second, laxer door onto the same bytes — it accepted and dropped
/// members the envelope rejects — and two copies of a strictness check drift.
/// [`Serialize`] is retained (it produces the envelope's flattened wire form),
/// so this is a read-side restriction only. Nothing is lost: a bare
/// `{"status": …}` object is not a payload this contract defines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BrokerResult {
    /// The action completed and produced this outcome.
    Succeeded {
        /// Action-specific success payload.
        #[serde(flatten)]
        outcome: ActionOutcome,
    },
    /// The action did not complete; no side effects are expected to persist.
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
    pub fn succeeded(outcome: ActionOutcome) -> Self {
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

    /// The outcome, when this is a success.
    #[must_use]
    pub fn outcome(&self) -> Option<&ActionOutcome> {
        match self {
            Self::Succeeded { outcome } => Some(outcome),
            Self::Failed { .. } | Self::Indeterminate { .. } => None,
        }
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
/// # Why deserialization goes through an intermediary
///
/// `#[serde(flatten)]` on `result` silently disables `deny_unknown_fields`, so
/// the derived reader accepted and discarded unknown members — exactly how a
/// secret-bearing host field crosses a boundary unnoticed. [`Deserialize`]
/// therefore routes through a private strict wire form (see `wire.rs`) with an
/// exact key set per status; anything else fails to parse and surfaces as
/// [`BrokerTransportError::MalformedResponse`]. Serialization is unchanged, and
/// a round-trip test pins that the strict reader accepts what the writer emits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    ///
    /// A plain `bool`, so it needs no explicit null guard: `null` already fails
    /// as a type error rather than defaulting to `false`.
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
    /// This checks only what a response asserts about itself. It cannot tell
    /// whether the response answers the request that was sent — for that, and
    /// for outcome-field validation, use [`Self::validate_for`]. A client should
    /// always prefer `validate_for`.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError::InvalidInput`] on a wrong `type`, an unsupported
    /// `protocolVersion`, a malformed `requestId`, an outcome with malformed
    /// identifiers, or an error code paired with the wrong status.
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
        validate_request_id(&self.request_id)?;
        match &self.result {
            BrokerResult::Succeeded { outcome } => outcome.validate()?,
            // A forbidden code/status pairing is a response contradicting
            // itself; neither half can be trusted, so it is rejected. The
            // table lives on `BrokerErrorCode`.
            BrokerResult::Failed { error } if !error.code.may_be_failed() => {
                return Err(SdkError::InvalidInput(format!(
                    "{} is not a valid code for a failed status",
                    error.code.as_str()
                )));
            }
            BrokerResult::Indeterminate { error } if !error.code.may_be_indeterminate() => {
                return Err(SdkError::InvalidInput(format!(
                    "{} is not a valid code for an indeterminate status",
                    error.code.as_str()
                )));
            }
            BrokerResult::Failed { .. } | BrokerResult::Indeterminate { .. } => {}
        }
        Ok(())
    }

    /// Validate this response *as the answer to `request`*.
    ///
    /// A response that validates in isolation can still be the wrong answer —
    /// a success for a different action, or for the wrong subject. A client
    /// never calls this directly: [`BrokerClientExt::execute`] runs it for
    /// every implementation and returns a [`ValidatedResponse`]. It stays
    /// public for a host validating its own output. Signature verification of
    /// read results is deliberately not included; see [`BrokerMessage::verify`].
    ///
    /// # Errors
    ///
    /// Returns everything [`Self::validate`] returns, plus
    /// [`SdkError::InvalidInput`] when the `requestId` does not correlate, a
    /// success outcome names a different action than the request, a success
    /// outcome echoes a different identity than the request supplied, or a
    /// read returned more messages than the request allowed.
    ///
    /// # What identity correlation compares
    ///
    /// Every identity the request supplies and the outcome echoes must name
    /// the same thing. Most outcomes echo nothing prior (host-minted ids, or a
    /// page with no `channelId` echo; `storage.address` deliberately omits the
    /// slug, whose `d` tag is a keyed hash of it). What remains: `agents.create`
    /// compares `channelId` as UUIDs; `agents.update`/`agents.delete` compare
    /// `agentPubkey` when targeted by pubkey. A name target is resolved
    /// host-side and unverifiable by construction — a rename may be the very
    /// thing the call performed.
    ///
    /// **Comparison is on parsed identities, never on bytes**: both identity
    /// types admit more than one legal spelling, and a byte comparison would
    /// reject a correct answer spelled differently — a worse failure than the
    /// one this check exists to catch.
    pub fn validate_for(&self, request: &PreparedRequest) -> Result<(), SdkError> {
        self.validate()?;
        if self.request_id != request.request_id() {
            return Err(SdkError::InvalidInput(format!(
                "response requestId \"{}\" does not match request \"{}\"",
                self.request_id,
                request.request_id()
            )));
        }
        if let BrokerResult::Succeeded { outcome } = &self.result {
            let expected = request.action();
            if outcome.action() != expected {
                return Err(SdkError::InvalidInput(format!(
                    "response carries a {} outcome for a {} request",
                    outcome.action().as_str(),
                    expected.as_str()
                )));
            }
            correlate::correlate_identities(&request.request.action, outcome)?;
            // `ActionOutcome::validate` never sees the request, so it can only
            // enforce the protocol-wide cap; the request's own limit is
            // applied here, the one place both halves are in scope.
            if let (
                ActionArgs::ChannelRead(args),
                ActionOutcome::ChannelRead(MessagePage { messages, .. }),
            ) = (&request.request.action, outcome)
            {
                let allowed = args.effective_limit() as usize;
                if messages.len() > allowed {
                    return Err(SdkError::InvalidInput(format!(
                        "read returned {} messages for a limit of {allowed}",
                        messages.len()
                    )));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
