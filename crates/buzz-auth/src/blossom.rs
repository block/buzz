//! Blossom kind:24242 authentication verification (BUD-11 compliant).

/// Blossom kind:24242 verbs Buzz currently accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlossomVerb {
    /// Authorize one blob upload.
    Upload,
    /// Authorize one blob download or a server-scoped download.
    Get,
}

impl BlossomVerb {
    fn as_str(self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::Get => "get",
        }
    }
}

/// Rejection from full Blossom operation verification.
#[derive(Debug, thiserror::Error)]
pub enum BlossomAuthError {
    /// The Schnorr signature is invalid.
    #[error("invalid signature")]
    InvalidSignature,
    /// The event is not kind 24242.
    #[error("invalid auth event kind")]
    InvalidAuthKind,
    /// The event does not contain a human-readable description.
    #[error("invalid auth event")]
    InvalidAuthEvent,
    /// The `t` tag does not name the required operation.
    #[error("invalid auth verb")]
    InvalidAuthVerb,
    /// A required tag is absent.
    #[error("missing required tag: {0}")]
    MissingTag(&'static str),
    /// The event has expired.
    #[error("token expired")]
    TokenExpired,
    /// The event creation time is outside the accepted replay window.
    #[error("timestamp out of window")]
    TimestampOutOfWindow,
    /// A server tag does not match the request-bound host.
    #[error("server mismatch")]
    ServerMismatch,
    /// No `x` tag matches the exact blob hash.
    #[error("hash mismatch")]
    HashMismatch,
    /// The event does not authorize the requested blob or server.
    #[error("insufficient scope")]
    InsufficientScope,
}

/// Verify common kind:24242 Blossom event validity for one exact verb.
///
/// This verifies the signature, kind, non-empty content, verb, expiration,
/// creation-time replay window, and any request-bound server tags. It does not
/// check verb-specific blob scope.
pub fn verify_blossom_auth_event_for_verb(
    auth_event: &nostr::Event,
    verb: BlossomVerb,
    server_domain: Option<&str>,
    max_age_secs: u64,
) -> Result<(), BlossomAuthError> {
    auth_event
        .verify()
        .map_err(|_| BlossomAuthError::InvalidSignature)?;

    if auth_event.kind.as_u16() != 24242 {
        return Err(BlossomAuthError::InvalidAuthKind);
    }
    if auth_event.content.trim().is_empty() {
        return Err(BlossomAuthError::InvalidAuthEvent);
    }

    let mut found_t = false;
    let mut found_exp = false;
    let mut server_tags: Vec<&str> = Vec::new();
    let mut exp_value: u64 = 0;

    for tag in auth_event.tags.iter() {
        match tag.kind().to_string().as_str() {
            "t" => {
                if let Some(value) = tag.content() {
                    if value != verb.as_str() {
                        return Err(BlossomAuthError::InvalidAuthVerb);
                    }
                    found_t = true;
                }
            }
            "expiration" => {
                if let Some(value) = tag.content() {
                    exp_value = value.parse().unwrap_or(0);
                    found_exp = true;
                }
            }
            "server" => {
                if let Some(value) = tag.content() {
                    server_tags.push(value);
                }
            }
            _ => {}
        }
    }

    if !found_t {
        return Err(BlossomAuthError::MissingTag("t"));
    }
    if !found_exp {
        return Err(BlossomAuthError::MissingTag("expiration"));
    }

    let now = nostr::Timestamp::now().as_secs();
    if exp_value <= now {
        return Err(BlossomAuthError::TokenExpired);
    }

    let created = auth_event.created_at.as_secs();
    if created > now + 5 || now > created + max_age_secs {
        return Err(BlossomAuthError::TimestampOutOfWindow);
    }

    if !server_tags.is_empty() {
        let Some(domain) = server_domain else {
            return Err(BlossomAuthError::ServerMismatch);
        };
        let expected = normalize_server_host(domain);
        if !server_tags
            .iter()
            .any(|tag| normalize_server_host(tag) == expected)
        {
            return Err(BlossomAuthError::ServerMismatch);
        }
    }

    Ok(())
}

/// Verify common upload auth event validity without checking the blob hash.
pub fn verify_blossom_auth_event(
    auth_event: &nostr::Event,
    server_domain: Option<&str>,
    max_age_secs: u64,
) -> Result<(), BlossomAuthError> {
    verify_blossom_auth_event_for_verb(auth_event, BlossomVerb::Upload, server_domain, max_age_secs)
}

/// Verify a kind:24242 upload event including the exact `x` tag blob hash.
pub fn verify_blossom_upload_auth(
    auth_event: &nostr::Event,
    sha256: &str,
    server_domain: Option<&str>,
    max_age_secs: u64,
) -> Result<(), BlossomAuthError> {
    verify_blossom_auth_event_for_verb(
        auth_event,
        BlossomVerb::Upload,
        server_domain,
        max_age_secs,
    )?;

    let has_matching_x = auth_event
        .tags
        .iter()
        .any(|tag| tag.kind().to_string() == "x" && tag.content() == Some(sha256));
    if !has_matching_x {
        return Err(BlossomAuthError::HashMismatch);
    }
    Ok(())
}

/// Verify a kind:24242 download event for one exact blob and server.
///
/// BUD-01 permits either an `x` tag matching `sha256` or a matching `server`
/// tag. Callers must still enforce relay membership after this verifier.
pub fn verify_blossom_get_auth(
    auth_event: &nostr::Event,
    sha256: &str,
    server_domain: Option<&str>,
    max_age_secs: u64,
) -> Result<(), BlossomAuthError> {
    verify_blossom_auth_event_for_verb(auth_event, BlossomVerb::Get, server_domain, max_age_secs)?;

    let has_matching_x = auth_event
        .tags
        .iter()
        .any(|tag| tag.kind().to_string() == "x" && tag.content() == Some(sha256));
    let has_matching_server = server_domain.is_some_and(|domain| {
        let expected = normalize_server_host(domain);
        auth_event.tags.iter().any(|tag| {
            tag.kind().to_string() == "server"
                && tag
                    .content()
                    .is_some_and(|value| normalize_server_host(value) == expected)
        })
    });

    if !has_matching_x && !has_matching_server {
        return Err(BlossomAuthError::InsufficientScope);
    }
    Ok(())
}

fn normalize_server_host(value: &str) -> String {
    let authority = match value.split_once("://") {
        Some((_scheme, rest)) => rest.split('/').next().unwrap_or(rest),
        None => value.split('/').next().unwrap_or(value),
    };
    buzz_core::tenant::normalize_host(authority)
}
