//! Request-bound HMAC verification for the NIP-FI trusted-proxy profile.
//!
//! Verification is read-only. The returned sealed evidence contains a nonce
//! claim that the final-admission transaction must consume atomically. The
//! read performed here rejects already committed nonces early but is not a
//! substitute for that final compare-and-insert.

use std::{
    fmt,
    future::Future,
    net::{Ipv4Addr, Ipv6Addr},
    pin::Pin,
    str::FromStr,
    time::Duration,
};

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::evidence::{proof_transport_code, SealedTransportEvidence, TrustedProxyNonceClaim};
use crate::ProofTransport;

/// Exact NIP-FI assertion header name.
pub const ASSERTION_HEADER_NAME: &str = "nostr-federated-identity";
/// Exact NIP-FI trusted-proxy provenance header name.
pub const PROVENANCE_HEADER_NAME: &str = "nostr-federated-identity-provenance";

const MAC_DOMAIN: &[u8] = b"NIP-FI-PROXY-1";
const MIN_SECRET_BYTES: usize = 32;
const MAX_SECRET_BYTES: usize = 4096;
const MAX_ACTIVE_SECRETS: usize = 4;
const MAX_ASSERTION_FIELD_BYTES: usize = 64 * 1024;
const MAX_METHOD_BYTES: usize = 32;
const MAX_AUTHORITY_BYTES: usize = 255;
const MAX_PATH_AND_QUERY_BYTES: usize = 16 * 1024;
const DEFAULT_MAX_PROVENANCE_FIELD_BYTES: usize = 1024;
const DEFAULT_MAX_NONCE_BYTES: usize = 64;
const MAX_PROVENANCE_AGE_SECONDS: u64 = 86_400;
const MAX_FUTURE_SKEW_SECONDS: u64 = 300;

type ProvenanceMac = Hmac<Sha256>;

/// One raw HTTP header from the complete request header collection.
///
/// Names are compared case-insensitively. Values are retained as bytes so
/// non-ASCII and ambiguous encodings can be rejected before parsing. Debug
/// output never includes the value.
pub struct HttpHeaderField<'a> {
    name: &'a str,
    value: &'a [u8],
}

impl<'a> HttpHeaderField<'a> {
    /// Borrow one raw header occurrence without selecting or combining it.
    pub const fn new(name: &'a str, value: &'a [u8]) -> Self {
        Self { name, value }
    }
}

impl fmt::Debug for HttpHeaderField<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HttpHeaderField([REDACTED])")
    }
}

/// Exact request coordinates resolved by trusted server routing.
pub struct TrustedProxyRequest {
    authorization_domain: CommunityId,
    method: Box<str>,
    authority: Box<str>,
    path_and_query: Box<str>,
    body_digest: [u8; 32],
    transport: ProofTransport,
}

impl TrustedProxyRequest {
    /// Validate server-owned coordinates and hash the exact request body.
    pub fn from_server_request(
        authorization_domain: CommunityId,
        transport: ProofTransport,
        method: &str,
        authority: &str,
        path_and_query: &str,
        body: &[u8],
    ) -> Result<Self, TrustedProxyError> {
        let body_digest = Sha256::digest(body).into();
        Self::from_server_parts(
            authorization_domain,
            transport,
            method,
            authority,
            path_and_query,
            body_digest,
        )
    }

    /// Validate server-owned coordinates with a previously computed body digest.
    ///
    /// The digest must be SHA-256 over the exact request body after proxy
    /// rewriting, including the empty body used by a WebSocket upgrade.
    pub fn from_server_parts(
        authorization_domain: CommunityId,
        transport: ProofTransport,
        method: &str,
        authority: &str,
        path_and_query: &str,
        body_digest: [u8; 32],
    ) -> Result<Self, TrustedProxyError> {
        if authorization_domain.as_uuid().is_nil()
            || !valid_method(method)
            || !valid_authority(authority)
        {
            return Err(TrustedProxyError::InvalidRequest);
        }
        let path_and_query = canonical_path_and_query(path_and_query)?;
        Ok(Self {
            authorization_domain,
            method: method.into(),
            authority: authority.into(),
            path_and_query,
            body_digest,
            transport,
        })
    }
}

