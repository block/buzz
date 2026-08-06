use std::{net::IpAddr, sync::Arc, time::Duration};

use futures_util::StreamExt;
use nostr::PublicKey;
use serde::Deserialize;
use tauri::ipc::Channel;
use url::{Host, Url};

use buzz_core_pkg::client_binding_bootstrap::ClientBindingEpoch;

use crate::{
    app_state::AppState,
    client_binding_status_session::{ClientBindingStatusSession, CurrentProjection},
};

use super::{ConnectionHandle, Id};

const NIP11_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_NIP11_BODY_BYTES: usize = 64 * 1024;

pub(super) struct StatusScope {
    pub(super) relay_url: String,
    pub(super) relay_signer: PublicKey,
    pub(super) expected_author: PublicKey,
    pub(super) epoch: ClientBindingEpoch,
    pub(super) projection_channel: Channel<serde_json::Value>,
    pub(super) generation: u64,
    pub(super) attempt: u64,
    pub(super) challenge: Option<String>,
    pub(super) auth_proven: bool,
}

pub(super) struct PreparedStatus {
    pub(super) session: ClientBindingStatusSession,
    pub(super) scope: StatusScope,
}

pub(crate) struct StatusAuthProof {
    pub(super) handle: Arc<ConnectionHandle>,
    pub(super) challenge: String,
    pub(super) relay_url: String,
    pub(super) relay_signer: PublicKey,
    pub(super) expected_author: PublicKey,
    pub(super) epoch: ClientBindingEpoch,
    pub(super) generation: u64,
    pub(super) attempt: u64,
}

impl StatusAuthProof {
    pub(crate) fn connection_epoch(&self) -> &ClientBindingEpoch {
        &self.epoch
    }

    pub(crate) const fn relay_signer(&self) -> PublicKey {
        self.relay_signer
    }
}

pub(super) struct ProjectionOwner {
    pub(super) id: Id,
    pub(super) handle: Arc<ConnectionHandle>,
    pub(super) epoch: ClientBindingEpoch,
    pub(super) attempt: u64,
    pub(super) presentation_token: u64,
    pub(super) channel: Channel<serde_json::Value>,
}

#[derive(Default)]
pub(super) struct ProjectionState {
    pub(super) generation: u64,
    pub(super) attempt_head: u64,
    pub(super) mutation_depth: u64,
    pub(super) suspended: bool,
    pub(super) owner: Option<ProjectionOwner>,
    pub(super) current: Option<CurrentProjection>,
}

pub(super) async fn prepare_status_session(
    state: &AppState,
    requested_url: &str,
    projection_channel: Channel<serde_json::Value>,
    generation: u64,
    attempt: u64,
) -> Option<PreparedStatus> {
    if requested_url != crate::relay::relay_ws_url_with_override(state) {
        return None;
    }
    let expected_author = state.signing_keys().ok()?.public_key();
    let relay_signer = fetch_nip11_signer(requested_url).await.ok()?;
    let epoch = ClientBindingEpoch::new_v4();
    Some(PreparedStatus {
        session: ClientBindingStatusSession::new(relay_signer, expected_author, epoch.clone()),
        scope: StatusScope {
            relay_url: requested_url.to_owned(),
            relay_signer,
            expected_author,
            epoch,
            projection_channel,
            generation,
            attempt,
            challenge: None,
            auth_proven: false,
        },
    })
}

#[derive(Deserialize)]
struct Nip11Identity {
    #[serde(rename = "self")]
    relay_self: String,
}

async fn fetch_nip11_signer(relay_url: &str) -> Result<PublicKey, String> {
    let url = nip11_url(relay_url)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(NIP11_TIMEOUT)
        .build()
        .map_err(|_| "NIP-11 unavailable".to_string())?;
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/nostr+json")
        .send()
        .await
        .map_err(|_| "NIP-11 unavailable".to_string())?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > MAX_NIP11_BODY_BYTES as u64)
    {
        return Err("NIP-11 unavailable".to_string());
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "NIP-11 unavailable".to_string())?;
        if body.len().saturating_add(chunk.len()) > MAX_NIP11_BODY_BYTES {
            return Err("NIP-11 unavailable".to_string());
        }
        body.extend_from_slice(&chunk);
    }
    let identity: Nip11Identity =
        serde_json::from_slice(&body).map_err(|_| "NIP-11 unavailable".to_string())?;
    if identity.relay_self.len() != 64
        || !identity
            .relay_self
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("NIP-11 unavailable".to_string());
    }
    PublicKey::from_hex(&identity.relay_self).map_err(|_| "NIP-11 unavailable".to_string())
}

pub(super) fn nip11_url(relay_url: &str) -> Result<Url, String> {
    let mut url = Url::parse(relay_url).map_err(|_| "NIP-11 unavailable".to_string())?;
    match url.scheme() {
        "wss" => url
            .set_scheme("https")
            .map_err(|_| "NIP-11 unavailable".to_string())?,
        "ws" if is_loopback_url(&url) => url
            .set_scheme("http")
            .map_err(|_| "NIP-11 unavailable".to_string())?,
        _ => return Err("NIP-11 unavailable".to_string()),
    }
    Ok(url)
}

pub(super) fn is_loopback_url(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(address)) => IpAddr::V4(address).is_loopback(),
        Some(Host::Ipv6(address)) => IpAddr::V6(address).is_loopback(),
        None => false,
    }
}

pub(super) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

pub(super) fn duration_until_unix_second(unix_second: u64) -> Duration {
    let Some(deadline) = std::time::UNIX_EPOCH.checked_add(Duration::from_secs(unix_second)) else {
        return Duration::ZERO;
    };
    deadline
        .duration_since(std::time::SystemTime::now())
        .unwrap_or_default()
}

pub(super) fn monotonic_deadline_after(delay: Duration) -> tokio::time::Instant {
    let now = tokio::time::Instant::now();
    now.checked_add(delay).unwrap_or(now)
}

pub(super) fn status_expiry_sleep(deadline: tokio::time::Instant) -> tokio::time::Sleep {
    tokio::time::sleep_until(deadline)
}
