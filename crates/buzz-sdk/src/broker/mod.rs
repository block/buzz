//! Agent ↔ broker contract — the operations an agent asks a host to perform.
//!
//! This module is a **contract only**. It defines the request envelope, the
//! closed set of [`Action`]s, the result shape, the HTTP binding, and a client
//! trait. It contains no host, no transport, and no signing: the only
//! implementation here is a test double.
//!
//! # Mental model
//!
//! ```text
//! agent → BrokerRequest → (POST /v1/action, bearer credential) → host
//!   host: authenticate → authorize → validate → execute → BrokerResponse
//! ```
//!
//! The agent holds its public key and a session credential. It holds no secret
//! key, has no relay connection of its own, and can reach the relay only by
//! asking. Everything it wants to do — including reading — is an action.
//!
//! # Why an operation enum rather than a "sign this" primitive
//!
//! [#6467](https://github.com/block/buzz/issues/6467) leaves this open. This
//! contract answers it: a closed enum of named business operations.
//!
//! A `sign(bytes)` primitive makes the broker a signing oracle. It can tell
//! *who* is asking but not *what for*, so the only policies it can express are
//! "all" and "none". Named operations invert that: a host can serve
//! `channel.read` while refusing `agents.create`, and a later policy or
//! information-flow layer has a per-operation surface to attach to. The cost is
//! that a new operation needs a new variant — deliberately, since adding one is
//! then a reviewable change to the contract rather than a new use of an existing
//! blank cheque.
//!
//! The same reasoning is why signing, publishing, and credential access are not
//! actions. `message.post` names an intent a host can reason about;
//! `publish(event)` names a mechanism it cannot.
//!
//! # How this satisfies #6467
//!
//! | #6467 requirement | Where |
//! |---|---|
//! | Identity separable from signing; pubkey-only identity | [`PubkeyHex`] is the only identity type; no secret-key type exists in this module. |
//! | Secret-dependent operations funnelled through the interface | Event signing is subsumed by the write actions (`message.post`, `message.reply`, `reaction.add`, `profile.set`, `agents.*`) — the agent states intent, the host signs. Encrypted-storage address derivation is [`Action::StorageAddress`]. NIP-42 relay auth, NIP-44 encryption, and NIP-98 request auth are **not** actions: they are mechanisms internal to whoever holds the key. |
//! | Relay auth is a control-flow choice | Nothing here mentions relay authentication, so there is no local auth step to skip. |
//! | Every relay-touching op, reads included, routable | [`Action::ChannelRead`] covers channel, thread, and mention-feed reads. No action assumes the caller can reach a relay. |
//! | Non-essential housekeeping skippable | [`Action::is_best_effort`] marks such actions; a host answers [`BrokerErrorCode::Unsupported`] and the agent carries on. |
//! | No secret leaks to children via env | No args type carries an environment map, and `deny_unknown_fields` means one cannot be smuggled in. Process spawning is a host concern this contract cannot express. |
//! | Lives in the shared client layer | `buzz-sdk`, which the CLI and the harness already depend on. |
//!
//! # The no-secret rule
//!
//! No secret key material crosses this boundary in either direction. What
//! enforces it is the wire schema, not a comment: every args and outcome type is
//! `deny_unknown_fields`, and tests pin each type's exact key set, so a
//! secret-bearing field cannot be added without a test failing.
//! [`AgentsCreateOutcome`] is the case that matters — it returns public identity
//! only, never the key it just minted.
//!
//! Strictness has to hold at every layer, or the outermost one decides. Three
//! places needed explicit work, because each had a hole a derive left open:
//! [`BrokerResponse`] cannot combine `deny_unknown_fields` with the `flatten`
//! that produces its wire shape; [`BrokerMessage`] wraps `nostr`'s `Event`,
//! whose own deserializer discards unknown members; and [`ActionArgs`] /
//! [`ActionOutcome`] are adjacently tagged, so without `deny_unknown_fields` on
//! the enum itself a sibling of their two keys was ignored when either type was
//! deserialized directly — and both are public and wire-facing, so that is a
//! real door, not a hypothetical one. The first two now deserialize through
//! private strict intermediaries and the third denies unknown fields, so the
//! rule reaches *inside* the envelope, inside each event object, and around each
//! nested action object — a host cannot ship a `secretKey` beside `sig`, or
//! beside `args`, and have it silently trimmed.
//!
//! There is also exactly **one** wire door per payload, so no lax reader sits
//! beside a strict one. [`BrokerResult`] is the case that needed work: its
//! members reach the wire only flattened into [`BrokerResponse`], but it also
//! derived a reader of its own, and that reader accepted and dropped siblings the
//! envelope rejects. It is no longer [`Deserialize`] — see the reasoning on that
//! type. Removing the second door is what keeps the rule single-sourced, since two
//! copies of a strictness check are exactly how this hole arose.
//!
//! # Identities have exactly one spelling
//!
//! Both identity types in this contract admit several legal spellings. A UUID may
//! be uppercase, unhyphenated, `{braced}`, or `urn:uuid:`-prefixed; hex may be
//! either case. Two spellings of one identity are the same identity, so the
//! contract picks one and normalizes to it: **lowercase hyphenated** for a
//! `channelId`, **lowercase** for a pubkey, `eventId`, or `dTag`.
//!
//! Normalization happens at both doors — each `validated()` and the
//! [`Deserialize`] impl of every member that holds an identity — so a value is
//! canonical whether it was built in Rust or parsed from JSON, and the frozen
//! request body carries the canonical spelling rather than the caller's. A host
//! may send any legal spelling and will be read as having sent the canonical one.
//!
//! Without this, [`BrokerResponse::validate_for`] would have compared the
//! caller's spelling of a `channelId` against the host's canonical echo of the
//! same channel and rejected a correct answer. That check compares parsed
//! identities as well, so neither guard is load-bearing alone.
//!
//! # Duplicate members are rejected
//!
//! A key may appear at most once in any object. serde's derived readers reject a
//! repeated field, so most of this contract gets that for free — but the strict
//! response intermediary originally buffered `outcome` through a
//! `serde_json::Value`, and that collapses duplicates last-wins. `outcome` was
//! therefore the one place a reader could observe a value the envelope's own
//! strictness never vetted, so it now re-parses the original bytes. The practical
//! consequence for a host author is the same as the no-null rule: emit each
//! member once, and do not rely on a later occurrence overriding an earlier one.
//!
//! A fourth hole was subtler and is closed the same way — see
//! [Optional members](#optional-members-omission-is-the-only-spelling-of-absence).
//!
//! Two limits are worth stating. A `String` field can physically hold secret
//! text, so keeping secrets out of message content and error messages is host
//! policy this contract cannot enforce. And nothing stops a host from *holding*
//! keys — that is the point; it stops one from handing them over.
//!
//! # Optional members: omission is the only spelling of absence
//!
//! **`null` is never a legal value anywhere in this contract.** Every optional
//! member means "absent" by being **omitted from the object**. A member present
//! with the value `null` is a malformed payload and is rejected — in a request,
//! in a response, in `args`, in an outcome, at any depth.
//!
//! This matters most to a host or agent implemented in another language. Many
//! serializers emit `null` for an unset field by default — Go's
//! `encoding/json` for a nil pointer without `omitempty`, Python's `json.dumps`
//! of an attribute left at `None`, a hand-built map that assigns the key
//! unconditionally. Such a payload will be rejected in full, not quietly read as
//! absent. Configure the serializer to **omit** unset members.
//!
//! The reason is that this contract decides meaning from absence, so absence
//! cannot have two spellings. `#[serde(default)] Option<T>` maps an explicit
//! `null` to the same value as an omitted member, which made
//! `{"status":"failed","outcome":null}` parse as a plain failure and skip the
//! per-status contradiction check. Rejecting `null` is stronger than recording
//! presence beside the value, because it leaves no layer with the question of
//! what a present-but-empty member was supposed to mean. Applied uniformly, it
//! also removes the guess about whether `{"limit": null}` requests the default
//! page limit or no limit at all: it requests neither, it is a malformed request.
//!
//! The rule is uniform, so there is nothing to look up per field: if a member
//! carries no value, leave it out.
//!
//! # Ownership recursion
//!
//! [`Action::AgentsCreate`] has no owner field. The owner of a created agent is
//! whichever identity the host authenticated for the request, so an agent that
//! creates an agent owns it, and following the chain upward always terminates at
//! a human. Bounding the depth of that chain is a host concern: it depends on
//! resources and policy this contract cannot see. A request that could name its
//! own authority would let any caller mint agents under someone else.
//!
//! # Deferred operations
//!
//! `presence.set` and `typing.set` are not in v1. They are housekeeping a host
//! can decline anyway, and the closed enum makes adding them purely additive —
//! a new variant, a new wire name, no change to existing ones.
//!
//! Streaming reads are also deferred. Reads are request/response; waking on a
//! mention is `channel.read` with `mentionsOnly`, polled.
//!
//! # Non-goals
//!
//! - **Hosts.** Authentication, authorization, idempotency storage, execution,
//!   and depth caps all live in the host.
//! - **Transports.** [`BrokerClient`] exists so an in-process and an HTTP
//!   implementation are interchangeable; neither is here.
//! - **Relay changes.** A host does ordinary relay work as an ordinary client.
//!   The relay never learns a broker exists.
//! - **Grants and authorization fields.** There is no `authorization` field.
//!   One gets added, as a discriminated object, when a real grant format and
//!   verifier exist — not before, so no field looks security-bearing while
//!   enforcing nothing.
//! - **Secret-key custody.** How the host holds keys, and whether it refuses to
//!   start when a stale local key is present, are host decisions.

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
/// # What this envelope deliberately omits
///
/// There is no requester, owner, scope, or relay field. Those are derived by the
/// host from the authenticated session credential. **A body that could name its
/// own subject would let any caller act as anyone** — that one rule is why
/// `channel.read` cannot ask about another identity's mentions, why
/// `profile.set` has no subject, and why `agents.create` has no owner.
///
/// # Retry contract
///
/// Retrying means resending the identical serialized request with the same
/// `requestId`, which is why a client never sends this type directly: call
/// [`Self::prepare`] to freeze it into a [`PreparedRequest`] and hand *that* to
/// [`BrokerClientExt::execute`]. The host hashes the bytes it receives and compares
/// that digest against the digest recorded under the same idempotency key:
///
/// - same key, same digest → the recorded outcome is replayed, nothing re-runs
/// - same key, different digest → rejected as a request-ID conflict
///
/// A typed value cannot carry that guarantee. Two serializations of one value
/// can differ in bytes across serde versions or implementations, and the
/// difference would surface as a spurious [`BrokerErrorCode::RequestIdConflict`]
/// on retry. Serializing once removes the possibility rather than warning about
/// it. There is no client-computed digest field: idempotency is decided
/// host-side, and a caller-supplied digest would be a claim the host has to
/// recompute anyway.
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
        // Store the normalized copy, not the caller's. `validated` both checks
        // and normalizes (trimming names, lowercasing pubkeys), so keeping the
        // original would let a padded selector pass validation and then travel
        // in the frozen body — the host would look up something the validator
        // never approved.
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
    /// This is the **one normalization door**. It consumes the request, so the
    /// un-normalized value is gone rather than sitting beside its normalized
    /// copy waiting to be executed by mistake.
    ///
    /// # Why there is no `validate(&self)`
    ///
    /// There used to be, and it was a trap. It called the arguments' own
    /// `validated()`, which *computes* a normalized copy, and then threw that
    /// copy away and returned `Ok(())`. A hand-built request targeting
    /// `"  helper  "` therefore validated successfully and still carried the
    /// padding, so a host that trusted the verdict and executed the struct
    /// looked up a name the validator never approved. [`Self::prepare`] was
    /// safe, but a host cannot force its callers through the client's outgoing
    /// path.
    ///
    /// A check that returns a verdict about a value it does not change can
    /// always drift from the value the caller keeps holding. So the verdict and
    /// the normalized value are now the same thing: the only way to learn that a
    /// request is valid is to receive the normalized [`ValidatedRequest`], and
    /// the only way to execute one is to have it. "Validated but not
    /// normalized" is unrepresentable rather than merely discouraged.
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
    /// will send.
    ///
    /// A convenience for the client path; it is exactly
    /// [`Self::validated`] followed by [`ValidatedRequest::prepare`], so the
    /// bytes are frozen from the normalized value and there is no second
    /// normalization to keep in step.
    ///
    /// # Errors
    ///
    /// Returns [`SdkError`] when [`Self::validated`] fails, or
    /// [`SdkError::InvalidInput`] if serialization fails.
    pub fn prepare(self) -> Result<PreparedRequest, SdkError> {
        self.validated()?.prepare()
    }

    /// Validate everything except the action arguments.
    ///
    /// Split out so [`Self::validated`] can check the envelope and then take the
    /// normalized arguments from a single `validated` call on the arguments,
    /// rather than validating them twice.
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
/// This is the type execution-side code accepts. Holding one is proof that every
/// field passed its validator *and* that the value carries what the validator
/// approved — not the caller's spelling of it. The two facts are inseparable
/// because there is one way to obtain this type, [`BrokerRequest::validated`],
/// which normalizes on the way through.
///
/// # Why the inner request is not reachable
///
/// The field is private and there is no accessor returning `&BrokerRequest`,
/// only [`Self::action`] and the metadata below. A borrow of the inner value
/// would let execution-side code clone it, mutate a public field, and execute
/// the result — which is the "validated but not normalized" state this type
/// exists to make unrepresentable. [`Self::into_request`] exists for a host that
/// genuinely needs to move the whole envelope onward; it consumes the wrapper,
/// so what it yields is no longer evidence of anything.
///
/// A host that receives bytes builds one the same way a client does: parse a
/// [`BrokerRequest`], call [`BrokerRequest::validated`], execute what comes
/// back. Only the host's verdict is authoritative, so it revalidates regardless
/// of what the client did.
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

    /// Consume this wrapper, yielding the normalized envelope.
    ///
    /// For a host that needs to move the whole request onward. The result is a
    /// plain [`BrokerRequest`] with public fields, so it is no longer evidence
    /// that anything was validated — which is why this consumes rather than
    /// borrows.
    #[must_use]
    pub fn into_request(self) -> BrokerRequest {
        self.0
    }
}