impl fmt::Debug for TrustedProxyRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TrustedProxyRequest([REDACTED])")
    }
}

/// Failure returned by a read-only committed-nonce lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("trusted-proxy replay state is unavailable")]
pub struct TrustedProxyReplayReadError;

/// Read-only view of committed trusted-proxy nonce claims.
///
/// Returning `false` does not reserve the nonce. Final admission must still
/// atomically insert the exact claim carried by [`SealedTransportEvidence`].
pub trait TrustedProxyNonceReplayReader: Send + Sync {
    /// Report whether the exact privacy-safe claim key is already committed.
    fn is_committed<'a>(
        &'a self,
        authorization_domain: CommunityId,
        claim: &'a TrustedProxyNonceClaim,
    ) -> Pin<Box<dyn Future<Output = Result<bool, TrustedProxyReplayReadError>> + Send + 'a>>;
}

/// Closed trusted-proxy verification failures with no credential material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TrustedProxyError {
    /// Secret set or time/size policy was invalid.
    #[error("invalid trusted-proxy configuration")]
    InvalidConfiguration,
    /// Required assertion field was absent.
    #[error("trusted-proxy assertion is missing")]
    MissingAssertion,
    /// Required provenance field was absent; direct ingress cannot fall back.
    #[error("trusted-proxy provenance is missing")]
    MissingProvenance,
    /// A protected field was repeated, combined, or ambiguously encoded.
    #[error("trusted-proxy header is ambiguous")]
    AmbiguousHeader,
    /// Bearer assertion framing was malformed or exceeded its bound.
    #[error("trusted-proxy assertion is malformed")]
    MalformedAssertion,
    /// Provenance framing, timestamp, nonce, or MAC was malformed.
    #[error("trusted-proxy provenance is malformed")]
    MalformedProvenance,
    /// Server-resolved request coordinates were not canonical.
    #[error("trusted-proxy request binding is invalid")]
    InvalidRequest,
    /// Provenance was at or beyond its exclusive maximum-age bound.
    #[error("trusted-proxy provenance is expired")]
    Expired,
    /// Provenance timestamp exceeded the configured future skew.
    #[error("trusted-proxy provenance is future-dated")]
    FutureDated,
    /// No active secret authenticated the exact request-bound provenance.
    #[error("trusted-proxy provenance was rejected")]
    InvalidMac,
    /// The nonce was already committed by a prior final admission.
    #[error("trusted-proxy nonce was already committed")]
    Replay,
    /// Current committed-nonce state could not be read.
    #[error("trusted-proxy replay state is unavailable")]
    ReplayUnavailable,
}

impl TrustedProxyError {
    /// Stable privacy-safe failure code for metrics and internal decisions.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "nip_fi_proxy_invalid_configuration",
            Self::MissingAssertion => "nip_fi_proxy_missing_assertion",
            Self::MissingProvenance => "nip_fi_proxy_missing_provenance",
            Self::AmbiguousHeader => "nip_fi_proxy_ambiguous_header",
            Self::MalformedAssertion => "nip_fi_proxy_malformed_assertion",
            Self::MalformedProvenance => "nip_fi_proxy_malformed_provenance",
            Self::InvalidRequest => "nip_fi_proxy_invalid_request",
            Self::Expired => "nip_fi_proxy_expired",
            Self::FutureDated => "nip_fi_proxy_future_dated",
            Self::InvalidMac => "nip_fi_proxy_invalid_mac",
            Self::Replay => "nip_fi_proxy_replay",
            Self::ReplayUnavailable => "nip_fi_proxy_replay_unavailable",
        }
    }
}

