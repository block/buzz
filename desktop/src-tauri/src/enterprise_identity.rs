//! Corporate login owns refresh credentials and session lifetime, never event construction.
use buzz_ws_client_pkg::enterprise_callback::EnterpriseCallback;
use buzz_ws_client_pkg::enterprise_oauth::{
    EnterpriseLoginAttempt, EnterpriseLoginConfig, EnterpriseOAuthTokens,
};
use buzz_ws_client_pkg::event_signer::EventSigner;
use buzz_ws_client_pkg::remote_identity::{
    ManagedIdentity, RemoteAuthorization, RemoteCredentials, RemoteEventSigner,
    RemoteIdentityClient,
};
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

pub(crate) fn build_config() -> Result<Option<EnterpriseLoginConfig>, String> {
    option_env!("BUZZ_DESKTOP_BUILD_ENTERPRISE")
        .map(|raw| {
            let config: EnterpriseLoginConfig =
                serde_json::from_str(raw).map_err(|_| "Invalid corporate build configuration")?;
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
struct CorporateSession {
    identity: ManagedIdentity,
    config: EnterpriseLoginConfig,
    client: RemoteIdentityClient,
    tokens: AsyncMutex<StoredTokens>,
    cancelled: CancellationToken,
    io: Arc<dyn CorporateSessionIo>,
    media_reads: crate::enterprise_media_cache::MediaReadProofCache,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredTokens {
    config: EnterpriseLoginConfig,
    tokens: EnterpriseOAuthTokens,
    expires_at: u64,
    identity: ManagedIdentity,
    // A durable rotation tombstone prevents replay after an ambiguous exchange/crash.
    rotation_pending: bool,
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
fn persist(stored: &StoredTokens) -> Result<(), String> {
    let encoded = zeroize::Zeroizing::new(
        serde_json::to_string(stored).map_err(|_| "Cannot encode corporate credentials")?,
    );
    secret_store().store("session", &encoded)
}
// Narrow IO seam around the original storage/refresh lifecycle, shared by the
// production owner and fault-injection tests. No identity lookup or event API.
trait CorporateSessionIo: Send + Sync {
    fn persist(&self, stored: &StoredTokens) -> Result<(), String>;
    fn refresh<'a>(
        &'a self,
        config: &'a EnterpriseLoginConfig,
        refresh: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<EnterpriseOAuthTokens, String>> + Send + 'a>>;
}
struct NativeSessionIo;
impl CorporateSessionIo for NativeSessionIo {
    fn persist(&self, stored: &StoredTokens) -> Result<(), String> {
        persist(stored)
    }
    fn refresh<'a>(
        &'a self,
        config: &'a EnterpriseLoginConfig,
        refresh: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<EnterpriseOAuthTokens, String>> + Send + 'a>> {
        Box::pin(buzz_ws_client_pkg::enterprise_oauth::refresh(
            config, refresh,
        ))
    }
}
impl RemoteAuthorization for CorporateSession {
    fn check_active(&self) -> Result<(), String> {
        if self.cancelled.is_cancelled() {
            Err("Corporate identity changed; sign in again".into())
        } else {
            Ok(())
        }
    }
    fn credentials(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<RemoteCredentials, String>> + Send + '_>> {
        Box::pin(async move {
            self.check_active()?;
            let mut stored = self.tokens.lock().await;
            self.check_active()?;
            let result = async {
                if stored.rotation_pending {
                    return Err("Corporate rotation interrupted; sign in again".into());
                }
                if stored.expires_at <= now()? + 30 {
                    stored.rotation_pending = true;
                    self.io.persist(&stored)?;
                    let refresh = stored
                        .tokens
                        .refresh_token
                        .as_ref()
                        .ok_or("Corporate session expired; sign in again")?;
                    let fresh = self.io.refresh(&self.config, refresh).await?;
                    self.check_active()?;
                    // Refresh is authorization-only. Do not re-enroll or silently switch identity.
                    // The sign response must match the pinned public key and exact event.
                    let next = StoredTokens {
                        config: self.config.clone(),
                        expires_at: now()? + fresh.expires_in,
                        tokens: fresh,
                        identity: self.identity.clone(),
                        rotation_pending: false,
                    };
                    self.io.persist(&next)?;
                    *stored = next;
                }
                self.check_active()?;
                Ok(RemoteCredentials::new(stored.tokens.access_token.clone()))
            }
            .await;
            if result.is_err() {
                self.cancelled.cancel();
            }
            result
        })
    }
}
impl EnterpriseIdentity {
    fn session(&self) -> Result<Arc<CorporateSession>, String> {
        let session = self
            .current
            .lock()
            .map_err(|_| "Corporate identity lock failed")?
            .clone()
            .ok_or("Corporate login required")?;
        session.check_active()?;
        Ok(session)
    }
    pub(crate) fn managed_identity(&self) -> Result<ManagedIdentity, String> {
        Ok(self.session()?.identity.clone())
    }
    pub(crate) fn event_signer(&self) -> Result<Arc<dyn EventSigner>, String> {
        let session = self.session()?;
        Ok(Arc::new(RemoteEventSigner::new(
            session.client.clone(),
            &session.identity,
            session.clone(),
        )?))
    }
    pub(crate) async fn media_read_proof(&self, base_url: &str) -> Result<String, String> {
        let session = self.session()?;
        if session.identity.relay_http_url.trim_end_matches('/') != base_url.trim_end_matches('/') {
            return Err("Corporate media host does not match login".into());
        }
        let signer =
            RemoteEventSigner::new(session.client.clone(), &session.identity, session.clone())?;
        session
            .media_reads
            .get(
                &signer,
                session.identity.relay_http_url.trim_end_matches('/'),
                &session.cancelled,
            )
            .await
    }
    pub(crate) fn cancellation(&self) -> Result<CancellationToken, String> {
        Ok(self.session()?.cancelled.clone())
    }
    async fn invalidate(&self) -> Result<(), String> {
        let old = self
            .current
            .lock()
            .map_err(|_| "Corporate identity lock failed")?
            .clone();
        if let Some(old) = old {
            old.cancelled.cancel();
            old.media_reads.clear().await;
            // Wait for an in-flight durable refresh before deleting/replacing its entry.
            let _tokens = old.tokens.lock().await;
        }
        Ok(())
    }
    async fn install(
        &self,
        config: EnterpriseLoginConfig,
        stored: StoredTokens,
    ) -> Result<ManagedIdentity, String> {
        if serde_json::to_value(&stored.config).map_err(|_| "Invalid saved configuration")?
            != serde_json::to_value(&config).map_err(|_| "Invalid build configuration")?
            || stored.rotation_pending
            || stored.expires_at > now()? + 300
        {
            return Err("Corporate build or session changed; sign in again".into());
        }
        stored.identity.validate()?;
        let session = Arc::new(CorporateSession {
            identity: stored.identity.clone(),
            client: RemoteIdentityClient::new(&config.signer_url)?,
            config,
            tokens: AsyncMutex::new(stored),
            cancelled: CancellationToken::new(),
            io: Arc::new(NativeSessionIo),
            media_reads: Default::default(),
        });
        session.credentials().await?;
        let identity = session.identity.clone();
        let mut current = self
            .current
            .lock()
            .map_err(|_| "Corporate identity lock failed")?;
        if let Some(old) = current.replace(session) {
            old.cancelled.cancel();
        }
        Ok(identity)
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnterpriseStatus {
    enabled: bool,
    identity: Option<ManagedIdentity>,
}
#[tauri::command]
pub(crate) async fn enterprise_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<EnterpriseStatus, String> {
    let Some(config) = build_config()? else {
        return Ok(EnterpriseStatus {
            enabled: false,
            identity: None,
        });
    };
    let _login = state.enterprise.login.lock().await;
    // Never restore a cancelled in-memory login from its old durable credentials.
    let current = state
        .enterprise
        .current
        .lock()
        .map_err(|_| "Corporate identity lock failed")?
        .clone();
    if let Some(session) = current {
        let identity = if session.credentials().await.is_ok() {
            Some(session.identity.clone())
        } else {
            None
        };
        return Ok(EnterpriseStatus {
            enabled: true,
            identity,
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
    app.state::<crate::native_relay_client::NativeRelayClient>()
        .bind_identity(state.enterprise.cancellation()?)
        .await;
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
) -> Result<ManagedIdentity, String> {
    let config = build_config()?.ok_or("Not a corporate build")?;
    let _login = state.enterprise.login.lock().await;
    state.enterprise.invalidate().await?;
    app.state::<crate::native_relay_client::NativeRelayClient>()
        .clear_identity()
        .await;
    secret_store().delete("session")?;
    let callback = EnterpriseCallback::bind(&config).await?;
    let mut verifier = [0; 32];
    let mut nonce = [0; 32];
    getrandom::getrandom(&mut verifier).map_err(|_| "Secure random unavailable")?;
    getrandom::getrandom(&mut nonce).map_err(|_| "Secure random unavailable")?;
    let attempt = EnterpriseLoginAttempt::new(verifier, nonce, config.redirect_uri.clone());
    app.opener()
        .open_url(attempt.authorization_url(&config)?.as_str(), None::<&str>)
        .map_err(|_| "Cannot open corporate login")?;
    let callback = callback.receive(&attempt).await?;
    let tokens = attempt.exchange(&config, &callback).await?;
    let identity = RemoteIdentityClient::new(&config.signer_url)?
        .ensure(&RemoteCredentials::new(tokens.access_token.clone()))
        .await?;
    let stored = StoredTokens {
        config: config.clone(),
        expires_at: now()? + tokens.expires_in,
        tokens,
        identity,
        rotation_pending: false,
    };
    persist(&stored)?;
    let identity = state.enterprise.install(config, stored).await?;
    app.state::<crate::native_relay_client::NativeRelayClient>()
        .bind_identity(state.enterprise.cancellation()?)
        .await;
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
        return Err("Not a corporate build".into());
    }
    let _login = state.enterprise.login.lock().await;
    state.enterprise.invalidate().await?;
    app.state::<crate::native_relay_client::NativeRelayClient>()
        .clear_identity()
        .await;
    secret_store().delete("session")
}

#[cfg(test)]
#[path = "enterprise_identity_tests.rs"]
mod tests;
