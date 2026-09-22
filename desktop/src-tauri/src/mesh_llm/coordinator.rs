//! Runtime-owned shared-compute coordinator.
//!
//! Buzz publishes a client-signed, replaceable discovery note containing the
//! member's MeshLLM owner identity and current iroh endpoint. MeshLLM itself
//! performs transport (direct QUIC or its encrypted iroh relays) and admission.
//! The Buzz relay is only a generic Nostr store for membership and discovery;
//! it does not coordinate connections or require mesh-specific handlers.

use std::time::Duration;

use nostr::Tag;
use tauri::{AppHandle, Manager};

use crate::app_state::AppState;

/// Client-owned parameterized-replaceable discovery note. We use the standard
/// NIP-51 bookmark-set kind with a reserved d-tag so existing Buzz relays accept
/// and store it through their generic user-state path. The relay needs no mesh
/// handler or kind-registry change.
pub const KIND_BUZZ_MESH_MEMBER_STATUS: u16 = buzz_core_pkg::kind::KIND_BOOKMARK_SET as u16;
const STATUS_D_TAG_PREFIX: &str = "buzz-mesh-member-status";
const ROSTER_POLL_INTERVAL: Duration = Duration::from_secs(60);
const STATUS_PUBLISH_INTERVAL: Duration = Duration::from_secs(45);
const STATUS_PUBLISH_TIMEOUT: Duration = Duration::from_secs(10);
/// Post-launch ingress liveness / re-arm for #2062. Bounded backoff: base 15s,
/// doubles after consecutive failures up to 120s so a sticky offline peer does
/// not hammer discovery every tick, but a recovered peer is noticed quickly.
const INGRESS_WATCHDOG_BASE: Duration = Duration::from_secs(15);
const INGRESS_WATCHDOG_MAX: Duration = Duration::from_secs(120);
/// A Share Compute node may start before another member's signed status reaches
/// the relay. Recheck promptly so simultaneous starts converge into one Buzz
/// mesh instead of remaining independent islands.
const MESH_JOIN_POLL_INTERVAL: Duration = Duration::from_secs(15);
const MESH_JOIN_RETRY_MAX: Duration = Duration::from_secs(120);

pub struct MeshCoordinator {
    _status_publisher: tokio::task::JoinHandle<()>,
    _roster_watcher: tokio::task::JoinHandle<()>,
    _ingress_watchdog: tokio::task::JoinHandle<()>,
    _mesh_join_watcher: tokio::task::JoinHandle<()>,
}

/// Start the runtime-owned status publisher and admission-roster watcher.
pub async fn start_coordinator(app: AppHandle) {
    {
        let state = app.state::<AppState>();
        if state.mesh_coordinator.lock().await.is_some() {
            return;
        }
    }

    let publisher_app = app.clone();
    let status_publisher = tokio::spawn(async move {
        // Clear a stale serving status promptly after an app restart. Keep
        // publishing while stopped too: serving nodes build their admission
        // allowlist from fresh member statuses, so consumer-only identities
        // must remain fresh even though they advertise no serving targets.
        publish_current_status_once(&publisher_app, "startup").await;
        loop {
            tokio::time::sleep(STATUS_PUBLISH_INTERVAL).await;
            publish_current_status_once(&publisher_app, "heartbeat").await;
        }
    });
    let roster_app = app.clone();
    let roster_watcher = tokio::spawn(async move {
        // Carries a shrink awaiting confirmation across polls (hysteresis):
        // a reduced roster must be seen twice in a row before we tear down.
        let mut pending_shrink: Option<Vec<String>> = None;
        loop {
            tokio::time::sleep(ROSTER_POLL_INTERVAL).await;
            if let Err(error) = reconcile_roster(&roster_app, &mut pending_shrink).await {
                eprintln!("buzz-mesh: roster reconcile failed: {error}");
            }
        }
    });
    let join_app = app.clone();
    let mesh_join_watcher = tokio::spawn(async move {
        let mut sleep_for = MESH_JOIN_POLL_INTERVAL;
        loop {
            tokio::time::sleep(sleep_for).await;
            match reconcile_buzz_mesh_join(&join_app).await {
                Ok(()) => sleep_for = MESH_JOIN_POLL_INTERVAL,
                Err(error) => {
                    eprintln!("buzz-mesh: community mesh join reconcile failed: {error}");
                    sleep_for = (sleep_for * 2).min(MESH_JOIN_RETRY_MAX);
                }
            }
        }
    });

    // Brad #2304 / #2062: ensure_relay_mesh_for_record only runs on explicit
    // start + launch restore. After launch, local buzz-agent processes talk
    // directly to :9337; there is no desktop "turn dispatch" hook. This
    // watchdog is the post-launch seam: probe ingress, drop a zombie handle,
    // re-arm via ensure_relay_mesh_for_record, surface last_error on failure.
    let ingress_app = app.clone();
    let ingress_watchdog = tokio::spawn(async move {
        let mut sleep_for = INGRESS_WATCHDOG_BASE;
        loop {
            tokio::time::sleep(sleep_for).await;
            match crate::mesh_llm::rearm_relay_mesh_for_running_agents(&ingress_app).await {
                Ok(()) => {
                    sleep_for = INGRESS_WATCHDOG_BASE;
                }
                Err(error) => {
                    eprintln!("buzz-mesh: ingress re-arm watchdog: {error}");
                    sleep_for = (sleep_for * 2).min(INGRESS_WATCHDOG_MAX);
                }
            }
        }
    });

    let state = app.state::<AppState>();
    let mut guard = state.mesh_coordinator.lock().await;
    if guard.is_none() {
        *guard = Some(MeshCoordinator {
            _status_publisher: status_publisher,
            _roster_watcher: roster_watcher,
            _ingress_watchdog: ingress_watchdog,
            _mesh_join_watcher: mesh_join_watcher,
        });
    } else {
        status_publisher.abort();
        roster_watcher.abort();
        ingress_watchdog.abort();
        mesh_join_watcher.abort();
    }
}

