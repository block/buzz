//! Public-client authorization-code/PKCE transport. The application owns browser callbacks and secure storage.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use zeroize::Zeroizing;

/// Non-secret, release-selected corporate login configuration.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnterpriseLoginConfig {
    /// Trusted signer base URL, including deployment prefix.
    pub signer_url: String,
    /// Auth0 HTTPS issuer origin.
    pub issuer: String,
    /// Registered native/public client ID (never a client secret).
    pub client_id: String,
    /// Dedicated signer API audience.
    pub audience: String,
    /// Exact Auth0 organization.
    pub organization: String,
    /// Exact corporate federation connection.
    pub connection: String,
}

impl EnterpriseLoginConfig {
    /// Validate the release-owned destinations before opening a browser or transmitting credentials.
    pub fn validate(&self) -> Result<(), String> {
        for (value, origin) in [(&self.issuer, true), (&self.signer_url, false)] {
            let url = url::Url::parse(value).map_err(|_| "Invalid enterprise URL")?;
            if url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || (origin && url.path() != "/")
            {
                return Err("Enterprise endpoints must be trusted HTTPS URLs".into());
            }
        }
        if [
            &self.client_id,
            &self.audience,
            &self.organization,
            &self.connection,
        ]
        .iter()
        .any(|s| s.is_empty() || s.len() > 1024)
        {
            return Err("Incomplete enterprise login configuration".into());
        }
        Ok(())
    }
}

/// One PKCE attempt. No Debug/Serialize: the verifier is a temporary credential.
pub struct EnterpriseLoginAttempt {
    verifier: Zeroizing<String>,
    state: String,
    redirect_uri: String,
}
impl EnterpriseLoginAttempt {
    /// Create from OS-random bytes supplied by the native platform (32 bytes each).
    pub fn new(verifier_entropy: [u8; 32], state_entropy: [u8; 32], redirect_uri: String) -> Self {
        Self {
            verifier: Zeroizing::new(URL_SAFE_NO_PAD.encode(verifier_entropy)),
            state: URL_SAFE_NO_PAD.encode(state_entropy),
            redirect_uri,
        }
    }
    /// Build authorization URL with PKCE S256 and exact organization/connection.
    pub fn authorization_url(&self, config: &EnterpriseLoginConfig) -> Result<url::Url, String> {
        config.validate()?;
        let mut url = url::Url::parse(&format!(
            "{}/authorize",
            config.issuer.trim_end_matches('/')
        ))
        .map_err(|_| "Invalid issuer")?;
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", config.client_id.as_str()),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("audience", config.audience.as_str()),
            ("organization", config.organization.as_str()),
            ("connection", config.connection.as_str()),
            ("scope", "openid profile email offline_access"),
            ("state", self.state.as_str()),
            ("code_challenge_method", "S256"),
            (
                "code_challenge",
                URL_SAFE_NO_PAD
                    .encode(Sha256::digest(self.verifier.as_bytes()))
                    .as_str(),
            ),
        ]);
        Ok(url)
    }
    /// Verify callback destination and state before consuming the authorization code.
    pub fn callback_code(&self, callback: &url::Url) -> Result<String, String> {
        let mut destination = callback.clone();
        destination.set_query(None);
        destination.set_fragment(None);
        if destination.as_str() != self.redirect_uri || callback.fragment().is_some() {
            return Err("Invalid login callback destination".into());
        }
        let pairs: Vec<_> = callback.query_pairs().collect();
        let states: Vec<_> = pairs.iter().filter(|(k, _)| k == "state").collect();
        let codes: Vec<_> = pairs.iter().filter(|(k, _)| k == "code").collect();
        if states.len() != 1
            || states[0].1 != self.state
            || codes.len() != 1
            || codes[0].1.is_empty()
            || codes[0].1.len() > 4096
            || pairs.iter().any(|(k, _)| k == "error")
        {
            return Err("Corporate login failed or callback state mismatch".into());
        }
        Ok(codes[0].1.to_string())
    }
    /// Consume this attempt so the same verifier is not accidentally reused.
    pub async fn exchange(
        self,
        config: &EnterpriseLoginConfig,
        callback: &url::Url,
    ) -> Result<EnterpriseOAuthTokens, String> {
        let code = self.callback_code(callback)?;
        token_request(config, serde_json::json!({"grant_type":"authorization_code", "client_id":config.client_id, "code":code, "redirect_uri":self.redirect_uri, "code_verifier":self.verifier.as_str()})).await
    }
}

