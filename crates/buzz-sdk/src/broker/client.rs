//! Client trait and HTTP binding for the broker contract.
//!
//! # HTTP binding
//!
//! ```text
//! POST /v1/action
//! Authorization: Bearer <opaque session credential>
//! Content-Type: application/json
//!
//! <PreparedRequest::body(), verbatim>
//! ```
//!
//! The response body is a [`BrokerResponse`] as JSON. Every terminal
//! disposition the *host* reached — including a rejected credential — is a
//! well-formed envelope returned with `200`: the verdict lives in `status`,
//! and a second copy in the status line could only ever disagree with it. A
//! client must nonetheless **attempt to parse an envelope regardless of HTTP
//! status** (an intermediary may map dispositions onto statuses); if a valid
//! envelope is present, it is the answer. Only when no envelope can be parsed
//! does the status matter, and then only as operator detail — see
//! [`BrokerTransportError`].
//!
//! # The credential
//!
//! The credential is **opaque to this contract**: a bearer token the agent
//! received at startup and can only replay — not a key, not a signature. A
//! rejected credential is a host verdict, not a transport failure: it arrives
//! as `Failed` with [`super::BrokerErrorCode::Unauthenticated`], which carries
//! the promise that the action did not run.
//!
//! Binding a credential to a specific (agent, conversation) pair **is the
//! host's concern**; the request body carries no requester and no scope,
//! precisely so the host's binding is the only thing that decides authority.
//! Since the credential is the whole of the agent's authority, serving this
//! over anything but a loopback socket or TLS is publishing it.

use std::future::Future;
use std::pin::Pin;

use super::{BrokerResponse, BrokerResult, PreparedRequest};

/// Path of the single broker endpoint.
pub const BROKER_ACTION_PATH: &str = "/v1/action";

/// Header carrying the opaque session credential, as `Bearer <credential>`.
pub const BROKER_CREDENTIAL_HEADER: &str = "authorization";

/// No usable [`BrokerResponse`] was obtained, so the request's fate is unknown.
///
/// Every variant means the same thing to a caller: nothing can be concluded
/// about side effects, and the only safe next step is to retry the identical
/// bytes (which the host will deduplicate) or to reconcile by reading state.
/// The variants differ only in what to tell an operator. Host verdicts never
/// appear here — this type is strictly for the absence of an answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BrokerTransportError {
    /// The host could not be reached, or the connection failed mid-request.
    #[error("broker unreachable: {0}")]
    Unreachable(String),
    /// An HTTP response arrived carrying no parseable envelope.
    ///
    /// Typically an intermediary answering instead of the host: a proxy `401`,
    /// a `404` for a missing route, a `502`. The status is recorded for
    /// operators and carries no contractual meaning — an intermediary's `401`
    /// does not prove the host never ran the action.
    #[error("no broker envelope in HTTP {status} response: {detail}")]
    NoEnvelope {
        /// The HTTP status observed.
        status: u16,
        /// Operator-facing detail about what arrived instead.
        detail: String,
    },
    /// An envelope arrived but did not validate against the request that was
    /// sent — wrong `requestId`, wrong action, a malformed outcome, or a status
    /// contradicting its own error code.
    ///
    /// A host that answers something other than what was asked has given no
    /// verdict at all, which is why this is a transport failure rather than a
    /// `Failed` result.
    #[error("malformed broker response: {0}")]
    MalformedResponse(String),
}

/// A future returned by [`BrokerClient::send`].
///
/// Spelled as a boxed future rather than `async fn` in the trait because this
/// trait must be object-safe: the harness holds one client and must not know
/// whether it talks to an in-process host or an HTTP one.
pub type BrokerFuture<'a> =
    Pin<Box<dyn Future<Output = Result<BrokerResponse, BrokerTransportError>> + Send + 'a>>;

/// A future returned by [`BrokerClientExt::execute`].
pub type ValidatedFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ValidatedResponse, BrokerTransportError>> + Send + 'a>>;

/// A host response that has been checked against the request it answers.
///
/// The only way to obtain one is [`ValidatedResponse::validate`], which
/// [`BrokerClientExt::execute`] calls — so correlation is not advice an
/// implementation may skip. [`BrokerResponse::validate_for`] remains public for
/// a host validating its own output, but a client never has to remember to
/// call it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedResponse(BrokerResponse);

