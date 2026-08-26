//! Broker ingress: a verified frame in, a signed result out.
//!
//! This module owns the two transport edges the pipeline deliberately does not:
//! turning a received observer frame into a [`VerifiedRequest`], and turning the
//! resulting [`BrokerResponse`] into a frame addressed back to the requester.
//!
//! ```text
//! signed frame → verify → decrypt → pipeline::execute → encrypt → signed result
//! ```
//!
//! # Why the request is the whole plaintext
//!
//! A broker request is the *entire* decrypted frame payload — a bare
//! [`BrokerRequest`] JSON object. It is not wrapped in the ACP observer
//! envelope, and that is a load-bearing choice: the envelope carries per-attempt
//! fields (`seq`, `timestamp`), and the idempotency digest is a hash of the
//! bytes as received. Hashing an envelope would make a legitimate retry — same
//! `requestId`, same operation, a second later — hash differently and be refused
//! as a request-ID conflict. Keeping the envelope out of the plaintext means the
//! only way the digest changes is if the request itself changed.
//!
//! Frame-level identity (`created_at`, event id, signature) varies freely
//! between attempts and is *outside* the hashed bytes by construction.
//!
//! # Why a rejected frame is silent
//!
//! [`verify_frame`] failures produce no result frame. A frame that fails
//! verification has no trustworthy requester to address a reply to, and
//! answering one anyway would let anybody make this host emit signed frames at a
//! target of their choosing. Only a frame that verified — and that actually
//! claims to be a broker request — can be answered.

use buzz_core_pkg::observer::{
    decrypt_observer_plaintext, encrypt_observer_payload, OBSERVER_AGENT_TAG,
    OBSERVER_FRAME_CONTROL, OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY,
};
use buzz_sdk_pkg::broker::{BrokerResponse, BROKER_REQUEST_TYPE};
use buzz_sdk_pkg::kind::KIND_AGENT_OBSERVER_FRAME;
use nostr::{Event, Keys, Kind, PublicKey};
use zeroize::Zeroize;

use super::pipeline::{execute, BoxFuture, BrokerContext, VerifiedRequest};
use super::MAX_REQUEST_BYTES;

/// Largest accepted clock skew between a frame and this host, in seconds.
///
/// Matches the relay's observer-frame freshness window. Enforced again here
/// because the relay's check protects the relay: a host that trusted it would be
/// trusting a hop it does not control to bound replay of an old signed frame.
pub const MAX_FRAME_SKEW_SECS: i64 = 300;

/// Why an inbound frame is not a broker request this host will act on.
///
/// [`Self::NotBrokerTraffic`] is the ordinary case, not an error: every observer
/// telemetry frame an owner receives reaches this check, and almost none are
/// broker requests. It is separated from [`Self::Rejected`] so a caller can log
/// a real refusal without logging every agent heartbeat.
#[derive(Debug, PartialEq, Eq)]
pub enum FrameRejection {
    /// The frame is well-formed observer traffic that is not a broker request.
    NotBrokerTraffic,
    /// The frame claims to be broker traffic but failed verification.
    Rejected(String),
}

impl FrameRejection {
    fn rejected(message: impl Into<String>) -> Self {
        Self::Rejected(message.into())
    }
}

