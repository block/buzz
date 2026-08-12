//! Origin-sealed Blossom transport proofs for canonical authorization.

use chrono::{DateTime, Utc};
use nostr::Event;

use crate::{AuthError, ProofTransport, VerifiedFederatedAssertion, VerifiedNostrProof};

/// Closed Blossom operation bound by a kind:24242 authorization event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlossomAuthorizationVerb {
    /// Upload one exact content digest.
    Upload,
    /// Read one exact content digest, optionally under a server-wide grant.
    Get,
}

impl BlossomAuthorizationVerb {
    const fn tag(self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::Get => "get",
        }
    }
}

/// Verify Blossom transport and bind it to one canonical federated assertion.
///
/// The event must name exactly one verb and expiration. Uploads require the
/// exact content digest; reads require either that digest or a matching server
/// grant. The returned proof is bound to the assertion's exact fingerprints.
#[allow(clippy::too_many_arguments)]
pub fn verify_blossom_authorization_proof(
    event: &Event,
    verb: BlossomAuthorizationVerb,
    expected_server: &str,
    content_sha256: &str,
    maximum_age_seconds: u64,
    assertion: &VerifiedFederatedAssertion,
    request_fingerprint: [u8; 32],
    target_fingerprint: [u8; 32],
    transport_context_fingerprint: [u8; 32],
) -> Result<VerifiedNostrProof, AuthError> {
    if maximum_age_seconds == 0
        || content_sha256.len() != 64
        || !content_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(AuthError::BlossomInvalid);
    }
    event.verify().map_err(|_| AuthError::BlossomInvalid)?;
    if event.kind.as_u16() != 24242 || event.content.trim().is_empty() {
        return Err(AuthError::BlossomInvalid);
    }

    let mut verb_tag = None;
    let mut expiration_tag = None;
    let mut digest_matches = false;
    let mut server_seen = false;
    let mut server_matches = false;
    let expected_server = normalized_server(expected_server);
    for tag in event.tags.iter() {
        match tag.kind().to_string().as_str() {
            "t" if verb_tag.replace(tag.content()).is_some() => {
                return Err(AuthError::BlossomInvalid);
            }
            "t" => {}
            "expiration" if expiration_tag.replace(tag.content()).is_some() => {
                return Err(AuthError::BlossomInvalid);
            }
            "expiration" => {}
            "x" => {
                digest_matches |= tag.content() == Some(content_sha256);
            }
            "server" => {
                server_seen = true;
                server_matches |= tag
                    .content()
                    .map(normalized_server)
                    .is_some_and(|server| server == expected_server);
            }
            _ => {}
        }
    }
    if verb_tag.flatten() != Some(verb.tag())
        || (server_seen && !server_matches)
        || match verb {
            BlossomAuthorizationVerb::Upload => !digest_matches,
            BlossomAuthorizationVerb::Get => !digest_matches && !server_matches,
        }
    {
        return Err(AuthError::BlossomInvalid);
    }

    let expiration_seconds = expiration_tag
        .flatten()
        .and_then(|value| value.parse::<u64>().ok())
        .and_then(|value| i64::try_from(value).ok())
        .ok_or(AuthError::BlossomInvalid)?;
    let event_expires_at =
        DateTime::<Utc>::from_timestamp(expiration_seconds, 0).ok_or(AuthError::BlossomInvalid)?;
    let now_seconds = Utc::now().timestamp();
    let created_seconds =
        i64::try_from(event.created_at.as_secs()).map_err(|_| AuthError::BlossomInvalid)?;
    let maximum_age_seconds =
        i64::try_from(maximum_age_seconds).map_err(|_| AuthError::BlossomInvalid)?;
    if created_seconds > now_seconds.saturating_add(5)
        || now_seconds.saturating_sub(created_seconds) > maximum_age_seconds
    {
        return Err(AuthError::BlossomInvalid);
    }

    let (_, assertion_expires_at) = assertion.time_bounds();
    let expires_at = event_expires_at.min(assertion_expires_at);
    let (assertion_transport, assertion_request, assertion_target, assertion_context) =
        assertion.request_binding();
    if assertion_transport != ProofTransport::Blossom
        || assertion_request != &request_fingerprint
        || assertion_target != &target_fingerprint
        || assertion_context != &transport_context_fingerprint
        || expires_at <= Utc::now()
    {
        return Err(AuthError::BlossomInvalid);
    }

    VerifiedNostrProof::from_verifier(
        assertion.authorization_domain(),
        event.pubkey,
        ProofTransport::Blossom,
        request_fingerprint,
        target_fingerprint,
        transport_context_fingerprint,
        Some(*assertion.assertion_fingerprint()),
        None,
        expires_at,
    )
    .ok_or(AuthError::BlossomInvalid)
}

fn normalized_server(value: &str) -> String {
    let authority = match value.split_once("://") {
        Some((_scheme, rest)) => match rest.split('/').next() {
            Some(authority) => authority,
            None => rest,
        },
        None => match value.split('/').next() {
            Some(authority) => authority,
            None => value,
        },
    };
    buzz_core::tenant::normalize_host(authority)
}
