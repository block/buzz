//! Sign in with Slack (OIDC).
//!
//! The server verifies the returned ID token (signature, issuer, audience,
//! expiry, nonce, workspace). Identity is then confirmed again through Slack's
//! authenticated userInfo endpoint before the migration service mints an app
//! handoff code. The separate app handoff is bound to a device-held verifier
//! in [`crate::server`].

use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ring::signature::{RsaPublicKeyComponents, RSA_PKCS1_2048_8192_SHA256};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Slack OIDC application credentials. Absent means the `/oidc/*` routes are
/// disabled.
#[derive(Clone)]
pub struct OidcConfig {
    /// Slack application's OAuth client id.
    pub client_id: String,
    /// Slack application's OAuth client secret.
    pub client_secret: String,
    /// Must exactly match a redirect URL registered on the Slack app.
    pub redirect_uri: String,
    /// The only Slack workspace whose identities may be bound.
    pub team_id: String,
}

const AUTHORIZE_URL: &str = "https://slack.com/openid/connect/authorize";
const TOKEN_URL: &str = "https://slack.com/api/openid.connect.token";
const USERINFO_URL: &str = "https://slack.com/api/openid.connect.userInfo";
const JWKS_URL: &str = "https://slack.com/openid/connect/keys";
const ISSUER: &str = "https://slack.com";

/// Build a Sign in with Slack authorization URL.
///
/// This follows Slack's OpenID Connect endpoint contract. The migration app's
/// own device-verifier handoff is separate from Slack's authorization-code
/// exchange.
pub fn authorize_url(cfg: &OidcConfig, state: &str, nonce: &str) -> String {
    format!(
        "{AUTHORIZE_URL}?response_type=code&scope=openid&client_id={}&redirect_uri={}&state={}&nonce={}&team={}",
        urlencode(&cfg.client_id),
        urlencode(&cfg.redirect_uri),
        urlencode(state),
        urlencode(nonce),
        urlencode(&cfg.team_id),
    )
}

/// RFC 7636 S256 challenge for a high-entropy verifier.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

