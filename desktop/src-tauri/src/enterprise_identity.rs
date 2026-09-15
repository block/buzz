//! Build-selected corporate identity. Local keys are never a fallback in enterprise builds.
use base64::{engine::general_purpose::STANDARD, Engine};
use buzz_ws_client_pkg::enterprise::{
    EnterpriseCredentials, EnterpriseSession, EnterpriseSigner, EnterpriseTemplate,
};
use buzz_ws_client_pkg::enterprise_oauth::{
    EnterpriseLoginAttempt, EnterpriseLoginConfig, EnterpriseOAuthTokens,
};
use nostr::{Event, EventBuilder, JsonUtil, Keys, PublicKey};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Mutex as AsyncMutex;

pub(crate) fn build_config() -> Result<Option<EnterpriseLoginConfig>, String> {
    option_env!("BUZZ_DESKTOP_BUILD_ENTERPRISE")
        .map(|raw| {
            let config: EnterpriseLoginConfig =
                serde_json::from_str(raw).map_err(|_| "Invalid enterprise build configuration")?;
            config.validate()?;
            Ok(config)
        })
        .transpose()
}
pub(crate) fn enabled() -> bool {
    option_env!("BUZZ_DESKTOP_BUILD_ENTERPRISE").is_some()
}

#[derive(Default)]
pub(crate) struct EnterpriseIdentity {
    current: Mutex<Option<Arc<CorporateSession>>>,
    login: AsyncMutex<()>,
}
pub(crate) struct CorporateSession {
    public_key: PublicKey,
    media: AsyncMutex<Option<(String, u64)>>,
    config: EnterpriseLoginConfig,
    signer: EnterpriseSigner,
    tokens: AsyncMutex<StoredTokens>,
    active: std::sync::atomic::AtomicBool,
}
#[derive(Serialize, Deserialize)]
struct StoredTokens {
    config: EnterpriseLoginConfig,
    tokens: EnterpriseOAuthTokens,
    expires_at: u64,
    identity: EnterpriseSession,
}
fn now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| "Invalid system clock".into())
}
fn secret_store() -> &'static crate::secret_store::SecretStore {
    static STORE: std::sync::OnceLock<crate::secret_store::SecretStore> =
        std::sync::OnceLock::new();
    STORE.get_or_init(|| {
        crate::secret_store::SecretStore::keyring(format!(
            "{}-enterprise",
            crate::build_identity::keyring_service()
        ))
    })
}

/// A snapshot binds async work to the same identity even when login changes.
#[derive(Clone)]
pub(crate) enum SigningIdentity {
    Local(Keys),
    Corporate(Arc<CorporateSession>),
}
impl From<Keys> for SigningIdentity {
    fn from(keys: Keys) -> Self {
        Self::Local(keys)
    }
}
impl From<&Keys> for SigningIdentity {
    fn from(keys: &Keys) -> Self {
        Self::Local(keys.clone())
    }
}
impl From<&SigningIdentity> for SigningIdentity {
    fn from(identity: &SigningIdentity) -> Self {
        identity.clone()
    }
}
impl SigningIdentity {
    pub(crate) fn public_key(&self) -> PublicKey {
        match self {
            Self::Local(keys) => keys.public_key(),
            Self::Corporate(session) => session.signer_public_key(),
        }
    }
    pub(crate) async fn sign(&self, builder: EventBuilder) -> Result<Event, String> {
        match self {
            Self::Local(keys) => builder.sign_with_keys(keys).map_err(|e| e.to_string()),
            Self::Corporate(session) => {
                let unsigned = builder.build(self.public_key());
                let template = EnterpriseTemplate {
                    kind: unsigned.kind.as_u16(),
                    created_at: unsigned.created_at.as_secs(),
                    tags: unsigned
                        .tags
                        .iter()
                        .map(|t| t.as_slice().to_vec())
                        .collect(),
                    content: unsigned.content,
                };
                session.sign(&template).await
            }
        }
    }
    pub(crate) async fn nip98(
        &self,
        method: &reqwest::Method,
        url: &str,
        body: &[u8],
    ) -> Result<String, String> {
        use sha2::{Digest, Sha256};
        if let Self::Corporate(session) = self {
            let target = url::Url::parse(url).map_err(|_| "Invalid request URL")?;
            let relay = url::Url::parse(&session.signer.identity().relay_http_url)
                .map_err(|_| "Invalid relay URL")?;
            if target.origin() != relay.origin()
                || !target.username().is_empty()
                || target.password().is_some()
            {
                return Err("Corporate request outside configured community".into());
            }
        }
        let tags = vec![
            vec!["u".to_owned(), url.to_owned()],
            vec!["method".into(), method.to_string()],
            vec!["payload".into(), hex::encode(Sha256::digest(body))],
            vec!["nonce".into(), uuid::Uuid::new_v4().to_string()],
        ];
        let tags = tags
            .into_iter()
            .map(nostr::Tag::parse)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let event = self
            .sign(EventBuilder::new(nostr::Kind::HttpAuth, "").tags(tags))
            .await?;
        Ok(format!("Nostr {}", STANDARD.encode(event.as_json())))
    }
    pub(crate) async fn media_read(&self, base: &str) -> Result<String, String> {
        let Self::Corporate(session) = self else {
            return Err("Expected corporate signer".into());
        };
        session.check_active()?;
        if base.trim_end_matches('/')
            != session
                .signer
                .identity()
                .relay_http_url
                .trim_end_matches('/')
        {
            return Err("Enterprise media scope mismatch".into());
        }
        let mut cached = session.media.lock().await;
        if let Some((header, expires)) = cached.as_ref() {
            if *expires > now()? + 15 {
                return Ok(header.clone());
            }
        }
        let expires = now()? + 120;
        let server = url::Url::parse(base).map_err(|_| "Invalid media origin")?;
        let authority = server[url::Position::BeforeHost..url::Position::AfterPort].to_owned();
        let event = session
            .sign(&EnterpriseTemplate {
                kind: 24242,
                created_at: now()?,
                content: "Get buzz-media".into(),
                tags: vec![
                    vec!["t".into(), "get".into()],
                    vec!["server".into(), authority],
                    vec!["expiration".into(), expires.to_string()],
                ],
            })
            .await?;
        session.check_active()?;
        let header = format!("Nostr {}", STANDARD.encode(event.as_json()));
        *cached = Some((header.clone(), expires));
        Ok(header)
    }