/// Join an isolated runtime to the existing Buzz community mesh. The relay is
/// discovery only: the selected endpoint is member-signed and validated, then
/// MeshLLM establishes the encrypted peer transport itself.
async fn reconcile_buzz_mesh_join(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (peer_ids, relay_url) = {
        let runtime = state.mesh_llm_runtime.lock().await;
        let Some(runtime) = runtime.as_ref() else {
            return Ok(());
        };
        let payload = runtime
            .status_report_payload()
            .await
            .map_err(|error| error.to_string())?;
        let relay_url = runtime
            .start_request()
            .relay_url
            .clone()
            .unwrap_or_else(|| crate::relay::relay_ws_url_with_override(&state));
        (visible_peer_ids(&payload), relay_url)
    };

    let targets =
        crate::commands::mesh_llm::resolve_buzz_mesh_join_targets_at(&state, &relay_url).await?;
    let Some(target) = targets
        .into_iter()
        .find(|target| !target_is_visible(target, &peer_ids))
    else {
        return Ok(());
    };

    let runtime = state.mesh_llm_runtime.lock().await;
    let Some(runtime) = runtime.as_ref() else {
        return Ok(());
    };
    let payload = runtime
        .status_report_payload()
        .await
        .map_err(|error| error.to_string())?;
    if target_is_visible(&target, &visible_peer_ids(&payload)) {
        return Ok(());
    }
    runtime
        .dial_endpoint_addr(target.endpoint_addr)
        .await
        .map_err(|error| format!("mesh join failed: {error:#}"))
}

fn visible_peer_ids(payload: &serde_json::Value) -> Vec<String> {
    payload
        .get("peers")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|peer| peer.get("id").and_then(serde_json::Value::as_str))
        .map(|id| id.trim().to_ascii_lowercase())
        .filter(|id| !id.is_empty())
        .collect()
}

fn target_is_visible(target: &crate::mesh_llm::MeshServeTarget, peer_ids: &[String]) -> bool {
    let Some(endpoint_id) = target.endpoint_id.as_deref() else {
        return false;
    };
    let endpoint_id = endpoint_id.trim().to_ascii_lowercase();
    if endpoint_id.is_empty() {
        return false;
    }
    peer_ids.iter().any(|peer_id| {
        let peer_id = peer_id.trim().to_ascii_lowercase();
        !peer_id.is_empty()
            && (endpoint_id.starts_with(&peer_id) || peer_id.starts_with(&endpoint_id))
    })
}