/// Verify an inbound observer frame and decrypt it into a [`VerifiedRequest`].
///
/// Every field of the returned request is derived from the frame this host
/// verified, never from the request body:
///
/// - `owner_pubkey` is *this host's own* public key. It is not read from the
///   frame at all, so no signer can name a different owner.
/// - `requester_pubkey` is the verified signer.
/// - `relay_scope` is the relay the caller received the frame on.
///
/// # Errors
///
/// Returns [`FrameRejection::NotBrokerTraffic`] for a frame that is not a broker
/// request, and [`FrameRejection::Rejected`] when a frame that claims to be one
/// fails verification: bad id or signature, wrong kind, missing or duplicated
/// routing tags, a direction other than requester-to-owner telemetry, a
/// timestamp outside [`MAX_FRAME_SKEW_SECS`], an undecryptable body, or a
/// payload larger than [`MAX_REQUEST_BYTES`].
pub fn verify_frame(
    owner_keys: &Keys,
    event: &Event,
    relay_scope: &str,
    now: i64,
) -> Result<VerifiedRequest, FrameRejection> {
    if event.kind != Kind::Custom(KIND_AGENT_OBSERVER_FRAME as u16) {
        return Err(FrameRejection::NotBrokerTraffic);
    }
    // Cheap structural gates run before the expensive ones, but nothing that
    // could admit a frame is skipped: id and signature are checked before the
    // content is decrypted, and the content is bounded before it is parsed.
    if single_tag(event, OBSERVER_FRAME_TAG).as_deref() != Some(OBSERVER_FRAME_TELEMETRY) {
        return Err(FrameRejection::NotBrokerTraffic);
    }

    if !event.verify_id() {
        return Err(FrameRejection::rejected("observer frame has an invalid id"));
    }
    if !event.verify_signature() {
        return Err(FrameRejection::rejected(
            "observer frame has an invalid signature",
        ));
    }

    let skew = event.created_at.as_secs() as i64 - now;
    if skew.abs() > MAX_FRAME_SKEW_SECS {
        return Err(FrameRejection::rejected(format!(
            "observer frame timestamp is {skew}s from now, outside the ±{MAX_FRAME_SKEW_SECS}s window"
        )));
    }

    let owner = owner_keys.public_key();
    let recipient = single_pubkey_tag(event, "p")
        .ok_or_else(|| FrameRejection::rejected("observer frame has no single p tag"))?;
    if recipient != owner {
        // Addressed to a different owner. This host holds no credentials that
        // could act on it, and cannot decrypt it either.
        return Err(FrameRejection::NotBrokerTraffic);
    }

    let agent = single_pubkey_tag(event, OBSERVER_AGENT_TAG)
        .ok_or_else(|| FrameRejection::rejected("observer frame has no single agent tag"))?;
    // Requester-to-owner telemetry only: the signer must be the agent the frame
    // names. A frame signed by anyone else claiming to be an agent's telemetry
    // would let a third party borrow that agent's authorization.
    if event.pubkey != agent {
        return Err(FrameRejection::rejected(
            "observer frame signer is not the agent it names",
        ));
    }
    if agent == owner {
        // The owner key signing owner-addressed telemetry is not a delegated
        // request; policy refuses it too, but it never becomes a request here.
        return Err(FrameRejection::NotBrokerTraffic);
    }

    let mut plaintext = decrypt_observer_plaintext(owner_keys, event)
        .map_err(|error| FrameRejection::rejected(format!("frame decrypt failed: {error}")))?;
    let outcome = classify_plaintext(&plaintext);
    let verified = match outcome {
        Ok(()) => Ok(VerifiedRequest {
            owner_pubkey: owner.to_hex(),
            requester_pubkey: event.pubkey.to_hex(),
            relay_scope: relay_scope.to_string(),
            payload: plaintext.as_bytes().to_vec(),
        }),
        Err(rejection) => Err(rejection),
    };
    plaintext.zeroize();
    verified
}

/// Decide whether a decrypted payload is a broker request worth dispatching.
///
/// Deliberately does not validate the request: an ill-formed *broker* request
/// must reach the pipeline so the requester gets a `invalid_request` answer,
/// while a payload that never claimed to be one must not be answered at all.
fn classify_plaintext(plaintext: &str) -> Result<(), FrameRejection> {
    if plaintext.len() > MAX_REQUEST_BYTES {
        return Err(FrameRejection::rejected(format!(
            "broker request payload exceeds {MAX_REQUEST_BYTES} bytes (got {})",
            plaintext.len()
        )));
    }
    let peeked: serde_json::Value =
        serde_json::from_str(plaintext).map_err(|_| FrameRejection::NotBrokerTraffic)?;
    if peeked.get("type").and_then(serde_json::Value::as_str) == Some(BROKER_REQUEST_TYPE) {
        Ok(())
    } else {
        Err(FrameRejection::NotBrokerTraffic)
    }
}

fn single_tag(event: &Event, name: &str) -> Option<String> {
    let mut values = event
        .tags
        .iter()
        .filter(|tag| tag.kind().to_string() == name)
        .filter_map(|tag| tag.content());
    let value = values.next()?;
    // A duplicated routing tag is ambiguous, and picking one would let a frame
    // present a different route to this host than it did to the relay.
    if values.next().is_some() {
        return None;
    }
    Some(value.to_string())
}

fn single_pubkey_tag(event: &Event, name: &str) -> Option<PublicKey> {
    PublicKey::from_hex(&single_tag(event, name)?).ok()
}