/// A validated request together with the exact bytes to send.
///
/// This is what [`BrokerClient::send`] takes, so the retry contract is
/// structural rather than documented: the first attempt and every retry send
/// `body` verbatim, and no implementation gets the chance to reserialize.
///
/// Construct one with [`ValidatedRequest::prepare`], or with
/// [`BrokerRequest::prepare`] which is the two steps together. The typed
/// [`BrokerRequest`] is retained privately and **deliberately not exposed**: an
/// implementation that could reach the typed value could serialize it again,
/// which is exactly the possibility freezing the bytes removes. What an
/// implementation legitimately needs is correlation metadata, so that — and only
/// that — is public: [`Self::request_id`] and [`Self::action`].
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
/// These name failures the *broker* is responsible for. Failures inside an
/// action arrive as [`BrokerErrorCode::ActionFailed`] with detail in the
/// message.
///
/// # Which status a code may carry
///
/// A code and a [`BrokerResult`] status are two statements about the same thing
/// — whether side effects landed — so they cannot be paired freely. This is the
/// whole table, and it lives only here:
///
/// | Code | with `failed` | with `indeterminate` |
/// |---|---|---|
/// | `outcome_unknown` | no | yes |
/// | `internal` | yes | yes |
/// | every other code | yes | no |
///
/// `Failed` promises no side effects took hold; `Indeterminate` promises
/// nothing. Every code but two names a fate the host *knows* — a refused
/// credential, a rejected envelope, a `requestId` conflict, an action that ran
/// and failed — so those are `Failed`-only. [`Self::OutcomeUnknown`] is the code
/// for not knowing, so it is `Indeterminate`-only. [`Self::Internal`] is the one
/// code that is legitimately either: a host fault before dispatch is a known
/// no-op, and the same fault mid-execution genuinely is not.
///
/// [`Self::may_be_failed`] and [`Self::may_be_indeterminate`] are that table in
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
    ///
    /// The only code that is [`BrokerResult::Indeterminate`]-only; see the
    /// status table on [`BrokerErrorCode`].
    OutcomeUnknown,
    /// An unexpected host-side fault.
    ///
    /// The only code that may carry either status: a fault before dispatch is a
    /// known no-op, the same fault mid-execution is not.
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

    /// Whether this code may appear with [`BrokerResult::Failed`], which
    /// promises no side effects took hold.
    ///
    /// One half of the table documented on [`BrokerErrorCode`]. Written as an
    /// exhaustive match so adding a code forces a decision here rather than
    /// silently inheriting a default.
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

    /// Whether this code may appear with [`BrokerResult::Indeterminate`], which
    /// promises nothing about side effects.
    ///
    /// The other half of the table documented on [`BrokerErrorCode`].
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
/// outcome" are unrepresentable rather than merely discouraged.
///
/// [`Self::Indeterminate`] is distinct from [`Self::Failed`] on purpose:
/// `Failed` promises no side effects took hold, while `Indeterminate` promises
/// nothing at all and demands reconciliation. Which [`BrokerErrorCode`] may
/// carry which status is a closed table, documented on that type.
///
/// # Why this type is not [`Deserialize`]
///
/// It is deliberately **not readable from the wire**, and that is the point:
/// [`BrokerResponse`] is the only door, and it is strict.
///
/// This type has no wire form of its own. Its members only ever appear flattened
/// into the response envelope, whose strict reader requires the exact key set the
/// declared status admits — so `status: failed` beside an `error` and a
/// `secretKey`, or a succeeded result beside an `error`, fail to parse. A derived
/// reader on this type answered the same bytes with `Ok`, dropping the members it
/// could not represent, so a consumer that parsed the result directly got a value
/// whose complete wire shape had never been checked.
///
/// The two ways to close that were to give this type its own strict reader, or to
/// take it off the wire. A second strict reader would be a second copy of the
/// per-status contradiction rules, and two copies of a security check drift — the
/// hole above exists precisely because one layer's strictness did not reach
/// another's. Removing the door leaves one implementation of the rule and nothing
/// to keep in sync. Nothing is lost: a bare `{"status": …}` object is not a
/// payload this contract defines, so no host or client had a legitimate reason to
/// parse one.
///
/// [`Serialize`] is retained — it is what produces the envelope's flattened wire
/// form — so this is a read-side restriction only, and the wire form is unchanged.
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
/// `#[serde(flatten)]` on `result` silently disables `deny_unknown_fields` —
/// serde cannot combine the two — so this envelope used to accept and discard an
/// unknown top-level key, an unknown key beside `status`, and even an `error`
/// riding alongside a succeeded outcome. Every other payload in this contract is
/// strict, and a discarded field is exactly how a secret-bearing host field
/// crosses a boundary unnoticed.
///
/// So [`Deserialize`] routes through a private strict wire form with an exact key
/// set per status, and anything else fails to parse. A transport reports that as
/// [`BrokerTransportError::MalformedResponse`] — the bytes claimed to be an
/// envelope and were not one — so the caller gets no verdict instead of a
/// quietly trimmed one. Serialization is unchanged, so the wire form is still
/// the flattened one; a round-trip test pins that the strict reader accepts what
/// the writer emits.
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
            // A code and a status are two statements about whether side effects
            // landed, so a pairing the table forbids is a response
            // contradicting itself. Neither half can be trusted over the other,
            // so it is rejected rather than reinterpreted. The table itself
            // lives on `BrokerErrorCode`.
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
    /// A response that validates in isolation can still be the wrong answer: a
    /// host (or a confused proxy) could return a `message.post` success to a
    /// `channel.read`, and a caller matching on the outcome enum would quietly
    /// take the wrong branch. This is the check that makes such a response
    /// unusable instead of merely surprising.
    ///
    /// A client does not call this directly and cannot forget to:
    /// [`BrokerClientExt::execute`] runs it for every implementation and returns
    /// a [`ValidatedResponse`], which is the only response type a caller can
    /// obtain. This method stays public for a host validating its own output.
    ///
    /// Signature verification of read results is deliberately not included; see
    /// [`BrokerMessage::verify`].
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
    /// `requestId` plus action is not enough: a host routing bug can return a
    /// well-formed success for the wrong *subject*. Every identity the request
    /// supplies and the outcome echoes must name the same thing. This is the
    /// whole table:
    ///
    /// | Action | compared | not compared, and why |
    /// |---|---|---|
    /// | `channel.read` | — | outcome carries a page and cursor, no echo of `channelId` |
    /// | `message.post` | — | outcome is `eventId`/`kind`/`createdAt`, all host-minted |
    /// | `message.reply` | — | same; the parent id is not echoed |
    /// | `reaction.add` | — | same |
    /// | `profile.set` | — | same |
    /// | `storage.address` | — | `slug` is deliberately absent from the outcome: a `d` tag is a keyed hash of it, and echoing the slug would defeat that |
    /// | `agents.create` | `channelId`, as UUIDs | pubkey and name are newly minted, so there is nothing prior to compare |
    /// | `agents.update` | `agentPubkey` when targeted by pubkey | a name target is resolved host-side; `displayName` may be exactly what this call changed |
    /// | `agents.delete` | `agentPubkey` when targeted by pubkey | ditto for a name target |
    ///
    /// **Comparison is on parsed identities, never on bytes.** Both identity
    /// types here admit more than one legal spelling — a UUID may be uppercase,
    /// unhyphenated, braced or `urn:uuid:`-prefixed; hex may be either case — and
    /// two spellings of one identity are the same identity. A byte comparison
    /// would reject a correct answer whenever the caller and the host spelled it
    /// differently, which is a worse failure than the one this check exists to
    /// catch. Values are also canonicalized where they enter — at each
    /// `validated()` and at the wire — so both sides normally arrive canonical;
    /// the parsed comparison does not depend on that having happened.
    ///
    /// A name-targeted `agents.update`/`agents.delete` is the one case where the
    /// caller asked for an identity it cannot verify in the reply. That is
    /// inherent: the host resolves the name, and a rename may be the very thing
    /// the call performed.
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
            // `ActionOutcome::validate` can only enforce the protocol-wide cap,
            // because it never sees the request. A page bounded only by that cap
            // still overruns a caller that asked for one message and was handed
            // five hundred, so the request's own number is applied here — the
            // one place both halves are in scope.
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
