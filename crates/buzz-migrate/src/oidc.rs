//! Sign in with Slack (OIDC) — the second, stronger claim channel.
//!
//! Where the email channel proves control of an inbox, this channel proves
//! control of the Slack account itself, live: the person authenticates to
//! Slack and the service reads back the verified user id. There is no bearer
//! token to leak — the proof is a fresh authorization Slack performs at claim
//! time.
//!
//! Flow:
//! 1. `GET /oidc/start?pubkey=X` — the app opens this with **its own** key. We
//!    stash `state → X` and redirect to Slack's authorize endpoint.
//! 2. Slack authenticates the person and redirects to
//!    `GET /oidc/callback?code&state`.
//! 3. We exchange the code for an access token, call Slack's userInfo endpoint,
//!    and read the verified `https://slack.com/user_id` claim. Because Slack
//!    pins that id to whoever actually authenticated, binding `slack:<id> → X`
//!    is safe — an attacker can only ever prove their *own* Slack id.
//! 4. We publish the attestation and deep-link back into the app to self-claim.
//!
//! The exchange needs Slack's `client_secret`, so it runs server-side. That is
//! the service's only Slack credential; it never touches a Buzz private key
//! other than the operator admin key used to sign attestations.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;

/// Slack OIDC application credentials. Absent → the `/oidc/*` routes return 503.
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

/// Build the Slack authorize URL to redirect the person to.
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
    /// Slack puts the workspace user id under this namespaced claim.
    #[serde(rename = "https://slack.com/user_id", default)]
    user_id: Option<String>,
    #[serde(rename = "https://slack.com/team_id", default)]
    team_id: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct IdTokenClaims {
    nonce: String,
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
    #[error("slack OIDC id_token is malformed")]
    MalformedIdToken,
    #[error("authenticated Slack workspace {actual:?} does not match configured workspace")]
    WrongTeam { actual: Option<String> },
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

/// Exchange an authorization `code` for the authenticated Slack user id.
/// Returns the `slack:<user id>` subject.
pub async fn exchange_code_for_subject(
    http: &reqwest::Client,
    cfg: &OidcConfig,
    code: &str,
    expected_nonce: &str,
) -> Result<String, OidcError> {
    if code.is_empty() {
        return Err(OidcError::Token("missing authorization code".into()));
    }
    // 1. code → access token (confidential client: client_secret required).
    // Build the x-www-form-urlencoded body by hand so we don't depend on
    // reqwest's optional form feature.
    let body = [
        ("client_id", cfg.client_id.as_str()),
        ("client_secret", cfg.client_secret.as_str()),
        ("code", code),
        ("redirect_uri", cfg.redirect_uri.as_str()),
        ("grant_type", "authorization_code"),
    ]
    .iter()
    .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v)))
    .collect::<Vec<_>>()
    .join("&");
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
    let claims = decode_id_token_claims(
        token
            .id_token
            .as_deref()
            .ok_or(OidcError::MalformedIdToken)?,
    )?;
    if claims.nonce != expected_nonce {
        return Err(OidcError::NonceMismatch);
    }

    // 2. access token → verified user id.
    let info: UserInfoResp = http
        .get(USERINFO_URL)
        .bearer_auth(&access)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .map_err(|e| OidcError::UserInfo(e.to_string()))?;
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
        .filter(|s| !s.is_empty())
        .ok_or(OidcError::NoUserId)?;
    Ok(format!("slack:{user_id}"))
}

/// Decode only the nonce from the ID token returned directly by Slack's token
/// endpoint. Identity still comes from the authenticated userInfo request.
fn decode_id_token_claims(id_token: &str) -> Result<IdTokenClaims, OidcError> {
    let payload = id_token
        .split('.')
        .nth(1)
        .ok_or(OidcError::MalformedIdToken)?;
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| OidcError::MalformedIdToken)?;
    serde_json::from_slice(&decoded).map_err(|_| OidcError::MalformedIdToken)
}

/// Same minimal percent-encoding used by the email channel (no external dep).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
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
    fn authorize_url_encodes_params() {
        let u = authorize_url(&cfg(), "st ate", "non/ce");
        assert!(u.starts_with(AUTHORIZE_URL));
        assert!(u.contains("client_id=123.456"));
        assert!(u.contains("redirect_uri=https%3A%2F%2Fmig.example%2Foidc%2Fcallback"));
        assert!(u.contains("state=st%20ate"));
        assert!(u.contains("nonce=non%2Fce"));
        assert!(u.contains("team=T060"));
        assert!(u.contains("scope=openid"));
    }

    #[test]
    fn userinfo_reads_namespaced_user_id() {
        let info: UserInfoResp = serde_json::from_str(
            r#"{"ok":true,"sub":"x","https://slack.com/user_id":"U060",
                    "https://slack.com/team_id":"T060"}"#,
        )
        .unwrap();
        assert_eq!(info.user_id.as_deref(), Some("U060"));
        assert_eq!(info.team_id.as_deref(), Some("T060"));
    }

    #[test]
    fn token_resp_surfaces_error() {
        let t: TokenResp = serde_json::from_str(r#"{"ok":false,"error":"bad_code"}"#).unwrap();
        assert!(!t.ok);
        assert_eq!(t.error.as_deref(), Some("bad_code"));
    }

    #[test]
    fn id_token_nonce_is_decoded() {
        let payload = URL_SAFE_NO_PAD.encode(r#"{"nonce":"expected"}"#);
        let claims =
            decode_id_token_claims(&format!("header.{payload}.signature")).expect("decodes");
        assert_eq!(claims.nonce, "expected");
        assert!(decode_id_token_claims("not-a-jwt").is_err());
    }
}