    pub(crate) async fn connect(
        &self,
        relay: &str,
        auth: Option<&nostr::Tag>,
    ) -> Result<buzz_ws_client_pkg::NostrWsConnection, String> {
        match self {
            Self::Local(keys) => {
                buzz_ws_client_pkg::NostrWsConnection::connect_authenticated(relay, keys, auth)
                    .await
                    .map_err(|e| e.to_string())
            }
            Self::Corporate(session) => {
                if auth.is_some() {
                    return Err("Agent delegation is unavailable in enterprise mode".into());
                }
                let credentials = session.credentials().await?;
                let mut connection = buzz_ws_client_pkg::NostrWsConnection::connect(relay)
                    .await
                    .map_err(|e| e.to_string())?;
                connection
                    .authenticate_enterprise(&session.signer, &credentials)
                    .await
                    .map_err(|e| e.to_string())?;
                session.check_active()?;
                Ok(connection)
            }
        }
    }
}
impl CorporateSession {
    fn signer_public_key(&self) -> PublicKey {
        // Validated at construction; retain an infallible public-key accessor without exposing key material.
        self.pubkey()
    }
    fn pubkey(&self) -> PublicKey {
        self.public_key
    }
    fn check_active(&self) -> Result<(), String> {
        if self.active.load(std::sync::atomic::Ordering::Acquire) {
            Ok(())
        } else {
            Err("Corporate identity changed; sign in again".into())
        }
    }
    async fn credentials(&self) -> Result<EnterpriseCredentials, String> {
        self.check_active()?;
        let mut stored = self.tokens.lock().await;
        self.check_active()?;
        if stored.expires_at <= now()? + 30 {
            let refresh = stored
                .tokens
                .refresh_token
                .as_ref()
                .ok_or("Corporate session expired; sign in again")?;
            // A rotating token may be consumed even when the response is lost. Remove the old durable
            // credential before exchange so a restart cannot replay it; failure requires browser login.
            secret_store().delete("session")?;
            let result = buzz_ws_client_pkg::enterprise_oauth::refresh(&self.config, refresh).await;
            let fresh = match result {
                Ok(tokens) => tokens,
                Err(error) => {
                    self.active
                        .store(false, std::sync::atomic::Ordering::Release);
                    return Err(error);
                }
            };
            let credentials = EnterpriseCredentials::new(fresh.access_token.clone());
            if self.signer.session(&credentials).await.is_err() {
                self.active
                    .store(false, std::sync::atomic::Ordering::Release);
                return Err("Corporate account changed or access denied; sign in again".into());
            }
            let next = StoredTokens {
                config: self.config.clone(),
                expires_at: now()? + fresh.expires_in,
                tokens: fresh,
                identity: stored.identity.clone(),
            };
            // Rotation is one atomic credential-store write; no success if rotated credentials cannot be saved.
            let encoded = zeroize::Zeroizing::new(
                serde_json::to_string(&next).map_err(|_| "Cannot encode corporate credentials")?,
            );
            if let Err(error) = secret_store().store("session", &encoded) {
                self.active
                    .store(false, std::sync::atomic::Ordering::Release);
                return Err(error);
            }
            *stored = next;
        }
        self.check_active()?;
        Ok(EnterpriseCredentials::new(
            stored.tokens.access_token.clone(),
        ))
    }
    async fn sign(&self, template: &EnterpriseTemplate) -> Result<Event, String> {
        let credentials = self.credentials().await?;
        let event = self
            .signer
            .sign(template, &credentials)
            .await
            .map_err(|e| e.to_string())?;
        self.check_active()?;
        Ok(event)
    }
}
impl EnterpriseIdentity {
    pub(crate) fn identity(&self) -> Result<SigningIdentity, String> {
        self.current
            .lock()
            .map_err(|_| "Corporate identity lock failed")?
            .as_ref()
            .cloned()
            .map(SigningIdentity::Corporate)
            .ok_or_else(|| "Corporate login required".into())
    }
    async fn install(
        &self,
        config: EnterpriseLoginConfig,
        stored: StoredTokens,
    ) -> Result<EnterpriseSession, String> {
        if serde_json::to_value(&stored.config).map_err(|_| "Invalid stored configuration")?
            != serde_json::to_value(&config).map_err(|_| "Invalid build configuration")?
        {
            return Err("Enterprise build changed; sign in again".into());
        }
        let public_key = PublicKey::from_hex(&stored.identity.pubkey)
            .map_err(|_| "Invalid corporate identity")?;
        let signer = EnterpriseSigner::new(&config.signer_url, stored.identity.clone())
            .map_err(|e| e.to_string())?;
        let session = Arc::new(CorporateSession {
            config,
            signer,
            public_key,
            media: AsyncMutex::new(None),
            tokens: AsyncMutex::new(stored),
            active: std::sync::atomic::AtomicBool::new(true),
        });
        session.credentials().await?;
        let identity = session.signer.identity().clone();
        let mut current = self
            .current
            .lock()
            .map_err(|_| "Corporate identity lock failed")?;
        if let Some(old) = current.replace(session) {
            old.active
                .store(false, std::sync::atomic::Ordering::Release);
        }
        Ok(identity)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnterpriseStatus {
    enabled: bool,
    identity: Option<EnterpriseSession>,
}
#[tauri::command]
pub(crate) async fn enterprise_status(
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<EnterpriseStatus, String> {
    let Some(config) = build_config()? else {
        return Ok(EnterpriseStatus {
            enabled: false,
            identity: None,
        });
    };
    let _lock = state.enterprise.login.lock().await;
    if let Ok(SigningIdentity::Corporate(session)) = state.enterprise.identity() {
        if session.credentials().await.is_err() {
            return Ok(EnterpriseStatus {
                enabled: true,
                identity: None,
            });
        }
        return Ok(EnterpriseStatus {
            enabled: true,
            identity: Some(session.signer.identity().clone()),
        });
    }
    let Some(encoded) = secret_store().load("session")? else {
        return Ok(EnterpriseStatus {
            enabled: true,
            identity: None,
        });
    };
    let encoded = zeroize::Zeroizing::new(encoded);
    let stored = serde_json::from_str(&encoded).map_err(|_| "Cannot restore corporate login")?;
    let identity = state.enterprise.install(config, stored).await?;
    *state
        .relay_url_override
        .lock()
        .map_err(|_| "Relay lock failed")? = Some(identity.relay_ws_url.clone());
    Ok(EnterpriseStatus {
        enabled: true,
        identity: Some(identity),
    })
}
#[tauri::command]
pub(crate) async fn enterprise_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<EnterpriseSession, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let config = build_config()?.ok_or("Not an enterprise build")?;
    app.state::<crate::native_relay_client::NativeRelayClient>()
        .clear_identity()
        .await;
    let _lock = state.enterprise.login.lock().await;
    let previous = state
        .enterprise
        .current
        .lock()
        .map_err(|_| "Corporate identity lock failed")?
        .clone();
    if let Some(previous) = previous {
        let _tokens = previous.tokens.lock().await;
        previous
            .active
            .store(false, std::sync::atomic::Ordering::Release);
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| "Cannot open login callback")?;
    let port = listener
        .local_addr()
        .map_err(|_| "Cannot resolve login callback")?
        .port();
    let redirect = format!("http://127.0.0.1:{port}/enterprise-callback");
    let mut verifier = [0; 32];
    let mut nonce = [0; 32];
    getrandom::getrandom(&mut verifier).map_err(|_| "Secure random unavailable")?;
    getrandom::getrandom(&mut nonce).map_err(|_| "Secure random unavailable")?;
    let attempt = EnterpriseLoginAttempt::new(verifier, nonce, redirect);
    app.opener()
        .open_url(attempt.authorization_url(&config)?.as_str(), None::<&str>)
        .map_err(|_| "Cannot open corporate login")?;
    let callback = tokio::time::timeout(Duration::from_secs(300), async {
        for _ in 0..32 {
            let (mut stream, _) = listener.accept().await.map_err(|_| "Login callback failed")?;
            let request = tokio::time::timeout(Duration::from_secs(3), async {
                let mut bytes = Vec::new();
                let mut chunk = [0; 1024];
                while !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    let count = stream.read(&mut chunk).await.map_err(|_| "Callback read failed")?;
                    if count == 0 || bytes.len() + count > 8192 { return Err("Invalid callback size"); }
                    bytes.extend_from_slice(&chunk[..count]);
                }
                String::from_utf8(bytes).map_err(|_| "Invalid callback encoding")
            }).await;
            let Ok(Ok(request)) = request else { continue; };
            let Some(target) = request.lines().next().and_then(|l| l.strip_prefix("GET ")).and_then(|l| l.strip_suffix(" HTTP/1.1")) else { continue; };
            if !target.starts_with('/') || target.starts_with("//") { continue; }
            let Ok(url) = url::Url::parse(&format!("http://127.0.0.1:{port}{target}")) else { continue; };
            let valid = attempt.callback_code(&url).is_ok();
            let response = if valid { "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nConnection: close\r\n\r\nReturn to Buzz to finish signing in." } else { "HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\nInvalid login callback." };
            let _ = tokio::time::timeout(Duration::from_secs(3), stream.write_all(response.as_bytes())).await;
            if valid { return Ok::<_, String>(url); }
        }
        Err("Too many invalid login callbacks".into())
    }).await.map_err(|_| "Corporate login timed out")??;
    let tokens = attempt.exchange(&config, &callback).await?;
    let credentials = EnterpriseCredentials::new(tokens.access_token.clone());
    let signer = EnterpriseSigner::login(&config.signer_url, &credentials)
        .await
        .map_err(|e| e.to_string())?;
    let stored = StoredTokens {
        config: config.clone(),
        expires_at: now()? + tokens.expires_in,
        tokens,
        identity: signer.identity().clone(),
    };
    let encoded = zeroize::Zeroizing::new(
        serde_json::to_string(&stored).map_err(|_| "Cannot encode credentials")?,
    );
    secret_store().store("session", &encoded)?;
    let identity = state.enterprise.install(config, stored).await?;
    *state
        .relay_url_override
        .lock()
        .map_err(|_| "Relay lock failed")? = Some(identity.relay_ws_url.clone());
    let _ = app.emit("enterprise-identity-changed", ());
    Ok(identity)
}

#[tauri::command]
pub(crate) async fn enterprise_logout(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<(), String> {
    if !enabled() {
        return Err("Not an enterprise build".into());
    }
    let _login = state.enterprise.login.lock().await;
    let session = state
        .enterprise
        .current
        .lock()
        .map_err(|_| "Corporate identity lock failed")?
        .take();
    if let Some(session) = session {
        let _tokens = session.tokens.lock().await;
        session
            .active
            .store(false, std::sync::atomic::Ordering::Release);
    }
    app.state::<crate::native_relay_client::NativeRelayClient>()
        .clear_identity()
        .await;
    secret_store().delete("session")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_session_never_provides_a_local_fallback() {
        let identity = EnterpriseIdentity::default();
        assert!(identity.identity().is_err());
    }
    #[tokio::test]
    async fn local_signing_snapshot_preserves_captured_identity_and_template() {
        let keys = Keys::generate();
        let signer = SigningIdentity::from(keys.clone());
        let event = signer
            .sign(
                EventBuilder::new(nostr::Kind::from(9), "message")
                    .custom_created_at(nostr::Timestamp::from(1000)),
            )
            .await
            .unwrap();
        assert_eq!(event.pubkey, keys.public_key());
        assert_eq!(event.created_at.as_secs(), 1000);
        assert!(event.verify().is_ok());
    }
    #[test]
    #[ignore = "run with BUZZ_BUILD_ENTERPRISE set to validate the compiled enterprise guard"]
    fn compiled_enterprise_build_rejects_local_key_access_and_import() {
        assert!(enabled());
        assert!(build_config().unwrap().is_some());
        let state = crate::app_state::build_app_state();
        assert!(state.signing_keys().is_err());
        assert!(state.signing_identity().is_err());
        let mut called = false;
        let result = crate::commands::commit_imported_identity(
            &state,
            std::path::Path::new("/unused"),
            Keys::generate(),
            |_| {
                called = true;
                Ok(crate::identity_storage::IdentityStorage::Ephemeral)
            },
        );
        assert!(result.is_err());
        assert!(!called);
    }
}
