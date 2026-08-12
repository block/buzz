use std::{collections::HashSet, net::IpAddr, sync::Arc, time::Duration};

use futures_util::StreamExt;
use nostr::PublicKey;
use serde::Deserialize;
use tauri::ipc::Channel;
use url::{Host, Url};

use buzz_core_pkg::{
    client_binding_bootstrap::ClientBindingEpoch,
    client_binding_status::MAX_CLIENT_BINDING_STATUS_LIFETIME_SECS,
    kind::{KIND_CLIENT_BINDING_BOOTSTRAP, KIND_CLIENT_BINDING_STATUS},
};

use crate::{
    app_state::AppState,
    client_binding_status_session::{
        ClientBindingStatusSession, CurrentProjection, ProjectionUpdate,
    },
};

use super::{ConnectionHandle, Id, WebSocketManager};

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
    pub(super) generation: u64,
    pub(super) attempt: u64,
    pub(super) presentation_token: u64,
    pub(super) channel: Channel<serde_json::Value>,
}

#[derive(Default)]
pub(super) struct ProjectionState {
    pub(super) generation: u64,
    pub(super) attempt_head: u64,
    pub(super) mutation_head: u64,
    pub(super) active_mutations: HashSet<u64>,
    pub(super) suspended: bool,
    pub(super) owner: Option<ProjectionOwner>,
    pub(super) current: Option<CurrentProjection>,
}

impl WebSocketManager {
    pub(super) fn advance_generation(projection: &mut ProjectionState) -> bool {
        let Some(generation) = projection.generation.checked_add(1) else {
            projection.suspended = true;
            if let Some(owner) = projection.owner.take() {
                let _ = owner.channel.send(serde_json::Value::Null);
            }
            projection.current = None;
            return false;
        };
        projection.generation = generation;
        true
    }

    #[cfg(test)]
    pub(super) async fn projection_generation(&self) -> u64 {
        self.projection.lock().await.generation
    }

    pub(super) async fn status_head(&self) -> Option<(u64, u64)> {
        let projection = self.projection.lock().await;
        (!projection.suspended && projection.active_mutations.is_empty())
            .then_some((projection.generation, projection.attempt_head))
    }

    pub(super) async fn begin_status_attempt(&self) -> Option<(u64, u64)> {
        let mut projection = self.projection.lock().await;
        if projection.suspended || !projection.active_mutations.is_empty() {
            return None;
        }
        let Some(next_attempt) = projection.attempt_head.checked_add(1) else {
            projection.suspended = true;
            if let Some(owner) = projection.owner.take() {
                let _ = owner.channel.send(serde_json::Value::Null);
            }
            projection.current = None;
            return None;
        };
        projection.attempt_head = next_attempt;
        Some((projection.generation, projection.attempt_head))
    }

    pub(super) async fn activate_projection_after_auth(
        &self,
        id: Id,
        proof: &StatusAuthProof,
    ) -> bool {
        if !self
            .connections
            .lock()
            .await
            .get(&id)
            .is_some_and(|current| Arc::ptr_eq(current, &proof.handle))
        {
            return false;
        }
        let mut projection = self.projection.lock().await;
        if projection.suspended
            || !projection.active_mutations.is_empty()
            || projection.generation != proof.generation
            || projection.attempt_head != proof.attempt
        {
            return false;
        }
        let mut status_scope = proof.handle.status_scope.lock().await;
        let Some(scope) = status_scope.as_mut() else {
            return false;
        };
        if scope.generation != proof.generation
            || scope.auth_proven
            || scope.challenge.as_deref() != Some(proof.challenge.as_str())
            || scope.relay_url != proof.relay_url
            || scope.relay_signer != proof.relay_signer
            || scope.expected_author != proof.expected_author
            || scope.epoch != proof.epoch
            || scope.attempt != proof.attempt
        {
            return false;
        }
        scope.auth_proven = true;
        if let Some(previous) = projection.owner.take() {
            let _ = previous.channel.send(serde_json::Value::Null);
        }
        projection.current = None;
        let _ = scope.projection_channel.send(serde_json::Value::Null);
        projection.owner = Some(ProjectionOwner {
            id,
            handle: Arc::clone(&proof.handle),
            epoch: proof.epoch.clone(),
            generation: proof.generation,
            attempt: proof.attempt,
            presentation_token: 0,
            channel: scope.projection_channel.clone(),
        });
        true
    }