/// Build the signed frame that carries `response` back to `requester`.
///
/// The result travels as an owner-to-agent *control* frame: encrypted to the
/// requester, signed by the owner, addressed with the requester as both
/// recipient and agent. That is the only direction the relay routes owner-signed
/// observer traffic in, and it means a result is readable by exactly the agent
/// that asked for it.
///
/// # Errors
///
/// Returns an error if `requester` is not a valid hex pubkey, or if the response
/// cannot be encrypted, built, or signed.
pub fn build_result_frame(
    owner_keys: &Keys,
    requester: &str,
    response: &BrokerResponse,
) -> Result<Event, String> {
    let requester = PublicKey::from_hex(requester.trim())
        .map_err(|error| format!("invalid requester pubkey: {error}"))?;
    let requester_hex = requester.to_hex();
    let encrypted = encrypt_observer_payload(owner_keys, &requester, response)
        .map_err(|error| format!("failed to encrypt broker result: {error}"))?;
    buzz_sdk_pkg::build_agent_observer_frame(
        &requester_hex,
        &requester_hex,
        OBSERVER_FRAME_CONTROL,
        &encrypted,
    )
    .map_err(|error| format!("failed to build broker result frame: {error}"))?
    .sign_with_keys(owner_keys)
    .map_err(|error| format!("failed to sign broker result frame: {error}"))
}

/// Delivers a signed result frame to the relay.
///
/// A trait because kind-24200 frames are only accepted over the relay's
/// WebSocket path — the HTTP bridge rejects the kind outright — so delivery is a
/// different transport from every other Desktop publish. Isolating it here keeps
/// that fact in one place and lets the ingress path be tested without a relay.
pub trait ResultPublisher: Send + Sync {
    /// Publish `event`, or report why delivery failed.
    fn publish(&self, event: Event) -> BoxFuture<'_, Result<(), String>>;
}

/// The outcome of handling one inbound frame.
#[derive(Debug)]
pub enum Handled {
    /// The frame was not a broker request for this host.
    Ignored(FrameRejection),
    /// The request produced `response`, and delivery reported `delivery`.
    ///
    /// `delivery` is separate from the response on purpose: a request can
    /// execute successfully and still fail to deliver its result. That must not
    /// be reported as an execution failure, and it must not retry the execution
    /// — the recorded outcome is already durable and a retry with the same
    /// `requestId` replays it.
    Executed {
        /// The terminal response the pipeline produced.
        response: Box<BrokerResponse>,
        /// Whether the result frame reached the relay.
        delivery: Result<(), String>,
    },
}

/// Verify, execute, and answer one inbound frame.
///
/// # Errors
///
/// Returns `Err` only for a host fault that leaves no deliverable response —
/// an unreachable execution log, for instance. Refusals the requester should
/// hear about come back inside [`Handled::Executed`].
pub async fn handle_frame(
    owner_keys: &Keys,
    event: &Event,
    relay_scope: &str,
    now: i64,
    ctx: BrokerContext<'_>,
    publisher: &dyn ResultPublisher,
) -> Result<Handled, String> {
    let request = match verify_frame(owner_keys, event, relay_scope, now) {
        Ok(request) => request,
        Err(rejection) => return Ok(Handled::Ignored(rejection)),
    };

    let requester = request.requester_pubkey.clone();
    let response = execute(&request, ctx).await?;

    // Audit line: identities, capability scope, and disposition. No prompts, no
    // arguments, no decrypted payload — an audit trail that leaks the thing it
    // audits is worse than none.
    eprintln!(
        "buzz-desktop: broker: request={} requester={} scope={} disposition={} replayed={}",
        response.request_id,
        requester,
        relay_scope,
        disposition(&response),
        response.replayed
    );

    let delivery = match build_result_frame(owner_keys, &requester, &response) {
        Ok(frame) => publisher.publish(frame).await,
        Err(error) => Err(error),
    };
    Ok(Handled::Executed {
        response: Box::new(response),
        delivery,
    })
}

/// Audit word for a response's terminal disposition.
fn disposition(response: &BrokerResponse) -> &'static str {
    use buzz_sdk_pkg::broker::BrokerResult;
    match response.result {
        BrokerResult::Succeeded { .. } => "succeeded",
        BrokerResult::Failed { .. } => "failed",
        BrokerResult::Indeterminate { .. } => "indeterminate",
    }
}

#[cfg(test)]
mod tests;
