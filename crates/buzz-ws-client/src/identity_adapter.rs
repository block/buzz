//! Assumed kgoose assertion API and agent-side credential source.
//! Assertions are opaque outside native transports; this validates metadata,
//! not signatures (the relay verifies issuer signatures and authorization).
use crate::federated_identity::{
    unix_now, Assertion, IdentityError, IdentitySession, IDENTITY_HEADER,
};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use nostr::{JsonUtil, Keys, PublicKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Explicit process configuration. Never log values or put them in agent prompts.
pub const ENV_KEYS: [&str; 3] = [
    "BUZZ_NIP_FI_ENDPOINT",
    "BUZZ_NIP_FI_CREDENTIAL",
    "BUZZ_NIP_FI_ORIGINS",
];

/// Validate an explicitly configured adapter address. Loopback is for the native broker/tests.
pub fn endpoint(value: &str) -> Result<url::Url, IdentityError> {
    let url = url::Url::parse(value).map_err(|_| IdentityError::Invalid)?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "[::1]"));
    if (url.scheme() != "https" && !(local && url.scheme() == "http"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
    {
        return Err(IdentityError::Invalid);
    }
    Ok(url)
}

/// Bounded adapter response, intentionally neither Debug nor Serialize.
#[derive(Deserialize)]
pub struct AdapterResponse {
    /// Compact JWS, never surfaced to renderer state.
    pub assertion: String,
    /// Exact proof key.
    pub nostr_pubkey: String,
    /// Adapter's effective deadline, no later than JWT exp.
    pub expires_at: u64,
    /// Returned only by the assumed /agent-delegations endpoint. Key-scoped renewal credential.
    #[serde(default)]
    pub agent_credential: Option<String>,
}

impl AdapterResponse {
    /// Check the selected token class and untrusted metadata before caching.
    /// Issuer/audience policy and cryptographic verification remain server-side.
    pub fn into_assertion(self, key: PublicKey, now: u64) -> Result<Assertion, IdentityError> {
        let mut parts = self.assertion.split('.');
        let decode = |part: Option<&str>| -> Result<serde_json::Value, IdentityError> {
            let bytes = URL_SAFE_NO_PAD
                .decode(part.ok_or(IdentityError::Invalid)?)
                .map_err(|_| IdentityError::Invalid)?;
            serde_json::from_slice(&bytes).map_err(|_| IdentityError::Invalid)
        };
        if self.assertion.len() > 16 * 1024 {
            return Err(IdentityError::Invalid);
        }
        let header = decode(parts.next())?;
        let claims = decode(parts.next())?;
        let exp = claims["exp"].as_u64().ok_or(IdentityError::Invalid)?;
        let iat = claims["iat"].as_u64().ok_or(IdentityError::Invalid)?;
        if header["typ"] != "nip-fi+jwt"
            || header["alg"] == "none"
            || header["alg"].as_str().is_none_or(str::is_empty)
            || self.nostr_pubkey != key.to_hex()
            || claims["nostr_pubkey"] != key.to_hex()
            || claims["iss"].as_str().is_none_or(str::is_empty)
            || claims["sub"].as_str().is_none_or(str::is_empty)
            || claims["aud"].as_str().is_none_or(str::is_empty)
            || iat > now.saturating_add(5)
            || exp <= iat
            || self.expires_at > exp
            || claims
                .get("nbf")
                .is_some_and(|v| v.as_u64().is_none_or(|n| n > now.saturating_add(5)))
        {
            return Err(IdentityError::Invalid);
        }
        Ok(
            Assertion::new(&self.assertion, key, self.expires_at, now)?.bind(
                claims["iss"].as_str().ok_or(IdentityError::Invalid)?.into(),
                claims["sub"].as_str().ok_or(IdentityError::Invalid)?.into(),
                claims["aud"].as_str().ok_or(IdentityError::Invalid)?.into(),
            ),
        )
    }
}

/// Exchange a login/delegated credential and a locally signed proof for a JWT.
/// The adapter must authorize the binding; merely knowing a pubkey is insufficient.
pub async fn exchange(
    client: &reqwest::Client,
    endpoint_url: &str,
    credential: &str,
    keys: &Keys,
    relay_url: &str,
    auth_tag: Option<&nostr::Tag>,
) -> Result<AdapterResponse, IdentityError> {
    let endpoint = endpoint(endpoint_url)?;
    let body = serde_json::to_vec(&serde_json::json!({
        "nostr_pubkey": keys.public_key().to_hex(), "relay_url": relay_url,
        "auth_tag": auth_tag.map(|tag| tag.as_slice())
    }))
    .map_err(|_| IdentityError::Invalid)?;
    let event = nostr::EventBuilder::new(nostr::Kind::Custom(27235), "")
        .tags(
            [
                nostr::Tag::parse(["u", endpoint.as_str()]),
                nostr::Tag::parse(["method", "POST"]),
                nostr::Tag::parse(["payload", &hex::encode(Sha256::digest(&body))]),
            ]
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| IdentityError::Invalid)?,
        )
        .sign_with_keys(keys)
        .map_err(|_| IdentityError::Invalid)?;
    let mut secret =
        reqwest::header::HeaderValue::from_str(credential).map_err(|_| IdentityError::Invalid)?;
    secret.set_sensitive(true);
    let mut response = client
        .post(endpoint)
        .header("X-BB-Session-Credential", secret)
        .header(
            "Authorization",
            format!("Nostr {}", STANDARD.encode(event.as_json())),
        )
        .header("Content-Type", "application/json")
        .body(body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| IdentityError::Unavailable)?;
    if !response.status().is_success() {
        return Err(if response.status().is_client_error() {
            IdentityError::LoginRequired
        } else {
            IdentityError::Unavailable
        });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| IdentityError::Unavailable)?
    {
        if bytes.len().saturating_add(chunk.len()) > 24 * 1024 {
            return Err(IdentityError::Invalid);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| IdentityError::Invalid)
}

/// Resolve agent credentials at each operation, allowing long-running CLI/MCP
/// processes to renew rather than inheriting a single expiring human JWT.
/// Missing all configuration preserves OSS behavior; partial config fails closed.
pub async fn environment_header(
    destination: &str,
    keys: &Keys,
) -> Result<Option<reqwest::header::HeaderValue>, IdentityError> {
    Ok(environment_admission(destination, keys)
        .await?
        .map(|(header, _)| header))
}

/// Obtain a header with its adapter-bounded deadline for a new socket.
pub async fn environment_admission(
    destination: &str,
    keys: &Keys,
) -> Result<Option<(reqwest::header::HeaderValue, u64)>, IdentityError> {
    let values = ENV_KEYS.map(std::env::var);
    if values
        .iter()
        .all(|v| matches!(v, Err(std::env::VarError::NotPresent)))
    {
        return Ok(None);
    }
    let [endpoint, credential, origins] = values;
    let endpoint = endpoint.map_err(|_| IdentityError::Invalid)?;
    let credential = credential.map_err(|_| IdentityError::Invalid)?;
    let origins = origins.map_err(|_| IdentityError::Invalid)?;
    if origins.is_empty() || credential.is_empty() {
        return Err(IdentityError::Invalid);
    }
    let session = IdentitySession::new(&origins.split(',').collect::<Vec<_>>())?;
    if !session.protects(destination)? {
        return Ok(None);
    }
    let relay_url = std::env::var("BUZZ_RELAY_URL").map_err(|_| IdentityError::Invalid)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| IdentityError::Unavailable)?;
    let auth_tag = std::env::var("BUZZ_AUTH_TAG")
        .ok()
        .map(|v| serde_json::from_str::<nostr::Tag>(&v))
        .transpose()
        .map_err(|_| IdentityError::Invalid)?;
    let response = exchange(
        &client,
        &endpoint,
        &credential,
        keys,
        &relay_url,
        auth_tag.as_ref(),
    )
    .await?;
    let deadline = response.expires_at;
    session.install(0, response.into_assertion(keys.public_key(), unix_now()?)?)?;
    Ok(session
        .header(destination, keys.public_key(), unix_now()?)?
        .map(|header| (header, deadline)))
}

/// Apply assertion to a fully constructed request, using its actual URL.
/// All clients using this helper must disable redirects.
pub async fn authorize(
    request: reqwest::RequestBuilder,
    keys: &Keys,
) -> Result<reqwest::RequestBuilder, IdentityError> {
    let url = request
        .try_clone()
        .ok_or(IdentityError::Invalid)?
        .build()
        .map_err(|_| IdentityError::Invalid)?
        .url()
        .to_string();
    Ok(match environment_header(&url, keys).await? {
        Some(header) => request.header(IDENTITY_HEADER, header),
        None => request,
    })
}

#[cfg(test)]
#[path = "identity_adapter_tests.rs"]
mod tests;
