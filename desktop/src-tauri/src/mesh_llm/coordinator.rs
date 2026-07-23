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

pub struct MeshCoordinator {
    _status_publisher: tokio::task::JoinHandle<()>,
    _roster_watcher: tokio::task::JoinHandle<()>,
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
            let state = roster_app.state::<AppState>();
            if let Err(error) = reconcile_roster(&state, &mut pending_shrink).await {
                eprintln!("buzz-mesh: roster reconcile failed: {error}");
            }
        }
    });

    let state = app.state::<AppState>();
    let mut guard = state.mesh_coordinator.lock().await;
    if guard.is_none() {
        *guard = Some(MeshCoordinator {
            _status_publisher: status_publisher,
            _roster_watcher: roster_watcher,
        });
    } else {
        status_publisher.abort();
        roster_watcher.abort();
    }
}

/// Outcome of a roster reconcile decision.
#[derive(Debug, PartialEq, Eq)]
enum RosterReconcileAction {
    /// Keep the running allowlist untouched (no-op, or a failure we ride out).
    Keep,
    /// Restart the node with a freshly resolved roster.
    Restart(Vec<String>),
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
/// - grows (only additions)            → `Restart` immediately (fast admission)
/// - shrinks/empties, first observation → `AwaitConfirm` (hold, re-check next poll)
/// - shrinks/empties, confirmed         → `Restart` (same reduced roster twice)
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
        return RosterReconcileAction::Restart(fresh);
    }

    // A shrink (including down to empty) must be confirmed across two
    // consecutive polls with the *same* reduced roster before we tear down.
    match pending_shrink {
        Some(pending) if pending == fresh => RosterReconcileAction::Restart(fresh),
        _ => RosterReconcileAction::AwaitConfirm(fresh),
    }
}

async fn reconcile_roster(
    state: &AppState,
    pending_shrink: &mut Option<Vec<String>>,
) -> Result<(), String> {
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
    let query = crate::commands::mesh_llm::resolve_trusted_owner_ids(state).await;
    let fresh = match roster_reconcile_action(current_owners, pending_shrink.as_deref(), query) {
        RosterReconcileAction::Keep => {
            *pending_shrink = None;
            return Ok(());
        }
        RosterReconcileAction::AwaitConfirm(reduced) => {
            eprintln!("buzz-mesh: roster shrink observed; awaiting confirmation before restart");
            *pending_shrink = Some(reduced);
            return Ok(());
        }
        RosterReconcileAction::Restart(fresh) => {
            *pending_shrink = None;
            fresh
        }
    };

    let mut request = current_request;
    request.trusted_owner_ids = Some(fresh);
    let mut guard = state.mesh_llm_runtime.lock().await;
    let Some(running) = guard.take() else {
        return Ok(());
    };
    eprintln!("buzz-mesh: membership roster changed; restarting mesh node with fresh allowlist");
    if let Err(error) = running.stop().await {
        eprintln!("buzz-mesh: stopping mesh node for roster restart failed: {error}");
    }
    let replacement = crate::mesh_llm::DesktopMeshRuntime::start(request)
        .await
        .map_err(|error| format!("mesh node restart after roster change failed: {error}"))?;
    *guard = Some(replacement);
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

pub(crate) async fn publish_stopped_status_once(app: &AppHandle, reason: &str) {
    let state = app.state::<AppState>();
    match tokio::time::timeout(
        STATUS_PUBLISH_TIMEOUT,
        publish_stopped_status_for_state(&state),
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
    let mut payload = {
        let runtime = state.mesh_llm_runtime.lock().await;
        match runtime.as_ref() {
            Some(runtime) => runtime
                .status_report_payload()
                .await
                .map_err(|error| error.to_string())?,
            None => stopped_status_payload(&identity),
        }
    };
    bind_payload_to_member(state, &identity, &mut payload)?;
    publish_status_report(state, payload).await
}

async fn publish_stopped_status_for_state(state: &AppState) -> Result<(), String> {
    let identity = super::ensure_owner_identity()
        .map_err(|error| format!("failed to load mesh owner identity: {error}"))?;
    let mut payload = stopped_status_payload(&identity);
    bind_payload_to_member(state, &identity, &mut payload)?;
    publish_status_report(state, payload).await
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

pub(crate) async fn publish_status_report(
    state: &AppState,
    payload: serde_json::Value,
) -> Result<(), String> {
    crate::relay::submit_event(build_status_report_event(payload)?, state)
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use nostr::JsonUtil;

    use super::*;

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
    fn roster_growth_restarts_immediately() {
        let current = vec!["owner-a".to_string()];
        let fresh = vec!["owner-a".to_string(), "owner-c".to_string()];
        let action = roster_reconcile_action(&current, None, Ok(fresh.clone()));
        assert_eq!(action, RosterReconcileAction::Restart(fresh));
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
    fn roster_shrink_restarts_once_confirmed() {
        let current = vec!["owner-a".to_string(), "owner-b".to_string()];
        let reduced = vec!["owner-a".to_string()];
        let action = roster_reconcile_action(&current, Some(&reduced), Ok(reduced.clone()));
        assert_eq!(action, RosterReconcileAction::Restart(reduced));
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
        assert_eq!(confirmed, RosterReconcileAction::Restart(Vec::new()));
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
