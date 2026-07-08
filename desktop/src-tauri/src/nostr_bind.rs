use chrono::{DateTime, Utc};
use url::Url;

pub(crate) const AUDIENCE: &str = "buzz:nostr-identity";
pub(crate) const ACTION: &str = "bind_nostr_identity";
pub(crate) const CONTENT: &str = "";
pub(crate) const KIND: u16 = buzz_core_pkg::kind::KIND_NOSTR_IDENTITY_BINDING as u16;
pub(crate) const PROTOCOL: &str = "buzz-nostr-identity";
pub(crate) const RETURN_MODE: &str = "clipboard";
pub(crate) const VERSION: &str = "1";

const NONCE_CHARS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";

pub(crate) fn validate_challenge_id(challenge_id: &str) -> Result<(), String> {
    if challenge_id.is_empty() {
        return Err("challenge_id is required".into());
    }
    uuid::Uuid::parse_str(challenge_id).map_err(|_| "invalid challenge_id".to_string())?;
    Ok(())
}

pub(crate) fn validate_nonce(nonce: &str) -> Result<(), String> {
    if nonce.is_empty() {
        return Err("nonce is required".into());
    }
    if nonce.len() != 43 || !nonce.chars().all(|ch| NONCE_CHARS.contains(ch)) {
        return Err("invalid nonce".into());
    }
    Ok(())
}

pub(crate) fn validate_protocol_fields(
    audience: &str,
    action: &str,
    protocol: &str,
    version: &str,
) -> Result<(), String> {
    if audience != AUDIENCE {
        return Err("unsupported audience".into());
    }
    if action != ACTION {
        return Err("unsupported action".into());
    }
    if protocol != PROTOCOL {
        return Err("unsupported protocol".into());
    }
    if version != VERSION {
        return Err("unsupported version".into());
    }
    Ok(())
}

pub(crate) fn validate_origin(origin: &str) -> Result<(), String> {
    let parsed = Url::parse(origin).map_err(|error| format!("invalid origin: {error}"))?;
    if parsed.scheme() != "https" {
        return Err("origin must use https".into());
    }
    if parsed.host_str().is_none() {
        return Err("origin missing host".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("origin must not include credentials".into());
    }
    if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("origin must not include path, query, or fragment".into());
    }
    Ok(())
}

pub(crate) fn validate_expires_at_format(expires_at: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(expires_at)
        .map_err(|error| format!("invalid expires_at: {error}"))
        .map(|parsed| parsed.with_timezone(&Utc))
}

pub(crate) fn validate_expires_at(expires_at: &str) -> Result<(), String> {
    let parsed = validate_expires_at_format(expires_at)?;
    if parsed <= Utc::now() {
        return Err("expires_at is expired".into());
    }
    Ok(())
}

pub(crate) fn validate_signing_request(
    challenge_id: &str,
    nonce: &str,
    origin: &str,
    expires_at: &str,
) -> Result<(), String> {
    validate_challenge_id(challenge_id)?;
    validate_nonce(nonce)?;
    validate_origin(origin)?;
    validate_expires_at(expires_at)?;
    Ok(())
}