/// Verifies `trusted-proxy-hmac-v1` provenance under a finite secret set.
pub struct TrustedProxyProvenanceVerifier {
    active_secrets: Vec<Box<[u8]>>,
    maximum_provenance_age_seconds: u64,
    future_skew_seconds: u64,
    maximum_provenance_field_bytes: usize,
    maximum_nonce_bytes: usize,
}

impl TrustedProxyProvenanceVerifier {
    /// Construct with secure finite default field and nonce bounds.
    pub fn new(
        active_secrets: Vec<Vec<u8>>,
        maximum_provenance_age: Duration,
        future_skew: Duration,
    ) -> Result<Self, TrustedProxyError> {
        Self::with_size_limits(
            active_secrets,
            maximum_provenance_age,
            future_skew,
            DEFAULT_MAX_PROVENANCE_FIELD_BYTES,
            DEFAULT_MAX_NONCE_BYTES,
        )
    }

    /// Construct with explicit finite provenance-field and decoded-nonce bounds.
    pub fn with_size_limits(
        active_secrets: Vec<Vec<u8>>,
        maximum_provenance_age: Duration,
        future_skew: Duration,
        maximum_provenance_field_bytes: usize,
        maximum_nonce_bytes: usize,
    ) -> Result<Self, TrustedProxyError> {
        let maximum_provenance_age_seconds = maximum_provenance_age.as_secs();
        let future_skew_seconds = future_skew.as_secs();
        let valid_secrets = !active_secrets.is_empty()
            && active_secrets.len() <= MAX_ACTIVE_SECRETS
            && active_secrets
                .iter()
                .all(|secret| (MIN_SECRET_BYTES..=MAX_SECRET_BYTES).contains(&secret.len()))
            && !has_duplicate_secrets(&active_secrets);
        if !valid_secrets
            || maximum_provenance_age.subsec_nanos() != 0
            || maximum_provenance_age_seconds == 0
            || maximum_provenance_age_seconds > MAX_PROVENANCE_AGE_SECONDS
            || future_skew.subsec_nanos() != 0
            || future_skew_seconds > MAX_FUTURE_SKEW_SECONDS
            || !(128..=4096).contains(&maximum_provenance_field_bytes)
            || !(16..=1024).contains(&maximum_nonce_bytes)
        {
            return Err(TrustedProxyError::InvalidConfiguration);
        }
        Ok(Self {
            active_secrets: active_secrets
                .into_iter()
                .map(Vec::into_boxed_slice)
                .collect(),
            maximum_provenance_age_seconds,
            future_skew_seconds,
            maximum_provenance_field_bytes,
            maximum_nonce_bytes,
        })
    }

