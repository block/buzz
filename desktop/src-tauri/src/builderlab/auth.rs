//! Bounded browser code exchange, current-session verification and identity discovery.
use super::*;
use crate::remote_signer::{
    bounded_body, bounded_client, transport_error, RemoteSigner, RemoteSignerError,
    RemoteSignerSession,
};
use chrono::{DateTime, Utc};
use nostr::PublicKey;
use reqwest::header::HeaderValue;
use session::{LoginAttempt, StoredSession};

pub(super) fn auth_base(remote: bool) -> Result<Url, String> {
    if remote {
        match crate::remote_signer::build_signer_config().map_err(str::to_owned)? {
            crate::signer_config::SignerConfig::Remote { api_base } => Ok(api_base),
            _ => Err("remote signer configuration unavailable".into()),
        }
    } else {
        api_url("")
    }
}
fn endpoint(base: &Url, path: &str) -> Url {
    let mut url = base.clone();
    url.set_path(&format!("{}{}", base.path().trim_end_matches('/'), path));
    url
}
pub(super) fn browser_url(base: &Url, return_to: &str) -> Url {
    let mut url = endpoint(base, "/v1/auth/login");
    url.query_pairs_mut()
        .append_pair("type", "cli")
        .append_pair("product", "buzz")
        .append_pair("returnTo", return_to);
    url
}
fn credential(value: &str) -> Result<HeaderValue, RemoteSignerError> {
    if value.is_empty() || value.trim() != value {
        return Err(RemoteSignerError::Configuration);
    }
    let mut header = HeaderValue::from_str(value).map_err(|_| RemoteSignerError::Configuration)?;
    header.set_sensitive(true);
    Ok(header)
}
async fn json<T: serde::de::DeserializeOwned>(
    request: reqwest::RequestBuilder,
) -> Result<T, RemoteSignerError> {
    let response = request.send().await.map_err(transport_error)?;
    let bytes = bounded_body(response, 16 * 1024).await?;
    serde_json::from_slice(&bytes).map_err(|_| RemoteSignerError::MalformedResponse)
}
fn expiry(value: &str) -> Result<DateTime<Utc>, String> {
    let expires = DateTime::parse_from_rfc3339(value)
        .map_err(|_| "invalid authentication expiry")?
        .with_timezone(&Utc);
    if expires <= Utc::now() {
        return Err("authentication expired".into());
    }
    Ok(expires)
}
pub(super) async fn me(base: &Url, value: &str) -> Result<AuthMeResponse, RemoteSignerError> {
    json(
        bounded_client(base)?
            .get(endpoint(base, "/v1/auth/me"))
            .header(BB_SESSION_CREDENTIAL_HEADER, credential(value)?),
    )
    .await
}