/// Token response. No Debug; Serialize is only for the application's encrypted credential store.
#[derive(Serialize, Deserialize)]
pub struct EnterpriseOAuthTokens {
    /// Short-lived dedicated API access token.
    pub access_token: String,
    /// Rotating refresh credential; never log or place in a URL.
    pub refresh_token: Option<String>,
    /// Access token lifetime in seconds.
    pub expires_in: u64,
    /// OAuth token type, required to be Bearer.
    pub token_type: String,
}
impl Drop for EnterpriseOAuthTokens {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.access_token.zeroize();
        if let Some(token) = self.refresh_token.as_mut() {
            token.zeroize();
        }
    }
}
/// Refresh once. On an ambiguous failure the owner must require login rather than retry a consumed rotating token.
pub async fn refresh(
    config: &EnterpriseLoginConfig,
    refresh_token: &str,
) -> Result<EnterpriseOAuthTokens, String> {
    token_request(config, serde_json::json!({"grant_type":"refresh_token", "client_id":config.client_id, "refresh_token":refresh_token})).await
}
async fn token_request(
    config: &EnterpriseLoginConfig,
    body: serde_json::Value,
) -> Result<EnterpriseOAuthTokens, String> {
    config.validate()?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "Cannot initialize corporate login")?;
    let mut response = client
        .post(format!(
            "{}/oauth/token",
            config.issuer.trim_end_matches('/')
        ))
        .header("Cache-Control", "no-store")
        .json(&body)
        .send()
        .await
        .map_err(|_| "Corporate token exchange failed; sign in again")?;
    if !response.status().is_success() {
        return Err("Corporate token exchange rejected; sign in again".into());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Corporate token response failed")?
    {
        if bytes.len() + chunk.len() > 64 * 1024 {
            return Err("Corporate token response too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let tokens: EnterpriseOAuthTokens =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid corporate token response")?;
    if tokens.access_token.is_empty()
        || tokens.access_token.len() > 16 * 1024
        || tokens.expires_in == 0
        || tokens.expires_in > 300
        || !tokens.token_type.eq_ignore_ascii_case("bearer")
        || tokens
            .refresh_token
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 16 * 1024)
    {
        return Err("Invalid corporate token lifetime or type".into());
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> EnterpriseLoginConfig {
        EnterpriseLoginConfig {
            signer_url: "https://signer.example/api".into(),
            issuer: "https://login.example/".into(),
            client_id: "native".into(),
            audience: "signer".into(),
            organization: "org".into(),
            connection: "okta".into(),
        }
    }
    #[test]
    fn pkce_login_pins_authority_and_checks_callback_state() {
        let attempt = EnterpriseLoginAttempt::new(
            [1; 32],
            [2; 32],
            "http://127.0.0.1:1234/enterprise-callback".into(),
        );
        let auth = attempt.authorization_url(&config()).unwrap();
        let params: std::collections::HashMap<_, _> = auth.query_pairs().collect();
        assert_eq!(auth.origin().ascii_serialization(), "https://login.example");
        assert_eq!(params["code_challenge_method"], "S256");
        assert_eq!(params["organization"], "org");
        assert_eq!(params["connection"], "okta");
        assert_eq!(
            params["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(URL_SAFE_NO_PAD.encode([1; 32])))
        );
        let callback = url::Url::parse(&format!(
            "http://127.0.0.1:1234/enterprise-callback?code=authorized&state={}",
            params["state"]
        ))
        .unwrap();
        assert_eq!(attempt.callback_code(&callback).unwrap(), "authorized");
        for value in [
            callback.to_string() + "&state=duplicate",
            callback.to_string() + "&code=duplicate",
            callback.to_string() + "#fragment",
            callback.to_string().replace("1234", "4321"),
            callback
                .to_string()
                .replace(params["state"].as_ref(), "wrong"),
        ] {
            assert!(attempt
                .callback_code(&url::Url::parse(&value).unwrap())
                .is_err());
        }
    }
    #[test]
    fn config_refuses_insecure_or_credential_bearing_endpoints() {
        for value in [
            "http://login.example",
            "https://user:pass@login.example",
            "https://login.example?token=x",
            "https://login.example/path",
        ] {
            let mut config = config();
            config.issuer = value.into();
            assert!(config.validate().is_err());
        }
    }
}