impl ValidatedResponse {
    /// Check `response` against the request it claims to answer.
    ///
    /// # Errors
    ///
    /// Returns [`BrokerTransportError::MalformedResponse`] when the response
    /// does not correlate, carries an outcome for a different action, asserts a
    /// malformed identifier, or pairs a status with a code that contradicts it.
    /// A response that fails here is not a host verdict — nothing can be
    /// concluded about side effects from it.
    pub fn validate(
        response: BrokerResponse,
        request: &PreparedRequest,
    ) -> Result<Self, BrokerTransportError> {
        response
            .validate_for(request)
            .map_err(|e| BrokerTransportError::MalformedResponse(e.to_string()))?;
        Ok(Self(response))
    }

    /// The terminal disposition the host reached.
    #[must_use]
    pub fn result(&self) -> &BrokerResult {
        &self.0.result
    }

    /// The correlated `requestId`.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.0.request_id
    }

    /// Whether the host replayed a previously recorded outcome.
    #[must_use]
    pub fn replayed(&self) -> bool {
        self.0.replayed
    }

    /// The underlying envelope, for logging or re-serialization.
    #[must_use]
    pub fn envelope(&self) -> &BrokerResponse {
        &self.0
    }

    /// Consume this wrapper, yielding the validated envelope.
    #[must_use]
    pub fn into_envelope(self) -> BrokerResponse {
        self.0
    }
}

/// Permission to call [`BrokerClient::send`], which only
/// [`BrokerClientExt::execute`] can mint.
///
/// This is what makes validation *structurally* the only door. `send` must be
/// public — an out-of-crate implementation has to define it — but a caller
/// must not be able to invoke it and receive an uncorrelated envelope. A token
/// with a private field satisfies both: any crate can accept one in a
/// signature, only this module can construct one. An implementation should
/// ignore its value; it carries no data.
///
/// An implementation that stashes and exposes the envelope it saw has
/// deliberately built an exfiltrating transport — a different thing from a
/// caller forgetting to correlate. Closing the accidental path is the goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dispatch(());

/// Something that can execute broker requests.
///
/// One method, because there is one endpoint: every operation is an
/// [`super::Action`] inside the request, so adding an action never changes
/// this trait. This is the **transport primitive** — frozen bytes out, one
/// envelope back — not the caller's interface; callers use
/// [`BrokerClientExt::execute`], where response correlation happens.
///
/// Implementations must be usable as `dyn BrokerClient`.
pub trait BrokerClient: Send + Sync {
    /// Send `request`'s frozen bytes and return whatever envelope came back.
    ///
    /// An implementation's whole job is transport: send
    /// [`PreparedRequest::body`] verbatim, parse an envelope regardless of
    /// HTTP status, and return it unjudged. The [`Dispatch`] argument is why
    /// this cannot be called directly by an outside caller; that is deliberate.
    ///
    /// # Errors
    ///
    /// Returns [`BrokerTransportError`] when no envelope could be obtained or
    /// parsed. A host that answered — even to refuse — returns `Ok`, with the
    /// verdict in [`BrokerResponse::result`].
    fn send<'a>(&'a self, request: &'a PreparedRequest, dispatch: Dispatch) -> BrokerFuture<'a>;
}

/// The caller-facing half of [`BrokerClient`]: send, then validate.
///
/// Blanket-implemented for every [`BrokerClient`], including `dyn BrokerClient`,
/// and **not overridable** — coherence forbids a second implementation, so there
/// is exactly one definition of what validating a response means and no client
/// can weaken it.
pub trait BrokerClientExt: BrokerClient {
    /// Send `request` and return a response already checked against it.
    ///
    /// # Errors
    ///
    /// Returns [`BrokerTransportError`] when no envelope arrived, or
    /// [`BrokerTransportError::MalformedResponse`] when the envelope that
    /// arrived does not answer `request`. Both mean the same thing to a caller:
    /// no verdict, so nothing is known about side effects.
    fn execute<'a>(&'a self, request: &'a PreparedRequest) -> ValidatedFuture<'a>;
}

impl<C: BrokerClient + ?Sized> BrokerClientExt for C {
    fn execute<'a>(&'a self, request: &'a PreparedRequest) -> ValidatedFuture<'a> {
        Box::pin(async move {
            let response = self.send(request, Dispatch(())).await?;
            ValidatedResponse::validate(response, request)
        })
    }
}