/// Outcome of a roster reconcile decision.
#[derive(Debug, PartialEq, Eq)]
enum RosterReconcileAction {
    /// Keep the running allowlist untouched (no-op, or a failure we ride out).
    Keep,
    /// Restart Buzz so MeshLLM is rebuilt with a freshly resolved roster.
    ///
    /// MeshLLM's native listeners are process-owned in practice: stopping and
    /// starting the embedded runtime in one process can terminate Buzz or race
    /// ports 9337/3131. The process boundary is therefore part of the safety
    /// contract, not an implementation detail.
    RestartProcess,
    /// Observed a *shrink* (or empty) once. Hold the current allowlist and
    /// require the same reduced roster on the next poll before tearing down,
    /// so a single transient short-read never drops a member mid-inference.
    AwaitConfirm(Vec<String>),
}

/// Whether `fresh` removes any owner present in `current` (a shrink), as
/// opposed to purely adding owners or leaving the set unchanged.
fn roster_shrinks(current: &[String], fresh: &[String]) -> bool {
    current.iter().any(|owner| !fresh.contains(owner))
}

/// Pure decision for `reconcile_roster`, extracted so the transient-failure
/// and hysteresis invariants are unit-testable without a live relay.
///
/// `pending_shrink` is the reduced roster we are waiting to re-confirm (from a
/// prior poll's [`RosterReconcileAction::AwaitConfirm`]), if any.
///
/// Rules:
/// - query failed (`Err`)              → `Keep` (never de-admit on a relay blip)
/// - resolved roster == current        → `Keep` (no-op)
/// - grows (only additions)            → `RestartProcess` immediately (fast admission)
/// - shrinks/empties, first observation → `AwaitConfirm` (hold, re-check next poll)
/// - shrinks/empties, confirmed         → `RestartProcess` (same reduced roster twice)
fn roster_reconcile_action(
    current_owners: &[String],
    pending_shrink: Option<&[String]>,
    query: Result<Vec<String>, String>,
) -> RosterReconcileAction {
    let fresh = match query {
        Err(error) => {
            eprintln!(
                "buzz-mesh: roster reconcile query failed; keeping current allowlist: {error}"
            );
            return RosterReconcileAction::Keep;
        }
        Ok(fresh) => fresh,
    };

    if fresh == current_owners {
        return RosterReconcileAction::Keep;
    }

    // Growth (pure additions) is safe to apply immediately.
    if !roster_shrinks(current_owners, &fresh) {
        return RosterReconcileAction::RestartProcess;
    }

    // A shrink (including down to empty) must be confirmed across two
    // consecutive polls with the *same* reduced roster before we tear down.
    match pending_shrink {
        Some(pending) if pending == fresh => RosterReconcileAction::RestartProcess,
        _ => RosterReconcileAction::AwaitConfirm(fresh),
    }
}

async fn reconcile_roster(
    app: &AppHandle,
    pending_shrink: &mut Option<Vec<String>>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let current_request = {
        let runtime = state.mesh_llm_runtime.lock().await;
        match runtime.as_ref() {
            Some(runtime) => runtime.start_request().clone(),
            None => {
                *pending_shrink = None;
                return Ok(());
            }
        }
    };
    let Some(current_owners) = current_request.trusted_owner_ids.as_ref() else {
        *pending_shrink = None;
        return Ok(());
    };
    // A failed roster query must NOT be treated as "the roster became empty":
    // doing so would restart the node down to self-only and de-admit every
    // other member on a transient relay blip (the flapping restart loop). Keep
    // the current allowlist and try again on the next poll. A shrink is held
    // for one extra poll (hysteresis) so a single short-read never tears down.
    let relay_url = current_request
        .relay_url
        .as_deref()
        .map(str::to_owned)
        .unwrap_or_else(|| crate::relay::relay_ws_url_with_override(&state));
    let query = crate::commands::mesh_llm::resolve_trusted_owner_ids_at(&state, &relay_url).await;
    match roster_reconcile_action(current_owners, pending_shrink.as_deref(), query) {
        RosterReconcileAction::Keep => {
            *pending_shrink = None;
            return Ok(());
        }
        RosterReconcileAction::AwaitConfirm(reduced) => {
            eprintln!("buzz-mesh: roster shrink observed; awaiting confirmation before restart");
            *pending_shrink = Some(reduced);
            return Ok(());
        }
        RosterReconcileAction::RestartProcess => {
            *pending_shrink = None;
        }
    }

    let guard = state.mesh_llm_runtime.lock().await;
    let startup_pending = match guard.as_ref() {
        Some(runtime) => runtime.is_starting().await,
        None => false,
    };
    if startup_pending {
        eprintln!(
            "buzz-mesh: membership roster changed while client management startup is pending; deferring restart"
        );
        return Ok(());
    }
    if guard
        .as_ref()
        .is_some_and(|runtime| runtime.start_request() != &current_request)
    {
        // The runtime changed while relay discovery was in flight. Its own
        // request is now authoritative; the next poll will reconcile that
        // runtime instead of tearing down a fresh replacement from this stale
        // snapshot.
        return Ok(());
    }
    if guard.is_none() {
        return Ok(());
    }
    drop(guard);
    eprintln!(
        "buzz-mesh: membership roster changed; restarting Buzz to rebuild MeshLLM with the fresh community allowlist"
    );
    app.request_restart();
    Ok(())
}