    pub(super) async fn apply_projection_update(
        &self,
        id: Id,
        handle: &Arc<ConnectionHandle>,
        epoch: &ClientBindingEpoch,
        update: ProjectionUpdate,
    ) {
        if matches!(update, ProjectionUpdate::Unchanged) {
            return;
        }
        let mut projection = self.projection.lock().await;
        let exhausted_channel = projection.owner.as_ref().and_then(|owner| {
            (owner.id == id
                && Arc::ptr_eq(&owner.handle, handle)
                && owner.epoch == *epoch
                && owner.generation == projection.generation
                && owner.presentation_token == u64::MAX)
                .then(|| owner.channel.clone())
        });
        if let Some(channel) = exhausted_channel {
            projection.owner = None;
            projection.current = None;
            let _ = channel.send(serde_json::Value::Null);
            return;
        }
        let owner_state = {
            let current_generation = projection.generation;
            let Some(owner) = projection.owner.as_mut() else {
                return;
            };
            if owner.id != id
                || !Arc::ptr_eq(&owner.handle, handle)
                || owner.epoch != *epoch
                || owner.generation != current_generation
            {
                return;
            }
            // The exact owner-at-`u64::MAX` case was cleared above while the
            // same lock was held. Saturation keeps this path panic-free even
            // if that invariant changes later.
            owner.presentation_token = owner.presentation_token.saturating_add(1);
            (
                owner.id,
                Arc::clone(&owner.handle),
                owner.epoch.clone(),
                owner.generation,
                owner.attempt,
                owner.presentation_token,
                owner.channel.clone(),
            )
        };
        let expiry = match update {
            ProjectionUpdate::Current(current) if unix_now() < current.fresh_until => {
                let fresh_until = current.fresh_until;
                projection.current = Some(current);
                Some((
                    owner_state.0,
                    owner_state.1,
                    owner_state.2,
                    owner_state.3,
                    owner_state.4,
                    owner_state.5,
                    fresh_until,
                ))
            }
            ProjectionUpdate::Current(_)
            | ProjectionUpdate::Clear
            | ProjectionUpdate::Unchanged => {
                projection.current = None;
                None
            }
        };
        let value = projection
            .current
            .as_ref()
            .and_then(|current| serde_json::to_value(current).ok())
            .unwrap_or(serde_json::Value::Null);
        let _ = owner_state.6.send(value);
        drop(projection);

        if let Some((id, handle, epoch, generation, attempt, presentation_token, fresh_until)) =
            expiry
        {
            let manager = self.clone();
            std::mem::drop(tauri::async_runtime::spawn(async move {
                let mut expires_at =
                    monotonic_deadline_after(duration_until_unix_second(fresh_until));
                loop {
                    status_expiry_sleep(expires_at).await;
                    let Some(next_deadline) = manager
                        .expire_projection_if_owner(
                            id,
                            &handle,
                            &epoch,
                            (generation, attempt, presentation_token),
                            fresh_until,
                        )
                        .await
                    else {
                        break;
                    };
                    expires_at = next_deadline;
                }
            }));
        }
    }

    pub(super) async fn expire_projection_if_owner(
        &self,
        id: Id,
        handle: &Arc<ConnectionHandle>,
        epoch: &ClientBindingEpoch,
        owner_fence: (u64, u64, u64),
        fresh_until: u64,
    ) -> Option<tokio::time::Instant> {
        let (generation, attempt, presentation_token) = owner_fence;
        let mut projection = self.projection.lock().await;
        let matches_current = projection.owner.as_ref().is_some_and(|owner| {
            owner.id == id
                && Arc::ptr_eq(&owner.handle, handle)
                && owner.epoch == *epoch
                && owner.generation == generation
                && projection.generation == generation
                && owner.attempt == attempt
                && owner.presentation_token == presentation_token
        }) && projection
            .current
            .as_ref()
            .is_some_and(|current| current.fresh_until == fresh_until);
        if !matches_current {
            return None;
        }
        if unix_now() < fresh_until {
            drop(projection);
            return Some(monotonic_deadline_after(duration_until_unix_second(
                fresh_until,
            )));
        }
        projection.current = None;
        if let Some(owner) = projection.owner.as_mut() {
            owner.presentation_token = owner.presentation_token.saturating_add(1);
            let _ = owner.channel.send(serde_json::Value::Null);
        }
        None
    }

    pub(super) async fn clear_projection_if_owner(
        &self,
        id: Id,
        handle: &Arc<ConnectionHandle>,
        epoch: &ClientBindingEpoch,
    ) {
        let mut projection = self.projection.lock().await;
        if projection.owner.as_ref().is_some_and(|owner| {
            owner.id == id
                && Arc::ptr_eq(&owner.handle, handle)
                && owner.epoch == *epoch
                && owner.generation == projection.generation
        }) {
            if let Some(owner) = projection.owner.take() {
                let _ = owner.channel.send(serde_json::Value::Null);
            }
            projection.current = None;
        }
    }
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
    #[serde(default)]
    supported_extensions: Vec<String>,
    client_status: Option<Nip11ClientStatus>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Nip11ClientStatus {
    version: u8,
    status_kind: u32,
    bootstrap_kind: u32,
    max_lifetime_seconds: u64,
    delivery: String,
    authoritative: bool,
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
    parse_nip11_status_identity(&body)
}

pub(super) fn parse_nip11_status_identity(body: &[u8]) -> Result<PublicKey, String> {
    let identity: Nip11Identity =
        serde_json::from_slice(body).map_err(|_| "NIP-11 unavailable".to_string())?;
    let extension_count = identity
        .supported_extensions
        .iter()
        .filter(|extension| extension.as_str() == "buzz-client-binding-status-v1")
        .count();
    let status = identity
        .client_status
        .ok_or_else(|| "NIP-11 unavailable".to_string())?;
    if extension_count != 1
        || status.version != 1
        || status.status_kind != KIND_CLIENT_BINDING_STATUS
        || status.bootstrap_kind != KIND_CLIENT_BINDING_BOOTSTRAP
        || status.max_lifetime_seconds == 0
        || status.max_lifetime_seconds > MAX_CLIENT_BINDING_STATUS_LIFETIME_SECS
        || status.delivery != "authenticated-connection"
        || status.authoritative
    {
        return Err("NIP-11 unavailable".to_string());
    }
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