    /// Verify exact headers and request binding, then reject committed nonces.
    ///
    /// `headers` must contain every occurrence from the complete request header
    /// collection. Missing provenance never falls back to client-attached mode.
    /// The replay lookup is read-only; the returned nonce claim must be consumed
    /// atomically by final admission.
    pub async fn verify<R: TrustedProxyNonceReplayReader + ?Sized>(
        &self,
        headers: &[HttpHeaderField<'_>],
        request: &TrustedProxyRequest,
        now: DateTime<Utc>,
        replay: &R,
    ) -> Result<SealedTransportEvidence, TrustedProxyError> {
        let assertion_field = exact_header(
            headers,
            ASSERTION_HEADER_NAME,
            TrustedProxyError::MissingAssertion,
            MAX_ASSERTION_FIELD_BYTES,
        )?;
        let provenance_field = exact_header(
            headers,
            PROVENANCE_HEADER_NAME,
            TrustedProxyError::MissingProvenance,
            self.maximum_provenance_field_bytes,
        )?;
        let assertion = parse_bearer_assertion(assertion_field)?;
        let parsed = self.parse_provenance(provenance_field, now)?;
        let assertion_digest: [u8; 32] = Sha256::digest(assertion.as_bytes()).into();
        let mac_input = mac_input(parsed.timestamp, &parsed.nonce, &assertion_digest, request);
        let mut authenticated = 0_u8;
        for secret in &self.active_secrets {
            let mut mac = <ProvenanceMac as KeyInit>::new_from_slice(secret)
                .map_err(|_| TrustedProxyError::InvalidConfiguration)?;
            mac.update(&mac_input);
            authenticated |= u8::from(mac.verify_slice(&parsed.mac).is_ok());
        }
        if authenticated != 1 {
            return Err(TrustedProxyError::InvalidMac);
        }

        let claim_key = framed_digest(b"buzz:nip-fi:trusted-proxy-nonce:v1", &[&parsed.nonce]);
        let claim = TrustedProxyNonceClaim::new(claim_key, parsed.expires_at);
        match replay
            .is_committed(request.authorization_domain, &claim)
            .await
        {
            Ok(false) => {}
            Ok(true) => return Err(TrustedProxyError::Replay),
            Err(_) => return Err(TrustedProxyError::ReplayUnavailable),
        }

        Ok(SealedTransportEvidence::from_trusted_proxy(
            request.authorization_domain,
            assertion.into(),
            assertion_digest,
            request.method.as_bytes(),
            request.authority.as_bytes(),
            request.path_and_query.as_bytes(),
            request.body_digest,
            request.transport,
            parsed.expires_at,
            claim,
        ))
    }

    fn parse_provenance(
        &self,
        field: &[u8],
        now: DateTime<Utc>,
    ) -> Result<ParsedProvenance, TrustedProxyError> {
        let field =
            std::str::from_utf8(field).map_err(|_| TrustedProxyError::MalformedProvenance)?;
        let mut components = field.split('.');
        if components.next() != Some("v1") {
            return Err(TrustedProxyError::MalformedProvenance);
        }
        let timestamp_text = components
            .next()
            .ok_or(TrustedProxyError::MalformedProvenance)?;
        let nonce_text = components
            .next()
            .ok_or(TrustedProxyError::MalformedProvenance)?;
        let mac_text = components
            .next()
            .ok_or(TrustedProxyError::MalformedProvenance)?;
        if components.next().is_some() || !canonical_unsigned_decimal(timestamp_text) {
            return Err(TrustedProxyError::MalformedProvenance);
        }
        let timestamp = timestamp_text
            .parse::<u64>()
            .map_err(|_| TrustedProxyError::MalformedProvenance)?;
        let nonce = decode_base64url(nonce_text, self.maximum_nonce_bytes)?;
        if nonce.len() < 16 {
            return Err(TrustedProxyError::MalformedProvenance);
        }
        let mac = decode_base64url(mac_text, 32)?;
        let mac: [u8; 32] = mac
            .try_into()
            .map_err(|_| TrustedProxyError::MalformedProvenance)?;
        let now = u64::try_from(now.timestamp()).map_err(|_| TrustedProxyError::InvalidRequest)?;
        let latest = now
            .checked_add(self.future_skew_seconds)
            .ok_or(TrustedProxyError::FutureDated)?;
        if timestamp > latest {
            return Err(TrustedProxyError::FutureDated);
        }
        let expires_at = timestamp
            .checked_add(self.maximum_provenance_age_seconds)
            .ok_or(TrustedProxyError::MalformedProvenance)?;
        if now >= expires_at {
            return Err(TrustedProxyError::Expired);
        }
        let expires_at = i64::try_from(expires_at)
            .ok()
            .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
            .ok_or(TrustedProxyError::MalformedProvenance)?;
        Ok(ParsedProvenance {
            timestamp,
            nonce,
            mac,
            expires_at,
        })
    }
}

impl fmt::Debug for TrustedProxyProvenanceVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TrustedProxyProvenanceVerifier([REDACTED])")
    }
}

struct ParsedProvenance {
    timestamp: u64,
    nonce: Vec<u8>,
    mac: [u8; 32],
    expires_at: DateTime<Utc>,
}