pub(crate) async fn publish_current_status_once(app: &AppHandle, reason: &str) {
    let state = app.state::<AppState>();
    match tokio::time::timeout(
        STATUS_PUBLISH_TIMEOUT,
        publish_current_status_for_state(&state),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(error)) => eprintln!("buzz-mesh: status report after {reason} failed: {error}"),
        Err(_) => eprintln!("buzz-mesh: status report after {reason} timed out"),
    }
}

pub(crate) async fn publish_stopped_status_once_at(
    app: &AppHandle,
    relay_url: Option<&str>,
    reason: &str,
) {
    let state = app.state::<AppState>();
    match tokio::time::timeout(
        STATUS_PUBLISH_TIMEOUT,
        publish_stopped_status_for_state(&state, relay_url),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            eprintln!("buzz-mesh: stopped status report after {reason} failed: {error}");
        }
        Err(_) => eprintln!("buzz-mesh: stopped status report after {reason} timed out"),
    }
}

async fn publish_current_status_for_state(state: &AppState) -> Result<(), String> {
    let identity = super::ensure_owner_identity()
        .map_err(|error| format!("failed to load mesh owner identity: {error}"))?;
    let (mut payload, relay_url) = {
        let runtime = state.mesh_llm_runtime.lock().await;
        match runtime.as_ref() {
            Some(runtime) => {
                let payload = runtime
                    .status_report_payload()
                    .await
                    .map_err(|error| error.to_string())?;
                let relay_url = runtime
                    .start_request()
                    .relay_url
                    .clone()
                    .unwrap_or_else(|| crate::relay::relay_ws_url_with_override(state));
                (payload, relay_url)
            }
            None => (
                stopped_status_payload(&identity),
                crate::relay::relay_ws_url_with_override(state),
            ),
        }
    };
    bind_payload_to_member(state, &identity, &mut payload)?;
    publish_status_report_at(state, &relay_url, payload).await
}

async fn publish_stopped_status_for_state(
    state: &AppState,
    relay_url: Option<&str>,
) -> Result<(), String> {
    let identity = super::ensure_owner_identity()
        .map_err(|error| format!("failed to load mesh owner identity: {error}"))?;
    let mut payload = stopped_status_payload(&identity);
    bind_payload_to_member(state, &identity, &mut payload)?;
    let relay_url = relay_url
        .map(str::to_owned)
        .unwrap_or_else(|| crate::relay::relay_ws_url_with_override(state));
    publish_status_report_at(state, &relay_url, payload).await
}

fn stopped_status_payload(identity: &super::identity::OwnerIdentity) -> serde_json::Value {
    serde_json::json!({
        "ownerId": identity.owner_id,
        "ownerVerifyingKey": identity.verifying_key_hex,
        "serveTargets": [],
        "models": [],
    })
}

fn bind_payload_to_member(
    state: &AppState,
    identity: &super::identity::OwnerIdentity,
    payload: &mut serde_json::Value,
) -> Result<(), String> {
    let member_pubkey = state.signing_keys()?.public_key().to_hex();
    let endpoint_tokens = super::identity::advertised_endpoint_tokens(payload)
        .ok_or_else(|| "mesh discovery status has malformed serveTargets".to_string())?;
    payload["ownerId"] = serde_json::Value::String(identity.owner_id.clone());
    payload["ownerVerifyingKey"] = serde_json::Value::String(identity.verifying_key_hex.clone());
    payload["ownerBindingSig"] = serde_json::Value::String(
        identity
            .sign_member_binding(&member_pubkey)
            .map_err(|error| error.to_string())?,
    );
    payload["ownerEndpointBindingSig"] = serde_json::Value::String(
        identity
            .sign_member_endpoint_binding(&member_pubkey, &endpoint_tokens)
            .map_err(|error| error.to_string())?,
    );
    Ok(())
}