/// The production exchange seam, also driven by a real loopback API in tests.
pub(super) async fn finish_login(
    attempt: LoginAttempt,
    base: &Url,
    code: String,
    remote: bool,
) -> Result<BuilderlabAuthInfo, String> {
    let client = bounded_client(base).map_err(|e| e.to_string())?;
    let exchanged: LoginExchangeResponse = attempt
        .run(async {
            json(
                client
                    .post(endpoint(base, "/v1/auth/login/exchange"))
                    .json(&serde_json::json!({"code":code})),
            )
            .await
            .map_err(|e| e.to_string())
        })
        .await?;
    credential(&exchanged.session_credential).map_err(|e| e.to_string())?;
    let expires = expiry(&exchanged.expires_at)?;
    let current = attempt
        .run(async {
            me(base, &exchanged.session_credential)
                .await
                .map_err(|e| e.to_string())
        })
        .await?;
    if expiry(&current.expires_at)? != expires {
        return Err("Builderlab session expiry did not match code exchange".into());
    }
    let mut validity = attempt.validity.clone();
    validity.expires = expires;
    let signer = if remote {
        #[derive(Deserialize)]
        struct Identity {
            pubkey: String,
        }
        let identity: Identity = attempt
            .run(async {
                json(
                    client
                        .post(endpoint(base, "/v1/buzz/identity"))
                        .header(
                            BB_SESSION_CREDENTIAL_HEADER,
                            credential(&exchanged.session_credential).map_err(|e| e.to_string())?,
                        )
                        .json(&serde_json::json!({})),
                )
                .await
                .map_err(|e| e.to_string())
            })
            .await?;
        if identity.pubkey.len() != 64
            || !identity
                .pubkey
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid remote identity public key".into());
        }
        let pubkey = PublicKey::from_hex(&identity.pubkey)
            .map_err(|_| "invalid remote identity public key")?;
        pubkey
            .xonly()
            .map_err(|_| "invalid remote identity public key")?;
        let binding = RemoteSignerSession::new(&exchanged.session_credential, pubkey)
            .map_err(|e| e.to_string())?;
        let remote = RemoteSigner::new(base.as_str(), binding)
            .map_err(|e| e.to_string())?
            .with_validity(validity.clone());
        let remote = std::sync::Arc::new(remote);
        Some(
            crate::active_user_signer::ActiveUserSigner::new(remote.clone())
                .await?
                .with_agent_capabilities(remote)?
                .with_lifetime(std::sync::Arc::new(validity.clone())),
        )
    } else {
        None
    };
    attempt.commit(StoredSession {
        credential: exchanged.session_credential,
        info: BuilderlabAuthInfo {
            expires_at: current.expires_at,
            email: current.email,
            name: current.name,
        },
        validity,
        signer,
    })
}

pub(super) async fn check_auth(
    owner: &BuilderlabSession,
    base: &Url,
) -> Result<Option<BuilderlabAuthInfo>, String> {
    let Some(stored) = owner.snapshot()? else {
        return Ok(None);
    };
    let result = tokio::select! { biased; _ = stored.validity.canceled() => return Err("native authentication expired".into()), result = me(base, &stored.credential) => result };
    owner.check(&stored.validity)?;
    match result {
        Ok(me) => {
            if expiry(&me.expires_at).ok() != Some(stored.validity.expires) {
                owner.clear_if_current(stored.validity.generation)?;
                return Err("authentication expiry changed".into());
            }
            Ok(Some(BuilderlabAuthInfo {
                expires_at: me.expires_at,
                email: me.email,
                name: me.name,
            }))
        }
        Err(error) => {
            if !matches!(
                error,
                RemoteSignerError::TransientStatus(_)
                    | RemoteSignerError::Transport
                    | RemoteSignerError::Timeout
            ) {
                owner.clear_if_current(stored.validity.generation)?;
            }
            Err(error.to_string())
        }
    }
}

/// Aborting a dropped IPC future cannot leave its listener running.
struct CallbackServer(tokio::task::JoinHandle<()>);
impl Drop for CallbackServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) async fn browser_code(
    attempt: &LoginAttempt,
    base: &Url,
    open: impl FnOnce(&str) -> Result<(), String>,
) -> Result<String, String> {
    let listener = attempt
        .run(async {
            TcpListener::bind("127.0.0.1:0")
                .await
                .map_err(|_| "could not start authentication callback".into())
        })
        .await?;
    let port = listener
        .local_addr()
        .map_err(|_| "could not read authentication callback address")?
        .port();
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let return_to = format!("http://127.0.0.1:{port}/callback/{nonce}");
    let (sender, receiver) = oneshot::channel();
    let router = Router::new()
        .route("/callback/{nonce}", get(login_callback))
        .with_state(std::sync::Arc::new(CallbackState {
            nonce,
            sender: Mutex::new(Some(sender)),
        }));
    let server = CallbackServer(tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    }));
    attempt.validity.check()?;
    open(browser_url(base, &return_to).as_str())?;
    let result = attempt
        .run(async {
            tokio::time::timeout(LOGIN_TIMEOUT, receiver)
                .await
                .map_err(|_| "authentication timed out")?
                .map_err(|_| "authentication callback stopped")?
        })
        .await;
    server.0.abort();
    // Join after abort on normal completion; Drop also covers future cancellation.
    let mut server = server;
    let _ = (&mut server.0).await;
    result
}