fn exact_header<'a>(
    headers: &[HttpHeaderField<'a>],
    expected_name: &str,
    missing: TrustedProxyError,
    maximum_bytes: usize,
) -> Result<&'a [u8], TrustedProxyError> {
    let mut matching = headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case(expected_name));
    let field = matching.next().ok_or(missing)?;
    if matching.next().is_some() {
        return Err(TrustedProxyError::AmbiguousHeader);
    }
    if field.value.is_empty()
        || field.value.len() > maximum_bytes
        || !field.value.is_ascii()
        || field.value.contains(&b',')
        || field.value.first().is_some_and(u8::is_ascii_whitespace)
        || field.value.last().is_some_and(u8::is_ascii_whitespace)
    {
        return Err(TrustedProxyError::AmbiguousHeader);
    }
    Ok(field.value)
}

fn parse_bearer_assertion(field: &[u8]) -> Result<&str, TrustedProxyError> {
    let field = std::str::from_utf8(field).map_err(|_| TrustedProxyError::MalformedAssertion)?;
    let assertion = field
        .strip_prefix("Bearer ")
        .ok_or(TrustedProxyError::MalformedAssertion)?;
    let mut segments = assertion.split('.');
    let valid = assertion.len() <= MAX_ASSERTION_FIELD_BYTES - "Bearer ".len()
        && segments.by_ref().take(3).count() == 3
        && segments.next().is_none()
        && assertion
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && assertion.split('.').all(|segment| !segment.is_empty());
    if !valid {
        return Err(TrustedProxyError::MalformedAssertion);
    }
    Ok(assertion)
}

fn valid_method(method: &str) -> bool {
    !method.is_empty()
        && method.len() <= MAX_METHOD_BYTES
        && method.bytes().all(|byte| {
            byte.is_ascii_uppercase()
                || byte.is_ascii_digit()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn valid_authority(authority: &str) -> bool {
    if authority.is_empty()
        || authority.len() > MAX_AUTHORITY_BYTES
        || !authority.is_ascii()
        || authority.bytes().any(|byte| byte.is_ascii_uppercase())
        || authority.contains('@')
        || authority.ends_with('.')
    {
        return false;
    }
    if let Some(rest) = authority.strip_prefix('[') {
        let Some((address, port)) = rest.split_once("]:") else {
            return false;
        };
        return Ipv6Addr::from_str(address).is_ok_and(|parsed| parsed.to_string() == address)
            && valid_port(port);
    }
    let Some((host, port)) = authority.rsplit_once(':') else {
        return false;
    };
    if host.is_empty() || host.contains(':') || !valid_port(port) {
        return false;
    }
    let valid_host = host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && label
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            && label
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric)
    });
    if !valid_host {
        return false;
    }
    if host
        .split('.')
        .all(|label| label.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Ipv4Addr::from_str(host).is_ok_and(|parsed| parsed.to_string() == host);
    }
    true
}

fn valid_port(port: &str) -> bool {
    !port.is_empty()
        && (port == "0" || !port.starts_with('0'))
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && port.parse::<u16>().is_ok_and(|port| port != 0)
}

fn canonical_path_and_query(value: &str) -> Result<Box<str>, TrustedProxyError> {
    if value.len() > MAX_PATH_AND_QUERY_BYTES
        || !value.is_ascii()
        || value
            .bytes()
            .any(|byte| !(0x21..=0x7e).contains(&byte) || byte == b'#')
        || has_invalid_percent_encoding(value.as_bytes())
    {
        return Err(TrustedProxyError::InvalidRequest);
    }
    let canonical = if value.is_empty() {
        "/".to_owned()
    } else if value.starts_with('?') {
        format!("/{value}")
    } else if value.starts_with('/') {
        value.to_owned()
    } else {
        return Err(TrustedProxyError::InvalidRequest);
    };
    Ok(canonical.into_boxed_str())
}