pub(crate) fn build_status_report_event(
    payload: serde_json::Value,
) -> Result<nostr::EventBuilder, String> {
    let owner_id = payload
        .get("ownerId")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "mesh discovery status is missing ownerId".to_string())?;
    let d_tag = format!("{STATUS_D_TAG_PREFIX}:{owner_id}");
    let d = Tag::parse(["d", d_tag.as_str()]).map_err(|error| error.to_string())?;
    let k = Tag::parse(["k", "buzz-mesh-status"]).map_err(|error| error.to_string())?;
    Ok(nostr::EventBuilder::new(
        nostr::Kind::Custom(KIND_BUZZ_MESH_MEMBER_STATUS),
        payload.to_string(),
    )
    .tags([d, k]))
}

async fn publish_status_report_at(
    state: &AppState,
    relay_url: &str,
    payload: serde_json::Value,
) -> Result<(), String> {
    let api_base_url = crate::relay::relay_http_base_url(relay_url);
    let keys = state.signing_keys()?;
    crate::relay::submit_event_at_with_keys(
        build_status_report_event(payload)?,
        state,
        &api_base_url,
        &keys,
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use nostr::JsonUtil;

    use super::*;

    fn join_target(endpoint_id: Option<&str>) -> crate::mesh_llm::MeshServeTarget {
        crate::mesh_llm::MeshServeTarget {
            model_id: "model".to_string(),
            model_name: None,
            endpoint_addr: "iroh://example".to_string(),
            reporter_pubkey: Some("member".to_string()),
            owner_id: Some("owner".to_string()),
            node_name: None,
            capacity: None,
            endpoint_id: endpoint_id.map(str::to_string),
            device_id: None,
            device_name: None,
        }
    }

    #[test]
    fn visible_peer_ids_reads_the_sdk_status_shape() {
        assert_eq!(
            visible_peer_ids(&serde_json::json!({
                "peers": [{"id": " ABC123 "}, {"id": "def456"}, {"name": "ignored"}]
            })),
            vec!["abc123".to_string(), "def456".to_string()]
        );
    }

    #[test]
    fn target_visibility_accepts_sdk_short_endpoint_ids() {
        let target = join_target(Some("ABC1234567890"));

        assert!(target_is_visible(&target, &["abc123".to_string()]));
        assert!(!target_is_visible(&target, &["def456".to_string()]));
    }

    #[test]
    fn target_without_an_endpoint_id_is_never_treated_as_connected() {
        assert!(!target_is_visible(
            &join_target(None),
            &["abc123".to_string()]
        ));
        assert!(!target_is_visible(
            &join_target(Some("  ")),
            &["abc123".to_string()]
        ));
    }

    // Regression: a transient roster-query failure must never restart the node
    // down to self-only. Before the fix, `resolve_trusted_owner_ids` returned
    // an empty Vec on error, which `reconcile_roster` read as "roster changed
    // to empty" and restarted — de-admitting every other member and flapping
    // the node on each relay blip. See #2000 follow-up.
    #[test]
    fn failed_roster_query_keeps_current_allowlist() {
        let current = vec!["owner-a".to_string(), "owner-b".to_string()];
        let action = roster_reconcile_action(&current, None, Err("relay returned 503".to_string()));
        assert_eq!(
            action,
            RosterReconcileAction::Keep,
            "a failed query must keep the running allowlist, never de-admit members"
        );
    }

    #[test]
    fn unchanged_roster_is_a_noop() {
        let current = vec!["owner-a".to_string()];
        let action = roster_reconcile_action(&current, None, Ok(vec!["owner-a".to_string()]));
        assert_eq!(action, RosterReconcileAction::Keep);
    }

    // Growth (pure additions) applies immediately — fast admission is fine.
    #[test]
    fn roster_growth_requests_process_restart_immediately() {
        let current = vec!["owner-a".to_string()];
        let fresh = vec!["owner-a".to_string(), "owner-c".to_string()];
        let action = roster_reconcile_action(&current, None, Ok(fresh));
        assert_eq!(action, RosterReconcileAction::RestartProcess);
    }

    // A shrink is NOT applied on first observation — it must be confirmed.
    #[test]
    fn roster_shrink_awaits_confirmation_first() {
        let current = vec!["owner-a".to_string(), "owner-b".to_string()];
        let reduced = vec!["owner-a".to_string()];
        let action = roster_reconcile_action(&current, None, Ok(reduced.clone()));
        assert_eq!(action, RosterReconcileAction::AwaitConfirm(reduced));
    }

    // The same reduced roster on two consecutive polls confirms the shrink.
    #[test]
    fn roster_shrink_requests_process_restart_once_confirmed() {
        let current = vec!["owner-a".to_string(), "owner-b".to_string()];
        let reduced = vec!["owner-a".to_string()];
        let action = roster_reconcile_action(&current, Some(&reduced), Ok(reduced.clone()));
        assert_eq!(action, RosterReconcileAction::RestartProcess);
    }

    // A shrink that changes between polls is not confirmed — it re-holds with
    // the newly observed reduced roster instead of tearing down.
    #[test]
    fn roster_shrink_reconfirms_when_it_changes() {
        let current = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let first_reduced = vec!["a".to_string(), "b".to_string()];
        let second_reduced = vec!["a".to_string()];
        let action =
            roster_reconcile_action(&current, Some(&first_reduced), Ok(second_reduced.clone()));
        assert_eq!(action, RosterReconcileAction::AwaitConfirm(second_reduced));
    }

    // A genuinely empty community (Ok(empty), distinct from a failed query)
    // still shrinks to self-only — but only after confirmation.
    #[test]
    fn genuinely_empty_roster_awaits_then_restarts_to_self_only() {
        let current = vec!["owner-a".to_string()];
        let first = roster_reconcile_action(&current, None, Ok(Vec::new()));
        assert_eq!(first, RosterReconcileAction::AwaitConfirm(Vec::new()));
        let empty: Vec<String> = Vec::new();
        let confirmed = roster_reconcile_action(&current, Some(&empty), Ok(Vec::new()));
        assert_eq!(confirmed, RosterReconcileAction::RestartProcess);
    }

    // A shrink followed by recovery to the full roster cancels the teardown.
    #[test]
    fn roster_shrink_then_recovery_keeps_allowlist() {
        let current = vec!["owner-a".to_string(), "owner-b".to_string()];
        let reduced = vec!["owner-a".to_string()];
        let held = roster_reconcile_action(&current, None, Ok(reduced.clone()));
        assert_eq!(held, RosterReconcileAction::AwaitConfirm(reduced.clone()));
        let recovered = roster_reconcile_action(&current, Some(&reduced), Ok(current.clone()));
        assert_eq!(recovered, RosterReconcileAction::Keep);
    }

    #[test]
    fn member_heartbeat_leaves_room_before_admission_status_expires() {
        assert!(
            STATUS_PUBLISH_INTERVAL.as_secs() * 2 < super::super::discovery::STATUS_FRESHNESS_SECS
        );
        assert!(STATUS_PUBLISH_TIMEOUT < STATUS_PUBLISH_INTERVAL);
    }

    #[test]
    fn stopped_status_advertises_identity_without_targets() {
        let identity = super::super::identity::OwnerIdentity {
            keystore_path: "/tmp/unused".into(),
            owner_id: "owner-test".into(),
            verifying_key_hex: "verify-test".into(),
        };
        assert_eq!(
            stopped_status_payload(&identity),
            serde_json::json!({
                "ownerId": "owner-test",
                "ownerVerifyingKey": "verify-test",
                "serveTargets": [],
                "models": [],
            })
        );
    }

    #[test]
    fn status_is_an_ordinary_client_replaceable_event() {
        let keys = nostr::Keys::generate();
        let event = build_status_report_event(serde_json::json!({"ownerId":"owner"}))
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(
            event.kind,
            nostr::Kind::Custom(KIND_BUZZ_MESH_MEMBER_STATUS)
        );
        assert_eq!(event.pubkey, keys.public_key());
        assert!(event
            .as_json()
            .contains(&format!("{STATUS_D_TAG_PREFIX}:owner")));
    }
}