#[derive(Deserialize)]
struct TokenResp {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct UserInfoResp {
    #[serde(default)]
    ok: bool,
    #[serde(rename = "https://slack.com/user_id", default)]
    user_id: Option<String>,
    #[serde(rename = "https://slack.com/team_id", default)]
    team_id: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IdTokenClaims {
    iss: String,
    aud: Audience,
    exp: u64,
    nonce: String,
    #[serde(rename = "https://slack.com/user_id")]
    user_id: String,
    #[serde(rename = "https://slack.com/team_id")]
    team_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

impl Audience {
    fn contains(&self, expected: &str) -> bool {
        match self {
            Self::One(value) => value == expected,
            Self::Many(values) => values.iter().any(|value| value == expected),
        }
    }
}

#[derive(Deserialize)]
struct IdTokenHeader {
    alg: String,
    kid: String,
}

#[derive(Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}

#[derive(Deserialize)]
struct Jwk {
    kty: String,
    kid: String,
    #[serde(default)]
    alg: Option<String>,
    n: String,
    e: String,
}

/// Why an OIDC exchange failed.
#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("slack token exchange failed: {0}")]
    Token(String),
    #[error("slack userinfo failed: {0}")]
    UserInfo(String),
    #[error("slack did not return a user id")]
    NoUserId,
    #[error("slack OIDC nonce did not match")]
    NonceMismatch,
    #[error("slack OIDC id_token is invalid: {0}")]
    InvalidIdToken(String),
    #[error("could not verify Slack OIDC signing keys: {0}")]
    SigningKeys(String),
    #[error("authenticated Slack workspace {actual:?} does not match configured workspace")]
    WrongTeam { actual: Option<String> },
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

/// Exchange an authorization code, verify the OIDC response, and return the
/// workspace-scoped `slack:<team>:<user>` subject.
pub async fn exchange_code_for_subject(
    http: &reqwest::Client,
    cfg: &OidcConfig,
    code: &str,
    expected_nonce: &str,
) -> Result<String, OidcError> {
    if code.is_empty() {
        return Err(OidcError::Token("missing authorization code".into()));
    }
    let body = token_request_body(cfg, code);
    let token: TokenResp = http
        .post(TOKEN_URL)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if !token.ok {
        return Err(OidcError::Token(
            token.error.unwrap_or_else(|| "unknown".into()),
        ));
    }

    let access = token
        .access_token
        .ok_or_else(|| OidcError::Token("no access_token".into()))?;
    let claims = verify_id_token(
        http,
        cfg,
        token
            .id_token
            .as_deref()
            .ok_or_else(|| OidcError::InvalidIdToken("missing token".into()))?,
        expected_nonce,
        unix_now(),
    )
    .await?;

    let info: UserInfoResp = http
        .get(USERINFO_URL)
        .bearer_auth(&access)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .map_err(|error| OidcError::UserInfo(error.to_string()))?;
    if !info.ok {
        return Err(OidcError::UserInfo(
            info.error.unwrap_or_else(|| "unknown".into()),
        ));
    }
    if info.team_id.as_deref() != Some(cfg.team_id.as_str()) {
        return Err(OidcError::WrongTeam {
            actual: info.team_id,
        });
    }
    let user_id = info
        .user_id
        .filter(|value| !value.is_empty())
        .ok_or(OidcError::NoUserId)?;
    if claims.team_id != cfg.team_id || claims.user_id != user_id {
        return Err(OidcError::InvalidIdToken(
            "ID token and userInfo identity did not agree".into(),
        ));
    }
    Ok(format!("slack:{}:{user_id}", cfg.team_id))
}

fn token_request_body(cfg: &OidcConfig, code: &str) -> String {
    let body = [
        ("client_id", cfg.client_id.as_str()),
        ("client_secret", cfg.client_secret.as_str()),
        ("code", code),
        ("redirect_uri", cfg.redirect_uri.as_str()),
    ]
    .iter()
    .map(|(key, value)| format!("{}={}", urlencode(key), urlencode(value)))
    .collect::<Vec<_>>()
    .join("&");
    body
}

async fn verify_id_token(
    http: &reqwest::Client,
    cfg: &OidcConfig,
    id_token: &str,
    expected_nonce: &str,
    now: u64,
) -> Result<IdTokenClaims, OidcError> {
    let mut parts = id_token.split('.');
    let (Some(header_wire), Some(claims_wire), Some(signature_wire), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(OidcError::InvalidIdToken("malformed compact JWT".into()));
    };
    let header: IdTokenHeader = decode_json_part(header_wire)?;
    if header.alg != "RS256" || header.kid.is_empty() {
        return Err(OidcError::InvalidIdToken(
            "unsupported signing algorithm or missing key id".into(),
        ));
    }

    let jwks: JwkSet = http
        .get(JWKS_URL)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .map_err(|error| OidcError::SigningKeys(error.to_string()))?;
    let key = jwks
        .keys
        .iter()
        .find(|key| {
            key.kid == header.kid
                && key.kty == "RSA"
                && key
                    .alg
                    .as_deref()
                    .is_none_or(|algorithm| algorithm == "RS256")
        })
        .ok_or_else(|| OidcError::SigningKeys("matching RSA key not found".into()))?;
    let modulus = decode_base64url(&key.n)?;
    let exponent = decode_base64url(&key.e)?;
    let signature = decode_base64url(signature_wire)?;
    let signed = format!("{header_wire}.{claims_wire}");
    RsaPublicKeyComponents {
        n: &modulus,
        e: &exponent,
    }
    .verify(&RSA_PKCS1_2048_8192_SHA256, signed.as_bytes(), &signature)
    .map_err(|_| OidcError::InvalidIdToken("signature verification failed".into()))?;

    let claims: IdTokenClaims = decode_json_part(claims_wire)?;
    validate_id_token_claims(&claims, cfg, expected_nonce, now)?;
    Ok(claims)
}

fn validate_id_token_claims(
    claims: &IdTokenClaims,
    cfg: &OidcConfig,
    expected_nonce: &str,
    now: u64,
) -> Result<(), OidcError> {
    if claims.iss != ISSUER {
        return Err(OidcError::InvalidIdToken("issuer mismatch".into()));
    }
    if !claims.aud.contains(&cfg.client_id) {
        return Err(OidcError::InvalidIdToken("audience mismatch".into()));
    }
    if claims.exp <= now {
        return Err(OidcError::InvalidIdToken("token expired".into()));
    }
    if claims.nonce != expected_nonce {
        return Err(OidcError::NonceMismatch);
    }
    if claims.team_id != cfg.team_id || claims.user_id.is_empty() {
        return Err(OidcError::WrongTeam {
            actual: Some(claims.team_id.clone()),
        });
    }
    Ok(())
}

fn decode_json_part<T: for<'de> Deserialize<'de>>(part: &str) -> Result<T, OidcError> {
    let decoded = decode_base64url(part)?;
    serde_json::from_slice(&decoded)
        .map_err(|_| OidcError::InvalidIdToken("malformed JWT JSON".into()))
}

fn decode_base64url(value: &str) -> Result<Vec<u8>, OidcError> {
    URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| OidcError::InvalidIdToken("malformed base64url".into()))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> OidcConfig {
        OidcConfig {
            client_id: "123.456".into(),
            client_secret: "shh".into(),
            redirect_uri: "https://mig.example/oidc/callback".into(),
            team_id: "T060".into(),
        }
    }

    #[test]
    fn authorize_url_encodes_openid_params() {
        let url = authorize_url(&cfg(), "st ate", "non/ce");
        assert!(url.starts_with(AUTHORIZE_URL));
        assert!(url.contains("client_id=123.456"));
        assert!(url.contains("redirect_uri=https%3A%2F%2Fmig.example%2Foidc%2Fcallback"));
        assert!(url.contains("state=st%20ate"));
        assert!(url.contains("nonce=non%2Fce"));
        assert!(url.contains("team=T060"));
        assert!(url.contains("scope=openid"));
        assert!(!url.contains("code_challenge"));
    }

    #[test]
    fn token_request_matches_sign_in_with_slack_contract() {
        assert_eq!(
            token_request_body(&cfg(), "co/de"),
            "client_id=123.456&client_secret=shh&code=co%2Fde&redirect_uri=https%3A%2F%2Fmig.example%2Foidc%2Fcallback"
        );
    }

    #[test]
    fn pkce_challenge_matches_rfc_7636_example() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn userinfo_reads_namespaced_user_id() {
        let info: UserInfoResp = serde_json::from_str(
            r#"{"ok":true,"sub":"x","https://slack.com/user_id":"U060",
                    "https://slack.com/team_id":"T060"}"#,
        )
        .expect("parse");
        assert_eq!(info.user_id.as_deref(), Some("U060"));
        assert_eq!(info.team_id.as_deref(), Some("T060"));
    }

    #[test]
    fn token_resp_surfaces_error() {
        let token: TokenResp =
            serde_json::from_str(r#"{"ok":false,"error":"bad_code"}"#).expect("parse");
        assert!(!token.ok);
        assert_eq!(token.error.as_deref(), Some("bad_code"));
    }

    #[test]
    fn id_token_claims_validate_all_security_fields() {
        let claims = IdTokenClaims {
            iss: ISSUER.into(),
            aud: Audience::One(cfg().client_id.clone()),
            exp: 2_000,
            nonce: "nonce".into(),
            user_id: "U060".into(),
            team_id: "T060".into(),
        };
        assert!(validate_id_token_claims(&claims, &cfg(), "nonce", 1_000).is_ok());
        assert!(validate_id_token_claims(&claims, &cfg(), "wrong", 1_000).is_err());
        assert!(validate_id_token_claims(&claims, &cfg(), "nonce", 2_000).is_err());
    }
}
