//! NIP-FP — work fingerprints and their verifier.
//!
//! A fingerprint is a signed, replaceable index of events its subject selects
//! as evidence of their work (`docs/nips/NIP-FP.md`). The [`Verifier`] fetches
//! every selected event from its origin, discovers outcomes the index leaves
//! out, and produces a [`Report`] with separate `selection`, `observed`, and
//! `interpretation` sections. It never produces a score.
//!
//! The verifier is sans-IO: it hands out [`Query`] values and takes back
//! [`Response`] values, so callers choose the transport and tests can serve
//! fixtures. No network calls are made here.
//!
//! ```text
//! Verifier::new(fingerprint) → loop { next_queries → fetch → ingest } → finish → Report
//! ```

mod assess;
mod evidence;
mod report;
mod verifier;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use nostr::{EventBuilder, EventId, Filter, Kind, PublicKey, Tag};

use crate::SdkError;

pub use report::*;
pub use verifier::Verifier;

/// Provisional kind of a work fingerprint (addressable), as registered in
/// [`buzz_core::kind::KIND_WORK_FINGERPRINT`].
pub const FINGERPRINT_KIND: u16 = buzz_core::kind::KIND_WORK_FINGERPRINT as u16;

/// Maximum selection tags a fingerprint may carry and a verifier processes.
pub const MAX_SELECTIONS: usize = 256;

/// `type` of a transport bundle.
pub const BUNDLE_TYPE: &str = "nip-fp/transport-bundle";

/// `label` of a transport bundle. A bundle is transport, not a ledger entry or
/// a permission grant.
pub const BUNDLE_LABEL: &str = "transport bundle (pre-registration)";

/// NIP-32 namespace for trust labels on work events.
pub const LABEL_NAMESPACE: &str = "buzz.wot";

/// A named, versioned viewer policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// Policy name.
    pub name: String,
    /// Policy version.
    pub version: u32,
    /// Signers the viewer already knows. Marking them is interpretation, not
    /// evidence, and never changes a bucket.
    pub viewer_known: BTreeSet<PublicKey>,
}

impl Default for Policy {
    /// `nip-fp/default` version 1: the NIP's rules with no viewer-known keys.
    fn default() -> Self {
        Self {
            name: "nip-fp/default".into(),
            version: 1,
            viewer_known: BTreeSet::new(),
        }
    }
}

impl Policy {
    /// `nip-fp/viewer-known` version 1: the default rules, marking `keys` as
    /// known to the viewer.
    pub fn viewer_known(keys: impl IntoIterator<Item = PublicKey>) -> Self {
        Self {
            name: "nip-fp/viewer-known".into(),
            version: 1,
            viewer_known: keys.into_iter().collect(),
        }
    }
}

/// One request the verifier needs answered.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    /// Coverage record index; pass it back to [`Verifier::ingest`].
    pub scope: usize,
    /// The origin to ask, exactly as the fingerprint names it.
    pub origin: String,
    /// What to ask for.
    pub request: Request,
}

/// What a [`Query`] asks an origin for.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// The NIP-11 relay information document, for its `self` pubkey.
    RelayInfo,
    /// Events matching one NIP-01 filter.
    Events(Filter),
}

/// An origin's answer to a [`Query`].
#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    /// The relay's NIP-11 `self` pubkey, if it advertises one.
    RelayInfo(Option<PublicKey>),
    /// Raw event JSON exactly as returned. The verifier checks every byte.
    Events(Vec<serde_json::Value>),
    /// The query was refused or failed. Everything it would have found is
    /// unknown.
    Failed(String),
}

/// One selected work event and the origin to fetch it from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selected {
    /// The work event id.
    pub id: EventId,
    /// The relay URL the viewer should query for it.
    pub origin: String,
}

/// Build an unsigned work fingerprint (`kind:30340`).
///
/// The caller signs it; a managed agent's client adds its NIP-OA `auth` tag.
///
/// # Errors
///
/// Returns [`SdkError::InvalidInput`] when `d` is empty, an origin is empty,
/// an id repeats, or there are more than [`MAX_SELECTIONS`] selections.
pub fn build_work_fingerprint(d: &str, selections: &[Selected]) -> Result<EventBuilder, SdkError> {
    if d.trim().is_empty() {
        return Err(SdkError::InvalidInput(
            "fingerprint d tag must not be empty".into(),
        ));
    }
    if selections.len() > MAX_SELECTIONS {
        return Err(SdkError::InvalidInput(format!(
            "a fingerprint selects at most {MAX_SELECTIONS} events, got {}",
            selections.len()
        )));
    }
    let mut seen = BTreeSet::new();
    let mut tags = vec![tag(["d", d])?, tag(["coverage", "selected"])?];
    for selected in selections {
        if selected.origin.trim().is_empty() {
            return Err(SdkError::InvalidInput(format!(
                "selection {} has no origin",
                selected.id
            )));
        }
        if !seen.insert(selected.id) {
            return Err(SdkError::InvalidInput(format!(
                "selection {} repeats",
                selected.id
            )));
        }
        tags.push(tag(["e", &selected.id.to_hex(), &selected.origin])?);
    }
    Ok(EventBuilder::new(Kind::Custom(FINGERPRINT_KIND), "").tags(tags))
}

/// Wrap a signed fingerprint as a transport bundle (NIP-FP §Transport bundle).
///
/// The bundle carries only the fingerprint; verifiers fetch every work event
/// from its origin.
pub fn transport_bundle(fingerprint: &nostr::Event) -> serde_json::Value {
    serde_json::json!({
        "type": BUNDLE_TYPE,
        "version": 1,
        "label": BUNDLE_LABEL,
        "fingerprint": fingerprint,
    })
}

/// Return the fingerprint JSON from a transport bundle, or `value` itself when
/// it is a bare event.
///
/// # Errors
///
/// Returns [`SdkError::InvalidInput`] for a bundle of another type or version,
/// or one without a fingerprint.
pub fn fingerprint_json(value: &serde_json::Value) -> Result<&serde_json::Value, SdkError> {
    let Some(kind) = value.get("type") else {
        return Ok(value);
    };
    if kind.as_str() != Some(BUNDLE_TYPE)
        || value.get("version").and_then(|v| v.as_u64()) != Some(1)
    {
        return Err(SdkError::InvalidInput(format!(
            "not a {BUNDLE_TYPE} version 1"
        )));
    }
    value
        .get("fingerprint")
        .ok_or_else(|| SdkError::InvalidInput("bundle has no fingerprint".into()))
}

fn tag<const N: usize>(parts: [&str; N]) -> Result<Tag, SdkError> {
    Tag::parse(parts).map_err(|e| SdkError::InvalidTag(e.to_string()))
}