fn has_invalid_percent_encoding(value: &[u8]) -> bool {
    let mut index = 0;
    while index < value.len() {
        if value[index] == b'%' {
            if index + 2 >= value.len()
                || !value[index + 1].is_ascii_hexdigit()
                || !value[index + 2].is_ascii_hexdigit()
            {
                return true;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    false
}

fn canonical_unsigned_decimal(value: &str) -> bool {
    !value.is_empty()
        && (value == "0" || !value.starts_with('0'))
        && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn mac_input(
    timestamp: u64,
    nonce: &[u8],
    assertion_digest: &[u8; 32],
    request: &TrustedProxyRequest,
) -> Vec<u8> {
    let mut input = Vec::with_capacity(
        MAC_DOMAIN.len()
            + 8 * 9
            + 8
            + 1
            + nonce.len()
            + assertion_digest.len()
            + request.authorization_domain.as_uuid().as_bytes().len()
            + request.method.len()
            + request.authority.len()
            + request.path_and_query.len()
            + request.body_digest.len(),
    );
    input.extend_from_slice(MAC_DOMAIN);
    append_length_prefixed(&mut input, &timestamp.to_be_bytes());
    append_length_prefixed(&mut input, nonce);
    append_length_prefixed(&mut input, assertion_digest);
    append_length_prefixed(
        &mut input,
        request.authorization_domain.as_uuid().as_bytes(),
    );
    append_length_prefixed(&mut input, request.method.as_bytes());
    append_length_prefixed(&mut input, request.authority.as_bytes());
    append_length_prefixed(&mut input, request.path_and_query.as_bytes());
    append_length_prefixed(&mut input, &request.body_digest);
    append_length_prefixed(&mut input, &[proof_transport_code(request.transport)]);
    input
}

fn append_length_prefixed(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u64).to_be_bytes());
    output.extend_from_slice(value);
}

fn framed_digest(domain: &[u8], fields: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(domain);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    digest.finalize().into()
}

fn has_duplicate_secrets(secrets: &[Vec<u8>]) -> bool {
    secrets.iter().enumerate().any(|(index, candidate)| {
        secrets
            .iter()
            .skip(index + 1)
            .any(|other| candidate == other)
    })
}

fn decode_base64url(
    value: &str,
    maximum_decoded_bytes: usize,
) -> Result<Vec<u8>, TrustedProxyError> {
    if value.is_empty()
        || value.len() % 4 == 1
        || value.len() > maximum_decoded_bytes.saturating_mul(4).div_ceil(3)
        || !value.bytes().all(|byte| base64url_value(byte).is_some())
    {
        return Err(TrustedProxyError::MalformedProvenance);
    }
    let mut output = Vec::with_capacity(value.len() * 3 / 4 + 2);
    let mut accumulator = 0_u32;
    let mut bits = 0_u8;
    for byte in value.bytes() {
        let decoded = base64url_value(byte).ok_or(TrustedProxyError::MalformedProvenance)?;
        accumulator = (accumulator << 6) | u32::from(decoded);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((accumulator >> bits) as u8);
            accumulator &= (1_u32 << bits).wrapping_sub(1);
        }
    }
    if (bits > 0 && accumulator != 0)
        || output.len() > maximum_decoded_bytes
        || encode_base64url(&output) != value
    {
        return Err(TrustedProxyError::MalformedProvenance);
    }
    Ok(output)
}

fn base64url_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'-' => Some(62),
        b'_' => Some(63),
        _ => None,
    }
}

fn encode_base64url(value: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::with_capacity(value.len() * 4 / 3 + 3);
    for chunk in value.chunks(3) {
        let first = chunk[0];
        output.push(ALPHABET[(first >> 2) as usize] as char);
        let second_high = chunk.get(1).copied().unwrap_or(0);
        output.push(ALPHABET[(((first & 0x03) << 4) | (second_high >> 4)) as usize] as char);
        if let Some(second) = chunk.get(1).copied() {
            let third_high = chunk.get(2).copied().unwrap_or(0);
            output.push(ALPHABET[(((second & 0x0f) << 2) | (third_high >> 6)) as usize] as char);
        }
        if let Some(third) = chunk.get(2).copied() {
            output.push(ALPHABET[(third & 0x3f) as usize] as char);
        }
    }
    output
}
